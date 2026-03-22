#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image_name="${GEOTIFF_RUST_REFERENCE_IMAGE:-geotiff-rust-reference}"

docker build -f "$repo_root/docker/reference.Dockerfile" -t "$image_name" "$repo_root"

docker run --rm \
    -v "$repo_root:/workspace" \
    -w /workspace \
    "$image_name" \
    bash -c '
        cargo bench -p tiff-reader --bench reference_compare_bench
        cargo bench -p geotiff-reader --bench reference_compare_bench
    '
