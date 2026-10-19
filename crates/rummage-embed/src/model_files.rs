//! The GGUF files from ggml-org/embeddinggemma-2-GGUF, pinned by revision and checksum.

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::Precision;

pub const REPO: &str = "ggml-org/embeddinggemma-2-GGUF";
/// Commit of the Hugging Face repo the checksums below belong to.
pub const REVISION: &str = "bfcd298762cc34d0357ece5ebdd31791a3a374d8";

pub struct ModelFile {
    pub name: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

pub fn files(precision: Precision) -> (ModelFile, ModelFile) {
    match precision {
        Precision::Q8_0 => (
            ModelFile {
                name: "embeddinggemma-2-Q8_0.gguf",
                sha256: "2188ac1deca4b77dffefd603c2776a9d76d9d74ec01841392982ebb840b09135",
                size: 309_855_456,
            },
            ModelFile {
                name: "mmproj-embeddinggemma-2-Q8_0.gguf",
                sha256: "c4a8a52691ecef40618438928bdf9e68379b854e24166f292592353db0aab64f",
                size: 554_821_024,
            },
        ),
        Precision::Bf16 => (
            ModelFile {
                name: "embeddinggemma-2-BF16.gguf",
                sha256: "68bae29d62fb8c7d23e98d21fd4662753ddd636e6b62b8a70dfe059a9844f216",
                size: 557_950_176,
            },
            ModelFile {
                name: "mmproj-embeddinggemma-2-BF16.gguf",
                sha256: "d2033b3cd0223cfb2e7a2b50dc80a664c78c816d375f5b017ec3ef80ee0bc766",
                size: 982_074_784,
            },
        ),
    }
}

/// Returns the path of `file` inside `dir`, downloading it first if it is missing.
/// A file that is already there is trusted if its size matches; downloads are checksummed.
pub fn ensure(dir: &Path, file: &ModelFile) -> Result<PathBuf> {
    let path = dir.join(file.name);
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() == file.size {
            return Ok(path);
        }
        bail!(
            "{} exists but has {} bytes, expected {}; delete it to download again",
            path.display(),
            meta.len(),
            file.size
        );
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let url = format!(
        "https://huggingface.co/{REPO}/resolve/{REVISION}/{}",
        file.name
    );
    eprintln!(
        "rummage-embed: downloading {url} ({} MB)",
        file.size / 1_000_000
    );
    let part = dir.join(format!("{}.part", file.name));
    let resp = ureq::get(&url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let mut reader = resp.into_body().into_reader();
    let mut out = BufWriter::new(File::create(&part)?);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done: u64 = 0;
    let mut next_report = 0u64;
    loop {
        let n = reader.read(&mut buf).context("reading download")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        done += n as u64;
        if done >= next_report {
            eprintln!(
                "rummage-embed:   {} / {} MB",
                done / 1_000_000,
                file.size / 1_000_000
            );
            next_report += 100_000_000;
        }
    }
    out.flush()?;
    drop(out);
    let got = format!("{:x}", hasher.finalize());
    if got != file.sha256 {
        let _ = fs::remove_file(&part);
        bail!(
            "checksum mismatch for {}: got {got}, expected {}",
            file.name,
            file.sha256
        );
    }
    fs::rename(&part, &path)?;
    Ok(path)
}
