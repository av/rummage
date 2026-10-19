//! Picks the embedding engine: EmbeddingGemma 2 on llama.cpp, configured from `model.*` and
//! `engine.*`, loaded on first use.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use rummage_core::config::{Backend, Quant};
use rummage_core::{Config, ConfigFile, EmbedInput, Embedder, Modalities, Service};
use rummage_embed::{Engine, EngineConfig, Precision};

/// The embedder for this config. The model loads (and downloads, the first time) when the
/// first thing is embedded, so commands that only read the index stay quick.
pub fn embedder(cfg: &Config) -> Result<Arc<dyn Embedder>> {
    Ok(Arc::new(LazyEngine::new(engine_config(cfg)?)))
}

/// Opens the index for this config with its embedder, ready for the indexer and searches.
/// With `load_now`, the model starts loading in the background straight away (for `serve`).
pub fn open_service(cf: ConfigFile, load_now: bool) -> Result<Arc<Service>> {
    let engine = Arc::new(LazyEngine::new(engine_config(cf.config())?));
    if load_now {
        engine.load();
    }
    let svc = Arc::new(Service::open(cf, engine)?);
    svc.set_decoder(Arc::new(crate::extract::upload::Decoder));
    Ok(svc)
}

/// The engine settings for a config; fails for a backend this binary wasn't built with.
pub fn engine_config(cfg: &Config) -> Result<EngineConfig> {
    let precision = match cfg.model.quant {
        Quant::Q8_0 => Precision::Q8_0,
        Quant::Bf16 => Precision::Bf16,
        Quant::F32 => bail!("model.quant F32 has no GGUF files here; use Q8_0 or BF16"),
    };
    let dir = cfg.model_dir();
    let file = |name: &str| -> Option<PathBuf> {
        (!name.is_empty()).then(|| {
            let p = Path::new(name);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                dir.join(p)
            }
        })
    };
    let gpu_layers = match cfg.engine.backend {
        Backend::Cpu => 0,
        Backend::Auto => -1,
        b => {
            let name = serde_json::to_value(b)?
                .as_str()
                .unwrap_or_default()
                .to_string();
            if !compiled_backends().contains(&name.as_str()) {
                bail!(
                    "engine.backend is {name}, but this build has only {}; rebuild with \
                     `--features {name}` or set engine.backend to auto",
                    compiled_backends().join(", ")
                );
            }
            -1
        }
    };
    Ok(EngineConfig {
        model: file(&cfg.model.file),
        mmproj: file(&cfg.model.mmproj),
        model_dir: dir,
        precision,
        dim: cfg.model.dim as usize,
        image_tokens: cfg.model.image_tokens,
        threads: cfg.engine.threads as i32,
        gpu_layers,
        ..Default::default()
    })
}

/// Loads the engine in a background thread, on first use or when [`LazyEngine::load`] is
/// called; after a failure, tries again at most once a minute.
pub struct LazyEngine {
    cfg: EngineConfig,
    state: Arc<(Mutex<LazyState>, Condvar)>,
}

#[derive(Default)]
struct LazyState {
    engine: Option<Arc<Engine>>,
    loading: bool,
    failed: Option<(Instant, String)>,
}

const RETRY_AFTER: Duration = Duration::from_secs(60);

impl LazyEngine {
    pub fn new(cfg: EngineConfig) -> Self {
        Self {
            cfg,
            state: Arc::new((Mutex::new(LazyState::default()), Condvar::new())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LazyState> {
        self.state.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts loading (and, the first time, downloading) the model unless it is loaded,
    /// loading, or failed less than a minute ago.
    pub fn load(&self) {
        let mut s = self.lock();
        self.start(&mut s);
    }

    fn start(&self, s: &mut LazyState) {
        let recent_failure = s
            .failed
            .as_ref()
            .is_some_and(|(at, _)| at.elapsed() < RETRY_AFTER);
        if s.engine.is_some() || s.loading || recent_failure {
            return;
        }
        s.loading = true;
        let (cfg, state) = (self.cfg.clone(), self.state.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = Engine::new(cfg);
            let (lock, ready) = &*state;
            let mut s = lock.lock().unwrap_or_else(|e| e.into_inner());
            s.loading = false;
            match result {
                Ok(e) => {
                    tracing::info!(
                        "loaded {} in {:.1} s",
                        e.name(),
                        started.elapsed().as_secs_f64()
                    );
                    s.engine = Some(Arc::new(e));
                    s.failed = None;
                }
                Err(err) => {
                    let msg = format!("{err:#}");
                    tracing::error!("can't load the embedding model: {msg}");
                    s.failed = Some((Instant::now(), msg));
                }
            }
            ready.notify_all();
        });
    }

    /// The engine, waiting for it to load.
    fn get(&self) -> Result<Arc<Engine>> {
        let mut s = self.lock();
        loop {
            if let Some(e) = &s.engine {
                return Ok(e.clone());
            }
            self.start(&mut s);
            if !s.loading {
                let why = s
                    .failed
                    .as_ref()
                    .map(|(_, m)| m.as_str())
                    .unwrap_or("unknown");
                return Err(anyhow!("the embedding model is not available: {why}"));
            }
            s = self.state.1.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn loaded(&self) -> Option<Arc<Engine>> {
        self.lock().engine.clone()
    }
}

impl Embedder for LazyEngine {
    fn dim(&self) -> usize {
        self.cfg.dim
    }

    fn embed(&self, inputs: &[EmbedInput]) -> Result<Vec<Vec<f32>>> {
        self.get()?.embed(inputs)
    }

    fn modalities(&self) -> Modalities {
        match self.loaded() {
            Some(e) => e.modalities(),
            None => Modalities {
                text: true,
                image: self.cfg.media,
                audio: self.cfg.media,
                video: self.cfg.media,
            },
        }
    }

    fn name(&self) -> String {
        match self.loaded() {
            Some(e) => e.name(),
            None => format!(
                "EmbeddingGemma 2 ({:?}, {} dims; {})",
                self.cfg.precision,
                self.cfg.dim,
                if self.lock().loading {
                    "loading"
                } else {
                    "loads on first use"
                }
            ),
        }
    }

    /// Loaded, or not started yet (the first embed then loads it and waits); false while it
    /// loads in the background or after it failed to load.
    fn ready(&self) -> bool {
        let s = self.lock();
        s.engine.is_some() || (!s.loading && s.failed.is_none())
    }
}

/// Backends compiled into this binary.
pub fn compiled_backends() -> Vec<&'static str> {
    [
        (cfg!(feature = "cpu"), "cpu"),
        (cfg!(feature = "vulkan"), "vulkan"),
        (cfg!(feature = "rocm"), "rocm"),
        (cfg!(feature = "cuda"), "cuda"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_maps_to_the_engine() {
        let mut cfg = Config::default();
        cfg.model.dim = 256;
        cfg.model.dir = "/models".into();
        cfg.model.file = "custom.gguf".into();
        let e = engine_config(&cfg).unwrap();
        assert_eq!(e.dim, 256);
        assert_eq!(e.model.as_deref(), Some(Path::new("/models/custom.gguf")));
        assert_eq!(e.mmproj, None);
        assert_eq!(e.gpu_layers, -1);
        cfg.engine.backend = Backend::Cpu;
        assert_eq!(engine_config(&cfg).unwrap().gpu_layers, 0);
        cfg.model.quant = Quant::F32;
        assert!(engine_config(&cfg).is_err());
        let lazy = LazyEngine::new(e);
        assert_eq!(lazy.dim(), 256);
        assert!(lazy.name().contains("loads on first use"));
        assert!(lazy.ready(), "an engine not started yet loads on first use");
    }

    #[test]
    fn missing_backend_is_an_error() {
        let missing = ["vulkan", "rocm", "cuda"]
            .into_iter()
            .find(|b| !compiled_backends().contains(b));
        if let Some(b) = missing {
            let mut cfg = Config::default();
            cfg.engine.backend = serde_json::from_value(serde_json::json!(b)).unwrap();
            let err = engine_config(&cfg).unwrap_err().to_string();
            assert!(err.contains("rebuild"), "{err}");
        }
    }
}
