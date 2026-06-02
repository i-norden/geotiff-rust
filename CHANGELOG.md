# Changelog

## Unreleased

## 0.5.0 - 2026-05-17

- add storage-domain single-band read APIs to `tiff-reader` and `geotiff-reader`, including full-image and windowed reads that return `[rows, cols]` arrays
- optimize separate-planar band reads so the reader only decodes the requested band plane instead of every plane
- optimize windowed strip/tile reads so small windows enumerate only intersecting storage blocks
- fix `block_cache_slots = 0` and HTTP `cache_slots = 0` so zero slots consistently disables cache storage
- fix PixelIsPoint GeoTIFF writing so transform serialization preserves normalized coordinates without a half-pixel shift
- support transform-only GeoTIFF metadata by accepting model georeferencing without a GeoKey directory and emitting a minimal GeoKey directory when writing transforms without CRS keys
- add SubIFD-backed COG overview writing alongside the existing top-level overview IFD layout, and scale overview georeferencing to each overview level
- remove duplicate GeoTIFF tags from COG overview IFDs
- make GeoKey serialization fallible instead of truncating oversized key counts and parameter offsets
- update `lerc-rust` dependencies to `0.4.2`
- add coverage for chunky and separate-planar band reads, GeoTIFF band windows, disabled zero-slot range caches, and LERC interoperability fuzz seeds

## 0.4.0 - 2026-04-19

- add JPEG-in-TIFF write support across `tiff-writer`, `geotiff-writer`, and COG output using standard compression code `7`
- add explicit decoded-pixel read APIs while preserving storage-domain sample reads, including palette expansion, sub-byte grayscale/palette decoding, YCbCr conversion, and CMYK conversion
- add structured TIFF color-model metadata for `ColorMap`, `ExtraSamples`, CMYK `InkSet`, YCbCr tags, and extended photometric interpretations
- add richer GeoTIFF CRS modeling for projected, geographic, geocentric, vertical, and compound CRS metadata
- discover overviews from both reduced-resolution top-level IFDs and recursive SubIFD overview trees
- improve COG generation with exact BigTIFF auto-selection, disk-backed `CogTileWriter` assembly, GDAL-compatible block ordering/ghost metadata, and nodata-aware average overviews
- reject streaming tile offsets, band-count mismatches, unsupported YCbCr subsampling, and JPEG layouts that are not interoperable with GDAL/libtiff
- prepare crates.io publishing metadata for the workspace crates and use the published `lerc-rust` 0.3 crates from the registry

## 0.3.1 - 2026-04-06

- move cross-crate release tests into non-publishable integration crates so publishable package tarballs stay focused
- fix release-time dev-dependency constraints for the workspace test crates

## 0.3.0 - 2026-04-06

- add pure-Rust TIFF `LERC` write support through the published `lerc-rust` 0.3 crates
- add GeoTIFF and COG `LERC`, `LERC+DEFLATE`, and `LERC+ZSTD` write support
- move `LercOptions` into `tiff-writer` and expose consistent builder configuration for TIFF and GeoTIFF writers
- add roundtrip and reference coverage for LERC writer behavior

## 0.2.5 - 2026-04-02

- add pure-Rust TIFF/GeoTIFF `LERC` read support through the published `lerc-rust` crates
- add TIFF `LercParameters` parsing and support for TIFF-side `LERC+DEFLATE` and `LERC+ZSTD`
- add real GDAL interoperability fixtures for plain `LERC`, `LERC+DEFLATE`, `LERC+ZSTD`, and tiled separate-planar RGB `LERC`
- preserve the existing write surface; TIFF `LERC` write is not part of this release
