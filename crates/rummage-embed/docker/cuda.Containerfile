# Build environment for the CUDA backend (compile check; needs no GPU).
# podman build -t rummage-build-cuda -f crates/rummage-embed/docker/cuda.Containerfile .
FROM docker.io/nvidia/cuda:12.9.1-devel-ubuntu24.04
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake ninja-build git curl ca-certificates build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.95.0
ENV PATH=/root/.cargo/bin:$PATH
