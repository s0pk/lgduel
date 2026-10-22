# Linux environment for scripts/check.sh (same packages as the GitHub Actions
# Linux job): ALSA headers for cpal, Mesa lavapipe as a software Vulkan GPU
# for the offscreen render checks.
FROM rust:1.99.1-bookworm
RUN apt-get update \
 && apt-get install -y --no-install-recommends libasound2-dev pkg-config mesa-vulkan-drivers libvulkan1 python3 \
 && rm -rf /var/lib/apt/lists/*
RUN rustup component add rustfmt clippy
WORKDIR /src
