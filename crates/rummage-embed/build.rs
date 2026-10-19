//! Builds llama.cpp (libllama + libmtmd + ggml) statically with CMake and links it in.
//!
//! Backends follow the Cargo features: `cpu` (always built), `vulkan`, `rocm` (HIP) and `cuda`.
//! Environment knobs (all optional):
//! - `RUMMAGE_LLAMA_CPP_DIR`: use another llama.cpp checkout instead of the `llama.cpp` submodule.
//! - `RUMMAGE_GGML_NATIVE`: `1` (default) tunes the CPU code for this machine, `0` for portable builds.
//! - `RUMMAGE_HIP_TARGETS`: AMD GPU targets, default
//!   `gfx1151;gfx1150;gfx1100;gfx1101;gfx1102;gfx1200;gfx1201;gfx1030;gfx90a;gfx942`.
//! - `RUMMAGE_CUDA_ARCHS`: CUDA architectures, default `75;80;86;89;90;100;120` (Turing to Blackwell).
//! - `RUMMAGE_CMAKE_ARGS`: extra `-D` arguments, separated by spaces.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_HIP_TARGETS: &str =
    "gfx1151;gfx1150;gfx1100;gfx1101;gfx1102;gfx1200;gfx1201;gfx1030;gfx90a;gfx942";
const DEFAULT_CUDA_ARCHS: &str = "75;80;86;89;90;100;120";

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = env::var_os("RUMMAGE_LLAMA_CPP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("llama.cpp"));
    if !src.join("CMakeLists.txt").exists() {
        panic!(
            "llama.cpp sources not found at {}. Run `git submodule update --init --depth 1` \
             or set RUMMAGE_LLAMA_CPP_DIR.",
            src.display()
        );
    }

    for var in [
        "RUMMAGE_LLAMA_CPP_DIR",
        "RUMMAGE_GGML_NATIVE",
        "RUMMAGE_HIP_TARGETS",
        "RUMMAGE_CUDA_ARCHS",
        "RUMMAGE_CMAKE_ARGS",
        "HIPCXX",
        "HIP_PATH",
        "CUDA_PATH",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=src/hip_compat.h");
    println!("cargo:rerun-if-changed=build.rs");
    // Re-run CMake when the llama.cpp checkout changes (it builds incrementally).
    println!("cargo:rerun-if-changed={}", src.display());

    let vulkan = env::var_os("CARGO_FEATURE_VULKAN").is_some();
    let rocm = env::var_os("CARGO_FEATURE_ROCM").is_some();
    let cuda = env::var_os("CARGO_FEATURE_CUDA").is_some();
    let native = env::var("RUMMAGE_GGML_NATIVE")
        .map(|v| v != "0")
        .unwrap_or(true);
    let on = |b: bool| if b { "ON" } else { "OFF" };

    let mut cfg = cmake::Config::new(&src);
    cfg.profile("Release")
        .define("BUILD_SHARED_LIBS", "OFF")
        // Rust links a position-independent executable; ROCm's clang does not default to PIC.
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .define("LLAMA_BUILD_COMMON", "OFF")
        .define("LLAMA_BUILD_TESTS", "OFF")
        .define("LLAMA_BUILD_TOOLS", "OFF")
        .define("LLAMA_BUILD_EXAMPLES", "OFF")
        .define("LLAMA_BUILD_SERVER", "OFF")
        .define("LLAMA_BUILD_APP", "OFF")
        .define("LLAMA_BUILD_MTMD", "ON")
        .define("LLAMA_SUBPROCESS", "OFF")
        .define("MTMD_VIDEO", "OFF")
        .define("LLAMA_OPENSSL", "OFF")
        .define("LLAMA_ALL_WARNINGS", "OFF")
        .define("GGML_OPENMP", "OFF")
        .define("GGML_NATIVE", on(native))
        .define("GGML_VULKAN", on(vulkan))
        .define("GGML_HIP", on(rocm))
        .define("GGML_CUDA", on(cuda))
        .build_target("mtmd");
    if which("ninja").is_some() {
        cfg.generator("Ninja");
    }
    if rocm {
        let targets =
            env::var("RUMMAGE_HIP_TARGETS").unwrap_or_else(|_| DEFAULT_HIP_TARGETS.into());
        cfg.define("GPU_TARGETS", &targets)
            .define("AMDGPU_TARGETS", &targets);
        let compat = manifest.join("src/hip_compat.h");
        cfg.define("CMAKE_HIP_FLAGS", format!("-include {}", compat.display()));
        // Fedora/Ubuntu ROCm: let HIP's clang compile the .cu files.
        if env::var_os("HIPCXX").is_none()
            && let Some(llvm) = cmd_output("hipconfig", &["-l"])
        {
            cfg.env("HIPCXX", Path::new(llvm.trim()).join("clang"));
        }
        if env::var_os("HIP_PATH").is_none()
            && let Some(root) = cmd_output("hipconfig", &["-R"])
        {
            cfg.env("HIP_PATH", root.trim());
        }
    }
    if cuda {
        let archs = env::var("RUMMAGE_CUDA_ARCHS").unwrap_or_else(|_| DEFAULT_CUDA_ARCHS.into());
        cfg.define("CMAKE_CUDA_ARCHITECTURES", archs);
        // NCCL (multi-GPU collectives) would be one more runtime library; one GPU is enough here.
        cfg.define("GGML_CUDA_NCCL", "OFF");
        // Without VMM ggml never calls the driver API directly, so the binary does not link
        // libcuda.so.1: the static CUDA runtime loads it at run time, and a machine without the
        // NVIDIA driver gets zero CUDA devices (and the CPU) instead of a loader error.
        cfg.define("GGML_CUDA_NO_VMM", "ON");
    }
    if let Ok(extra) = env::var("RUMMAGE_CMAKE_ARGS") {
        for arg in extra.split_whitespace() {
            let arg = arg.trim_start_matches("-D");
            if let Some((k, v)) = arg.split_once('=') {
                cfg.define(k, v);
            }
        }
    }
    let dst = cfg.build();
    let build_dir = dst.join("build");

    // The shim only fills in llama.cpp's parameter structs, so Rust never mirrors their layout.
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("src/shim.cpp")
        .include(src.join("include"))
        .include(src.join("ggml/include"))
        .include(src.join("tools/mtmd"))
        .warnings(false)
        .compile("rummage_shim");

    // Link every static library the mtmd target produced, dependents before dependencies.
    let mut libs = Vec::new();
    collect_static_libs(&build_dir, &mut libs);
    libs.sort_by_key(|(_, name)| link_rank(name));
    let mut seen_dirs = std::collections::BTreeSet::new();
    for (dir, _) in &libs {
        if seen_dirs.insert(dir.clone()) {
            println!("cargo:rustc-link-search=native={}", dir.display());
        }
    }
    for (_, name) in &libs {
        println!("cargo:rustc-link-lib=static={name}");
    }

    if vulkan {
        println!("cargo:rustc-link-lib=dylib=vulkan");
    }
    if rocm {
        let root = env::var("HIP_PATH")
            .ok()
            .or_else(|| cmd_output("hipconfig", &["-R"]).map(|s| s.trim().to_string()))
            .unwrap_or_else(|| "/opt/rocm".into());
        for sub in ["lib", "lib64"] {
            let p = Path::new(&root).join(sub);
            if p.exists() {
                println!("cargo:rustc-link-search=native={}", p.display());
            }
        }
        for lib in ["amdhip64", "hipblas", "rocblas"] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }
    if cuda {
        let root = env::var("CUDA_PATH").unwrap_or_else(|_| "/usr/local/cuda".into());
        for sub in ["lib64", "lib64/stubs", "targets/x86_64-linux/lib"] {
            let p = Path::new(&root).join(sub);
            if p.exists() {
                println!("cargo:rustc-link-search=native={}", p.display());
            }
        }
        println!("cargo:rustc-link-lib=static=cudart_static");
        for lib in ["cublas", "cublasLt", "dl", "rt", "pthread"] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }
    let target = env::var("TARGET").unwrap();
    if target.contains("apple") {
        println!("cargo:rustc-link-lib=dylib=c++");
        println!("cargo:rustc-link-lib=framework=Accelerate");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=Metal");
        println!("cargo:rustc-link-lib=framework=MetalKit");
    } else if !target.contains("msvc") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }

    println!(
        "cargo:rustc-env=RUMMAGE_LLAMA_CPP_COMMIT={}",
        llama_commit(&src)
    );
    let mut backends = vec!["cpu"];
    if vulkan {
        backends.push("vulkan");
    }
    if rocm {
        backends.push("rocm");
    }
    if cuda {
        backends.push("cuda");
    }
    println!(
        "cargo:rustc-env=RUMMAGE_EMBED_BACKENDS={}",
        backends.join(",")
    );
}

fn collect_static_libs(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "CMakeFiles") {
                continue;
            }
            collect_static_libs(&path, out);
        } else if let Some(stem) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("lib"))
            .and_then(|n| n.strip_suffix(".a"))
        {
            out.push((path.parent().unwrap().to_path_buf(), stem.to_string()));
        }
    }
}

/// Order for the linker: users of a library must come before the library.
fn link_rank(name: &str) -> u32 {
    match name {
        "mtmd" => 0,
        "llama" => 1,
        "ggml" => 2,
        "ggml-base" => 9,
        "ggml-cpu" => 8,
        n if n.starts_with("ggml-") => 3,
        _ => 5,
    }
}

fn which(bin: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|p| p.join(bin))
            .find(|p| p.is_file())
    })
}

fn cmd_output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn llama_commit(src: &Path) -> String {
    cmd_output(
        "git",
        &[
            "-C",
            src.to_str().unwrap(),
            "rev-parse",
            "--short=12",
            "HEAD",
        ],
    )
    .map(|s| s.trim().to_string())
    .unwrap_or_else(|| "unknown".into())
}
