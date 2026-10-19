//! Builds the web UI (`ui/`, Svelte + Vite) so `cargo build` embeds it in the binary.
//!
//! - `RUMMAGE_UI_DIST=<dir>` embeds an already built UI instead (e.g. built in another step).
//! - Without bun on `PATH`, the binary gets a placeholder page and the build prints a warning.
//! - `RUMMAGE_SKIP_UI=1` forces the placeholder.

use std::path::{Path, PathBuf};
use std::process::Command;

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>rummage</title>
<style>
  :root { color-scheme: light dark; }
  body { font: 16px/1.5 system-ui, sans-serif; max-width: 40rem; margin: 15vh auto; padding: 0 1rem; }
</style>
</head>
<body>
<h1>rummage</h1>
<p>This binary was built without the web UI (bun was not found). Install bun and rebuild.</p>
<p>The API is at <a href="api/v1/health">api/v1</a>, described in <a href="api/docs">api/docs</a>.</p>
</body>
</html>
"#;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let ui = manifest.join("ui");
    for f in [
        "src",
        "public",
        "index.html",
        "package.json",
        "bun.lock",
        "vite.config.ts",
        "svelte.config.js",
        "tsconfig.json",
    ] {
        println!("cargo:rerun-if-changed=ui/{f}");
    }
    println!("cargo:rerun-if-env-changed=RUMMAGE_UI_DIST");
    println!("cargo:rerun-if-env-changed=RUMMAGE_SKIP_UI");

    if let Some(dist) = std::env::var_os("RUMMAGE_UI_DIST") {
        let dist = PathBuf::from(dist);
        assert!(
            dist.join("index.html").is_file(),
            "RUMMAGE_UI_DIST={} has no index.html",
            dist.display()
        );
        embed(&dist);
        return;
    }

    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("ui");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let skip = std::env::var_os("RUMMAGE_SKIP_UI").is_some_and(|v| !v.is_empty() && v != "0");
    let bun = which("bun");
    match bun.filter(|_| !skip) {
        Some(bun) => build(&bun, &ui, &out),
        None => {
            if !skip {
                println!(
                    "cargo:warning=bun not found: building rummage with a placeholder page instead of the web UI"
                );
            }
            std::fs::write(out.join("index.html"), PLACEHOLDER).unwrap();
        }
    }
    embed(&out);
}

/// Points the `Ui` rust-embed folder at `dir`.
fn embed(dir: &Path) {
    println!("cargo:rustc-env=RUMMAGE_UI_DIR={}", dir.display());
}

fn build(bun: &Path, ui: &Path, out: &Path) {
    let stamp = ui.join("node_modules/.rummage-install");
    let newest_manifest = ["package.json", "bun.lock"]
        .iter()
        .filter_map(|f| {
            std::fs::metadata(ui.join(f))
                .and_then(|m| m.modified())
                .ok()
        })
        .max();
    let installed = std::fs::metadata(&stamp).and_then(|m| m.modified()).ok();
    if installed.is_none() || newest_manifest > installed {
        run(
            Command::new(bun)
                .args(["install", "--frozen-lockfile"])
                .current_dir(ui),
            "bun install",
        );
        std::fs::write(&stamp, "").unwrap();
    }
    run(
        Command::new(bun)
            .args(["x", "vite", "build", "--logLevel", "warn", "--emptyOutDir"])
            .arg("--outDir")
            .arg(out)
            .current_dir(ui),
        "vite build",
    );
}

fn run(cmd: &mut Command, what: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("{what} failed to start: {e}"));
    assert!(
        status.success(),
        "{what} failed ({status}); set RUMMAGE_SKIP_UI=1 to build without the web UI"
    );
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| p.is_file())
    })
}
