#!/usr/bin/env bash
# Packs a built binary as rummage-<version>-linux-x86_64-<backend>.tar.gz plus a .sha256 file.
#   docker/package.sh <version> <backend> <path to rummage> <output dir>
set -euo pipefail
version=$1 backend=$2 bin=$3 out=$4
root=$(cd "$(dirname "$0")/.." && pwd)
name=rummage-$version-linux-x86_64-$backend
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

mkdir "$stage/$name"
install -m 755 "$bin" "$stage/$name/rummage"
cp "$root/README.md" "$stage/$name/README.md"
{
    cat "$root/docker/NOTICE"
    printf '\n---------------------------------------- llama.cpp and ggml\n\n'
    cat "$root/crates/rummage-embed/llama.cpp/LICENSE"
} >"$stage/$name/LICENSE"

mkdir -p "$out"
tar -C "$stage" --owner=0 --group=0 --sort=name -czf "$out/$name.tar.gz" "$name"
(cd "$out" && sha256sum "$name.tar.gz" >"$name.tar.gz.sha256")
echo "$out/$name.tar.gz"
