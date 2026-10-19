//! How CLI commands reach rummage: through a running server's API, or the store directly.

use std::io::BufRead;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use rummage_core::service::Service;
use rummage_core::types::*;
use rummage_core::{Config, ConfigFile};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub enum Api {
    /// A running server.
    Remote(Client),
    /// No server: the store is opened in this process.
    Local(Arc<Service>),
}

impl Api {
    /// Uses `server` if given, else a server at the configured address if one answers,
    /// else the store directly. `local` forces the store.
    pub fn connect(cf: ConfigFile, server: Option<&str>, local: bool) -> Result<Api> {
        let cfg = cf.config();
        if !local {
            let explicit = server.is_some();
            let url = server
                .map(|s| s.trim_end_matches('/').to_string())
                .unwrap_or_else(|| local_url(cfg));
            let client = Client::new(&url, &cfg.server.auth_token);
            match client.health() {
                Ok(_) => return Ok(Api::Remote(client)),
                Err(e) if explicit => return Err(e.context(format!("no rummage server at {url}"))),
                Err(_) => {}
            }
        }
        Ok(Api::Local(crate::engine::open_service(cf, false)?))
    }

    pub fn describe(&self) -> String {
        match self {
            Api::Remote(c) => format!("server at {}", c.base),
            Api::Local(s) => format!("index at {}", s.config().db_path().display()),
        }
    }

    pub fn health(&self) -> Result<Health> {
        match self {
            Api::Remote(c) => c.health(),
            Api::Local(s) => Ok(s.health()?),
        }
    }

    pub fn folders(&self) -> Result<Vec<Folder>> {
        match self {
            Api::Remote(c) => c.get("/folders"),
            Api::Local(s) => Ok(s.folders()?),
        }
    }

    pub fn add_folder(&self, f: NewFolder) -> Result<Folder> {
        match self {
            Api::Remote(c) => c.send("POST", "/folders", &f),
            Api::Local(s) => Ok(s.add_folder(f)?),
        }
    }

    pub fn remove_folder(&self, id: &str) -> Result<()> {
        match self {
            Api::Remote(c) => c.delete(&format!("/folders/{}", url_escape(id))),
            Api::Local(s) => Ok(s.remove_folder(id)?),
        }
    }

    pub fn search(&self, req: &SearchRequest) -> Result<SearchResponse> {
        match self {
            Api::Remote(c) => c.send("POST", "/search", req),
            Api::Local(s) => Ok(s.search(req.clone(), "")?),
        }
    }

    pub fn status(&self) -> Result<IndexStatus> {
        match self {
            Api::Remote(c) => c.get("/index/status"),
            Api::Local(s) => Ok(s.index_status()?),
        }
    }

    pub fn reindex(&self, req: &ReindexRequest) -> Result<ReindexResponse> {
        match self {
            Api::Remote(c) => c.send("POST", "/reindex", req),
            Api::Local(s) => Ok(s.reindex(req)?),
        }
    }

    pub fn pause(&self) -> Result<IndexStatus> {
        match self {
            Api::Remote(c) => c.send("POST", "/index/pause", &serde_json::json!({})),
            Api::Local(s) => Ok(s.pause()?),
        }
    }

    pub fn resume(&self) -> Result<IndexStatus> {
        match self {
            Api::Remote(c) => c.send("POST", "/index/resume", &serde_json::json!({})),
            Api::Local(s) => Ok(s.resume()?),
        }
    }

    /// Asks a running indexer to rescan now.
    pub fn scan(&self, req: &ScanRequest) -> Result<()> {
        match self {
            Api::Remote(c) => c.send_empty("POST", "/index/scan", req),
            Api::Local(s) => Ok(s.scan(req)?),
        }
    }

    pub fn problems(&self, limit: u32) -> Result<ProblemList> {
        match self {
            Api::Remote(c) => c.get(&format!("/index/problems?limit={limit}")),
            Api::Local(s) => Ok(s.problems(
                &PageParams {
                    limit: Some(limit),
                    offset: 0,
                },
                "",
            )?),
        }
    }
}

/// The URL a local CLI uses to reach `server.bind`.
pub fn local_url(cfg: &Config) -> String {
    let bind = cfg.server.bind.as_str();
    let host_port = match bind.parse::<std::net::SocketAddr>() {
        Ok(a) if a.ip().is_unspecified() && a.is_ipv4() => format!("127.0.0.1:{}", a.port()),
        Ok(a) if a.ip().is_unspecified() => format!("[::1]:{}", a.port()),
        _ => bind.to_string(),
    };
    format!("http://{host_port}{}", cfg.base_path())
}

pub struct Client {
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(base: &str, token: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_millis(500)))
            .build()
            .into();
        Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            agent,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.base)
    }

    fn auth<B>(&self, req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        if self.token.is_empty() {
            req
        } else {
            req.header("Authorization", &format!("Bearer {}", self.token))
        }
    }

    pub fn health(&self) -> Result<Health> {
        let resp = self
            .agent
            .get(&self.url("/health"))
            .config()
            .timeout_global(Some(Duration::from_secs(2)))
            .build()
            .call()?;
        let health: Health = read(resp)?;
        anyhow::ensure!(health.status == "ok", "server is not healthy");
        Ok(health)
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        read(self.auth(self.agent.get(&self.url(path))).call()?)
    }

    pub fn send<B: Serialize, T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = self.url(path);
        let resp = match method {
            "POST" => self.auth(self.agent.post(&url)).send_json(body)?,
            "PATCH" => self.auth(self.agent.patch(&url)).send_json(body)?,
            "PUT" => self.auth(self.agent.put(&url)).send_json(body)?,
            m => bail!("unsupported method {m}"),
        };
        read(resp)
    }

    /// Sends a JSON body and expects no response body.
    pub fn send_empty<B: Serialize>(&self, method: &str, path: &str, body: &B) -> Result<()> {
        let url = self.url(path);
        let resp = match method {
            "POST" => self.auth(self.agent.post(&url)).send_json(body)?,
            m => bail!("unsupported method {m}"),
        };
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(api_error(resp))
        }
    }

    pub fn delete(&self, path: &str) -> Result<()> {
        let resp = self.auth(self.agent.delete(&self.url(path))).call()?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(api_error(resp))
        }
    }

    /// Calls `on_event` for each `/index/events` message until it returns false.
    pub fn follow_events(&self, mut on_event: impl FnMut(IndexEvent) -> bool) -> Result<()> {
        let resp = self
            .auth(self.agent.get(&self.url("/index/events")))
            .config()
            .timeout_global(None)
            .build()
            .call()?;
        if !resp.status().is_success() {
            return Err(api_error(resp));
        }
        let reader = std::io::BufReader::new(resp.into_body().into_reader());
        for line in reader.lines() {
            let line = line.context("reading event stream")?;
            if let Some(data) = line.strip_prefix("data:") {
                let event: IndexEvent = serde_json::from_str(data.trim())?;
                if !on_event(event) {
                    break;
                }
            }
        }
        Ok(())
    }
}

fn read<T: DeserializeOwned>(mut resp: ureq::http::Response<ureq::Body>) -> Result<T> {
    if !resp.status().is_success() {
        return Err(api_error(resp));
    }
    resp.body_mut()
        .read_json()
        .context("unexpected response from server")
}

fn api_error(mut resp: ureq::http::Response<ureq::Body>) -> anyhow::Error {
    let status = resp.status();
    match resp.body_mut().read_json::<ApiError>() {
        Ok(e) => anyhow!("{} ({status})", e.message),
        Err(_) => anyhow!("server answered {status}"),
    }
}

fn url_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
