# Build environment for the Vulkan backend (Vulkan headers + glslc), matching fedora's glibc.
# podman build -t rummage-build-vulkan -f crates/rummage-embed/docker/vulkan.Containerfile .
FROM registry.fedoraproject.org/fedora:43
RUN dnf -y install --setopt=install_weak_deps=False \
        gcc gcc-c++ cmake ninja-build git rust cargo \
        vulkan-headers vulkan-loader-devel glslc spirv-headers-devel \
    && dnf clean all
