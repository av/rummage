#!/usr/bin/env bash
# Runs a command in a toolchain image (docker/Dockerfile, --target toolchain) with the workspace
# at /src, as the calling user. C/C++/CUDA/HIP compiles go through ccache.
#   docker/in-toolchain.sh <image> <command...>
# Environment (all optional):
#   CACHE_DIR   ccache, cargo registry and bun cache, mounted at /cache (default .ci-cache)
#   TARGET_DIR  cargo target dir, mounted at /target (default: target/ in the workspace)
#   MODEL_DIR   GGUF files for the parity test, mounted at /models
#   GPU=1       pass the host's /dev/dri (and /dev/kfd) with the groups that own them
# CI and any RUMMAGE_* variable are passed through.
set -euo pipefail
image=$1
shift
root=$(cd "$(dirname "$0")/.." && pwd)
cache=$(realpath -m "${CACHE_DIR:-$root/.ci-cache}")
mkdir -p "$cache/ccache" "$cache/cargo-registry" "$cache/cargo-git" "$cache/bun"

args=(
    --rm --user "$(id -u):$(id -g)" -e HOME=/tmp
    -v "$root:/src" -w /src
    -v "$cache:/cache"
    -v "$cache/cargo-registry:/usr/local/cargo/registry"
    -v "$cache/cargo-git:/usr/local/cargo/git"
    -e CCACHE_DIR=/cache/ccache -e CCACHE_BASEDIR=/src -e CCACHE_NOHASHDIR=1
    -e CCACHE_COMPILERCHECK=content -e "CCACHE_MAXSIZE=${CCACHE_MAXSIZE:-5G}"
    -e CMAKE_C_COMPILER_LAUNCHER=ccache -e CMAKE_CXX_COMPILER_LAUNCHER=ccache
    -e CMAKE_CUDA_COMPILER_LAUNCHER=ccache -e CMAKE_HIP_COMPILER_LAUNCHER=ccache
    -e BUN_INSTALL_CACHE_DIR=/cache/bun
    -e CARGO_TERM_COLOR=always
)
if [[ -n "${TARGET_DIR:-}" ]]; then
    mkdir -p "$TARGET_DIR"
    args+=(-v "$(realpath "$TARGET_DIR"):/target" -e CARGO_TARGET_DIR=/target)
fi
if [[ -n "${MODEL_DIR:-}" ]]; then
    mkdir -p "$MODEL_DIR"
    args+=(-v "$(realpath "$MODEL_DIR"):/models" -e RUMMAGE_EMBED_MODEL_DIR=/models)
fi
if [[ "${GPU:-}" == 1 ]]; then
    for dev in /dev/dri /dev/kfd; do
        [[ -e $dev ]] && args+=(--device "$dev")
    done
    for group in video render; do
        gid=$(getent group "$group" | cut -d: -f3) && args+=(--group-add "$gid")
    done
fi
while IFS='=' read -r name _; do
    args+=(-e "$name")
done < <(env | grep -E '^(CI|RUMMAGE_[A-Z0-9_]+)=' || true)

exec docker run "${args[@]}" "$image" "$@"
