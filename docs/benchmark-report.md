# Benchmark Report

Date: 2026-06-13

This report summarizes the current Dockerized comparison benchmark suite for
`geotiff-rust` against GDAL and a same-environment `v0.6.1` baseline. It
captures the performance shape of the reader comparison benches after making
mmap an explicit opt-in mode and adding a safe file-backed default.

## System Under Test

- Machine: Apple M1
- OS: macOS 13.0
- Architecture: `arm64`
- Benchmark toolchain: Dockerized `rustc 1.91.0`
- Reference environment: Docker image with `gdal-bin`, `python3-gdal`, and
  `libtiff-tools`

These measurements reflect this machine. GDAL and libtiff ran in Docker, but
the timings still reflect the same host CPU and storage stack.

## Scope

- `tiff-reader` full-decode comparison against the repo's GDAL helper
- `geotiff-reader` open-plus-full-decode comparison against the repo's GDAL helper
- `geotiff-writer` multiband planar COG decode comparison against the repo's
  GDAL helper
- mmap (`geotiff-rust`) versus safe file-backed (`geotiff-rust-file`) local
  open modes
- same-environment mmap regression check against `v0.6.1`

## Methodology

Commands used for this report:

```sh
./scripts/run-reference-benchmarks.sh
```

The `v0.6.1` baseline was run from an exported copy of the `v0.6.1` tag with
the same Docker image and the same three integration benchmark targets.

Notes:

- The `tiff-reader` benchmark uses a synthetic 2048x2048 tiled,
  Deflate-compressed `u16` TIFF fixture generated at benchmark time.
- The `geotiff-reader` benchmark uses a matching synthetic GeoTIFF fixture with
  `EPSG:32615` metadata.
- The `geotiff-writer` decode benchmark uses a synthetic multiband planar COG
  fixture with internal overviews.
- The benchmarks validate byte length and raster hash equality against the GDAL
  helper before timing.
- The comparison target is the repo's Python GDAL helper, not a direct GDAL C API benchmark.

## Current Results

### Current Timings

Mean Criterion point estimates from the current branch:

| workload | mmap | safe file | file vs mmap | GDAL helper |
| --- | ---: | ---: | ---: | ---: |
| `tiff-reader` full decode | 14.34 ms | 15.44 ms | +7.7% | 19.78 ms |
| `tiff-reader` planar full decode | 11.67 ms | 14.68 ms | +25.8% | 391.95 ms |
| `geotiff-writer` multiband planar COG decode | 11.28 ms | 18.92 ms | +67.7% | 428.52 ms |
| `geotiff-reader` open + full decode | 15.19 ms | 16.36 ms | +7.7% | 18.28 ms |
| `geotiff-reader` planar open + full decode | 11.40 ms | 12.50 ms | +9.6% | 402.99 ms |

### Regression Check

Mean Criterion point estimates for current mmap mode versus the same benchmarks
on `v0.6.1`, where `open` was mmap-backed. Negative change is faster:

| workload | v0.6.1 mmap | current mmap | change |
| --- | ---: | ---: | ---: |
| `tiff-reader` full decode | 14.83 ms | 14.34 ms | -3.3% |
| `tiff-reader` planar full decode | 11.79 ms | 11.67 ms | -1.0% |
| `geotiff-writer` multiband planar COG decode | 12.19 ms | 11.28 ms | -7.4% |
| `geotiff-reader` open + full decode | 20.37 ms | 15.19 ms | -25.4% |
| `geotiff-reader` planar open + full decode | 12.35 ms | 11.40 ms | -7.7% |

## Interpretation

- The mmap-backed benchmark IDs show no regression against the `v0.6.1` mmap
  baseline in this run.
- The safe file-backed default is slower than mmap on every measured workload,
  from about 8% on full-decode workloads to about 68% on the synthetic
  multiband planar COG decode workload.
- Current safe file-backed timings remain faster than the GDAL helper in this
  run, but the GDAL helper is a Python subprocess benchmark target and should
  not be read as a direct GDAL C API ceiling.
- The benchmark IDs named `geotiff-rust` remain mmap-backed for historical
  comparison; the `geotiff-rust-file` IDs are the safe file-backed default.

## Limits

- This report reflects one machine.
- The benchmark fixtures are synthetic and intentionally narrow; they do not
  cover every real-world TIFF or GeoTIFF workload shape.
- The parity suite uses real interoperability fixtures, but the benchmark suite
  does not currently time that broader corpus.
- Docker improves reproducibility here, but containerized results remain host-specific.
