# Build environment for the ROCm (HIP) backend with Fedora's ROCm packages.
# podman build -t rummage-build-rocm -f crates/rummage-embed/docker/rocm.Containerfile .
FROM registry.fedoraproject.org/fedora:43
RUN dnf -y install --setopt=install_weak_deps=False \
        gcc gcc-c++ cmake ninja-build git rust cargo \
        rocm-hip-devel hipblas-devel rocblas-devel rocm-cmake rocm-runtime-devel \
    && dnf clean all
