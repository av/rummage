# rummage images, one per backend: cpu, vulkan, rocm, cuda.
#
#   docker build -f docker/Dockerfile --build-arg BACKEND=vulkan -t rummage:vulkan .
#
# Builds from source by default. CI builds the binary first (same toolchain stage) and passes
# it in with `--build-context dist=<dir with rummage>`, which replaces the `dist` stage below.
# Everything is Ubuntu 24.04 based (glibc 2.39), so the binaries also run on newer distros.

ARG BACKEND=cpu
ARG UBUNTU=24.04
ARG CUDA_VERSION=12.9.2
ARG ROCM_VERSION=7.2.4
ARG RUST_VERSION=1.99.1

# ---------------------------------------------------------------- toolchains

FROM docker.io/library/ubuntu:${UBUNTU} AS toolchain-cpu
ARG RUST_VERSION
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake ninja-build git curl ca-certificates pkg-config ccache ffmpeg \
        poppler-utils \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
# rustfmt and clippy for CI's lint job, which runs in this image.
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain ${RUST_VERSION} --component rustfmt,clippy \
    && chmod -R a+rX /usr/local/rustup /usr/local/cargo

FROM toolchain-cpu AS toolchain-vulkan
# Mesa's drivers too, so the tests can run on the GPU when /dev/dri is passed in.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libvulkan-dev glslc spirv-headers mesa-vulkan-drivers \
    && rm -rf /var/lib/apt/lists/*

FROM docker.io/nvidia/cuda:${CUDA_VERSION}-devel-ubuntu${UBUNTU} AS toolchain-cuda
ARG RUST_VERSION
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake ninja-build git curl ca-certificates pkg-config ccache ffmpeg \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain ${RUST_VERSION} \
    && chmod -R a+rX /usr/local/rustup /usr/local/cargo

FROM docker.io/rocm/dev-ubuntu-${UBUNTU}:${ROCM_VERSION} AS toolchain-rocm
ARG RUST_VERSION
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake ninja-build git curl ca-certificates pkg-config ccache ffmpeg \
        hipblas-dev rocblas-dev \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:/opt/rocm/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain ${RUST_VERSION} \
    && chmod -R a+rX /usr/local/rustup /usr/local/cargo

FROM toolchain-${BACKEND} AS toolchain
# bun builds the web UI (crates/rummage/build.rs).
COPY --from=docker.io/oven/bun:1 /usr/local/bin/bun /usr/local/bin/bun
# Portable CPU code (x86-64-v3: AVX2, FMA, F16C) instead of tuning for the build machine.
ENV RUMMAGE_GGML_NATIVE=0

# ---------------------------------------------------------------- build

FROM toolchain AS build
ARG BACKEND
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p rummage --features ${BACKEND} \
    && mkdir /dist && cp target/release/rummage /dist/

# Replaced by `--build-context dist=...` when the binary is already built.
FROM scratch AS dist
COPY --from=build /dist/rummage /rummage

# ---------------------------------------------------------------- runtimes

FROM docker.io/library/ubuntu:${UBUNTU} AS runtime-cpu
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl ffmpeg poppler-utils \
    && rm -rf /var/lib/apt/lists/*

FROM runtime-cpu AS runtime-vulkan
# The Vulkan loader plus Mesa's drivers (RADV for AMD, ANV for Intel). NVIDIA's Vulkan driver
# comes from the host through the NVIDIA container toolkit.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libvulkan1 mesa-vulkan-drivers \
    && rm -rf /var/lib/apt/lists/*

# NVIDIA's base image has the driver hooks but no CUDA libraries; the binary needs only cuBLAS
# (the CUDA runtime is linked in).
FROM docker.io/nvidia/cuda:${CUDA_VERSION}-base-ubuntu${UBUNTU} AS runtime-cuda
ARG CUDA_VERSION
ENV DEBIAN_FRONTEND=noninteractive
RUN v=$(echo ${CUDA_VERSION} | cut -d. -f1,2 | tr . -) \
    && apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl ffmpeg poppler-utils libcublas-$v \
    && rm -rf /var/lib/apt/lists/*
ENV NVIDIA_VISIBLE_DEVICES=all NVIDIA_DRIVER_CAPABILITIES=compute,utility

FROM docker.io/library/ubuntu:${UBUNTU} AS runtime-rocm
ARG ROCM_VERSION
ENV DEBIAN_FRONTEND=noninteractive
# HIP runtime, hipBLAS and rocBLAS from AMD's repository. rocBLAS and hipBLASLt ship GPU kernels
# for every AMD GPU (over 5 GB); keep only the targets the binary is built for (the default
# RUMMAGE_HIP_TARGETS in crates/rummage-embed/build.rs).
ARG KEEP_GPU_TARGETS="gfx1151 gfx1150 gfx1100 gfx1101 gfx1102 gfx1200 gfx1201 gfx1030 gfx90a gfx942"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl gnupg \
    && curl -fsSL https://repo.radeon.com/rocm/rocm.gpg.key | gpg --dearmor -o /etc/apt/keyrings/rocm.gpg \
    && echo "deb [arch=amd64 signed-by=/etc/apt/keyrings/rocm.gpg] https://repo.radeon.com/rocm/apt/${ROCM_VERSION} noble main" \
        > /etc/apt/sources.list.d/rocm.list \
    && printf 'Package: *\nPin: release o=repo.radeon.com\nPin-Priority: 600\n' > /etc/apt/preferences.d/rocm-pin-600 \
    && apt-get update && apt-get install -y --no-install-recommends \
        ffmpeg poppler-utils hip-runtime-amd hipblas rocblas \
    && apt-get purge -y gnupg && apt-get autoremove -y && rm -rf /var/lib/apt/lists/* \
    && set -f && keep=$(for t in $KEEP_GPU_TARGETS; do printf "! -name *%s* " "$t"; done) \
    && find /opt/rocm/lib/rocblas/library /opt/rocm/lib/hipblaslt/library -type f -name '*gfx*' $keep -delete
ENV PATH=/opt/rocm/bin:$PATH LD_LIBRARY_PATH=/opt/rocm/lib

# ---------------------------------------------------------------- image

FROM runtime-${BACKEND}
ARG BACKEND
LABEL org.opencontainers.image.source="https://github.com/av/rummage" \
      org.opencontainers.image.description="rummage: local multimodal semantic search (${BACKEND} build)" \
      org.opencontainers.image.licenses="Apache-2.0"
# Ubuntu images ship a user 1000 named ubuntu; rummage takes its place. GPU device nodes belong
# to the host's video/render groups: pass them with --group-add (see README).
RUN (userdel -r ubuntu 2>/dev/null || true) \
    && groupadd -g 1000 rummage && useradd -u 1000 -g 1000 -d /data -M -s /usr/sbin/nologin rummage \
    && mkdir -p /data && chown rummage:rummage /data
COPY --from=dist /rummage /usr/local/bin/rummage
COPY LICENSE NOTICE docker/THIRD-PARTY-NOTICES /usr/share/doc/rummage/
ENV RUMMAGE_DATA_DIR=/data \
    XDG_CONFIG_HOME=/data/config \
    RUMMAGE_BIND=0.0.0.0:8099 \
    HOME=/data
USER rummage
WORKDIR /data
VOLUME /data
EXPOSE 8099
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8099/api/v1/health >/dev/null || exit 1
ENTRYPOINT ["rummage"]
CMD ["serve"]
