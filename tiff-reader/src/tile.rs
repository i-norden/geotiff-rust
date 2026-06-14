//! Tile-based data access for TIFF images.

use std::sync::Arc;

#[cfg(feature = "rayon")]
use rayon::prelude::*;

use crate::block_decode;
use crate::cache::{BlockCache, BlockKey, BlockKind};
use crate::error::{Error, Result};
use crate::header::ByteOrder;
use crate::ifd::{Ifd, RasterLayout};
use crate::source::TiffSource;
use crate::{
    allocate_decode_output, read_block_payload, read_gdal_block_payload, DecodeReadOptions,
    GdalStructuralMetadata, Window,
};

const TAG_JPEG_TABLES: u16 = 347;

pub(crate) fn read_window(
    source: &dyn TiffSource,
    ifd: &Ifd,
    byte_order: ByteOrder,
    cache: &BlockCache,
    window: Window,
    options: DecodeReadOptions<'_>,
) -> Result<Vec<u8>> {
    let layout = ifd.raster_layout()?;
    if window.is_empty() {
        return Ok(Vec::new());
    }

    let output_len = window.output_len(&layout)?;
    let mut output = allocate_decode_output(output_len, options.decode_output_bytes)?;
    let window_row_end = window.row_end();
    let window_col_end = window.col_end();
    let output_row_bytes = window.cols * layout.pixel_stride_bytes();

    let relevant_specs = collect_tile_specs_for_window(ifd, &layout, window, None)?;

    #[cfg(feature = "rayon")]
    let decoded_blocks: Result<Vec<_>> = relevant_specs
        .par_iter()
        .map(|&spec| {
            read_tile_block(
                source,
                ifd,
                byte_order,
                cache,
                spec,
                &layout,
                options.gdal_structural_metadata,
            )
            .map(|block| (spec, block))
        })
        .collect();

    #[cfg(not(feature = "rayon"))]
    let decoded_blocks: Result<Vec<_>> = relevant_specs
        .iter()
        .map(|&spec| {
            read_tile_block(
                source,
                ifd,
                byte_order,
                cache,
                spec,
                &layout,
                options.gdal_structural_metadata,
            )
            .map(|block| (spec, block))
        })
        .collect();

    for (spec, block) in decoded_blocks? {
        let block = &*block;
        let copy_row_start = spec.y.max(window.row_off);
        let copy_row_end = (spec.y + spec.rows_in_tile).min(window_row_end);
        let copy_col_start = spec.x.max(window.col_off);
        let copy_col_end = (spec.x + spec.cols_in_tile).min(window_col_end);

        let src_row_bytes = spec.tile_width
            * if layout.planar_configuration == 1 {
                layout.pixel_stride_bytes()
            } else {
                layout.bytes_per_sample
            };

        if layout.planar_configuration == 1 {
            let copy_bytes_per_row = (copy_col_end - copy_col_start) * layout.pixel_stride_bytes();
            for row in copy_row_start..copy_row_end {
                let src_row_index = row - spec.y;
                let dest_row_index = row - window.row_off;
                let src_offset = src_row_index * src_row_bytes
                    + (copy_col_start - spec.x) * layout.pixel_stride_bytes();
                let dest_offset = dest_row_index * output_row_bytes
                    + (copy_col_start - window.col_off) * layout.pixel_stride_bytes();
                output[dest_offset..dest_offset + copy_bytes_per_row]
                    .copy_from_slice(&block[src_offset..src_offset + copy_bytes_per_row]);
            }
        } else {
            for row in copy_row_start..copy_row_end {
                let src_row_index = row - spec.y;
                let dest_row_index = row - window.row_off;
                let src_row =
                    &block[src_row_index * src_row_bytes..(src_row_index + 1) * src_row_bytes];
                let dest_row = &mut output
                    [dest_row_index * output_row_bytes..(dest_row_index + 1) * output_row_bytes];
                for col in copy_col_start..copy_col_end {
                    let src = &src_row[(col - spec.x) * layout.bytes_per_sample
                        ..(col - spec.x + 1) * layout.bytes_per_sample];
                    let pixel_base = (col - window.col_off) * layout.pixel_stride_bytes()
                        + spec.plane * layout.bytes_per_sample;
                    dest_row[pixel_base..pixel_base + layout.bytes_per_sample].copy_from_slice(src);
                }
            }
        }
    }

    Ok(output)
}

pub(crate) fn read_window_band(
    source: &dyn TiffSource,
    ifd: &Ifd,
    byte_order: ByteOrder,
    cache: &BlockCache,
    window: Window,
    band_index: usize,
    options: DecodeReadOptions<'_>,
) -> Result<Vec<u8>> {
    let layout = ifd.raster_layout()?;
    if band_index >= layout.samples_per_pixel {
        return Err(Error::BandIndexOutOfBounds {
            index: band_index,
            band_count: layout.samples_per_pixel,
        });
    }
    if window.is_empty() {
        return Ok(Vec::new());
    }

    let output_len = window.band_output_len(&layout)?;
    let mut output = allocate_decode_output(output_len, options.decode_output_bytes)?;
    let window_row_end = window.row_end();
    let window_col_end = window.col_end();
    let output_row_bytes = window.cols * layout.bytes_per_sample;

    let relevant_specs = collect_tile_specs_for_window(ifd, &layout, window, Some(band_index))?;

    #[cfg(feature = "rayon")]
    let decoded_blocks: Result<Vec<_>> = relevant_specs
        .par_iter()
        .map(|&spec| {
            read_tile_block(
                source,
                ifd,
                byte_order,
                cache,
                spec,
                &layout,
                options.gdal_structural_metadata,
            )
            .map(|block| (spec, block))
        })
        .collect();

    #[cfg(not(feature = "rayon"))]
    let decoded_blocks: Result<Vec<_>> = relevant_specs
        .iter()
        .map(|&spec| {
            read_tile_block(
                source,
                ifd,
                byte_order,
                cache,
                spec,
                &layout,
                options.gdal_structural_metadata,
            )
            .map(|block| (spec, block))
        })
        .collect();

    for (spec, block) in decoded_blocks? {
        let block = &*block;
        let copy_row_start = spec.y.max(window.row_off);
        let copy_row_end = (spec.y + spec.rows_in_tile).min(window_row_end);
        let copy_col_start = spec.x.max(window.col_off);
        let copy_col_end = (spec.x + spec.cols_in_tile).min(window_col_end);

        let src_row_bytes = spec.tile_width
            * if layout.planar_configuration == 1 {
                layout.pixel_stride_bytes()
            } else {
                layout.bytes_per_sample
            };

        if layout.planar_configuration == 1 {
            let band_offset = band_index * layout.bytes_per_sample;
            for row in copy_row_start..copy_row_end {
                let src_row_index = row - spec.y;
                let dest_row_index = row - window.row_off;
                let src_row =
                    &block[src_row_index * src_row_bytes..(src_row_index + 1) * src_row_bytes];
                let dest_row = &mut output
                    [dest_row_index * output_row_bytes..(dest_row_index + 1) * output_row_bytes];
                for col in copy_col_start..copy_col_end {
                    let src_base = (col - spec.x) * layout.pixel_stride_bytes() + band_offset;
                    let dest_col_index = col - window.col_off;
                    let dest_base = dest_col_index * layout.bytes_per_sample;
                    dest_row[dest_base..dest_base + layout.bytes_per_sample]
                        .copy_from_slice(&src_row[src_base..src_base + layout.bytes_per_sample]);
                }
            }
        } else {
            let copy_bytes_per_row = (copy_col_end - copy_col_start) * layout.bytes_per_sample;
            for row in copy_row_start..copy_row_end {
                let src_row_index = row - spec.y;
                let dest_row_index = row - window.row_off;
                let src_offset = src_row_index * src_row_bytes
                    + (copy_col_start - spec.x) * layout.bytes_per_sample;
                let dest_offset = dest_row_index * output_row_bytes
                    + (copy_col_start - window.col_off) * layout.bytes_per_sample;
                output[dest_offset..dest_offset + copy_bytes_per_row]
                    .copy_from_slice(&block[src_offset..src_offset + copy_bytes_per_row]);
            }
        }
    }

    Ok(output)
}

fn collect_tile_specs_for_window(
    ifd: &Ifd,
    layout: &RasterLayout,
    window: Window,
    band_index: Option<usize>,
) -> Result<Vec<TileBlockSpec>> {
    let tile_width = ifd
        .tile_width()
        .ok_or(Error::TagNotFound(crate::ifd::TAG_TILE_WIDTH))? as usize;
    let tile_height = ifd
        .tile_height()
        .ok_or(Error::TagNotFound(crate::ifd::TAG_TILE_LENGTH))? as usize;
    if tile_width == 0 || tile_height == 0 {
        return Err(Error::InvalidImageLayout(
            "tile width and height must be greater than zero".into(),
        ));
    }

    let offsets = ifd
        .tile_offsets()
        .ok_or(Error::TagNotFound(crate::ifd::TAG_TILE_OFFSETS))?;
    let counts = ifd
        .tile_byte_counts()
        .ok_or(Error::TagNotFound(crate::ifd::TAG_TILE_BYTE_COUNTS))?;
    if offsets.len() != counts.len() {
        return Err(Error::InvalidImageLayout(format!(
            "TileOffsets has {} entries but TileByteCounts has {}",
            offsets.len(),
            counts.len()
        )));
    }

    let tiles_across = layout.width.div_ceil(tile_width);
    let tiles_down = layout.height.div_ceil(tile_height);
    let tiles_per_plane = tiles_across
        .checked_mul(tiles_down)
        .ok_or_else(tile_count_overflow)?;
    let expected = match layout.planar_configuration {
        1 => tiles_per_plane,
        2 => tiles_per_plane
            .checked_mul(layout.samples_per_pixel)
            .ok_or_else(tile_count_overflow)?,
        planar => return Err(Error::UnsupportedPlanarConfiguration(planar)),
    };
    if offsets.len() != expected {
        return Err(Error::InvalidImageLayout(format!(
            "expected {expected} tiles, found {}",
            offsets.len()
        )));
    }

    let first_tile_row = window.row_off / tile_height;
    let last_tile_row = window.row_end().div_ceil(tile_height).min(tiles_down);
    let first_tile_col = window.col_off / tile_width;
    let last_tile_col = window.col_end().div_ceil(tile_width).min(tiles_across);
    let plane_range = if layout.planar_configuration == 1 {
        0..1
    } else if let Some(band_index) = band_index {
        band_index..band_index + 1
    } else {
        0..layout.samples_per_pixel
    };
    let spec_count = (last_tile_row - first_tile_row)
        .saturating_mul(last_tile_col - first_tile_col)
        .saturating_mul(plane_range.end - plane_range.start);
    let mut specs = Vec::with_capacity(spec_count);

    for plane in plane_range {
        for tile_row in first_tile_row..last_tile_row {
            for tile_col in first_tile_col..last_tile_col {
                let plane_tile_index = tile_row
                    .checked_mul(tiles_across)
                    .and_then(|base| base.checked_add(tile_col))
                    .ok_or_else(tile_count_overflow)?;
                let tile_index = if layout.planar_configuration == 1 {
                    plane_tile_index
                } else {
                    plane
                        .checked_mul(tiles_per_plane)
                        .and_then(|base| base.checked_add(plane_tile_index))
                        .ok_or_else(tile_count_overflow)?
                };
                let x = tile_col * tile_width;
                let y = tile_row * tile_height;
                let cols_in_tile = tile_width.min(layout.width.saturating_sub(x));
                let rows_in_tile = tile_height.min(layout.height.saturating_sub(y));
                specs.push(TileBlockSpec {
                    index: tile_index,
                    plane,
                    x,
                    y,
                    cols_in_tile,
                    rows_in_tile,
                    offset: offsets[tile_index],
                    byte_count: counts[tile_index],
                    tile_width,
                    tile_height,
                });
            }
        }
    }

    Ok(specs)
}

fn tile_count_overflow() -> Error {
    Error::InvalidImageLayout("tile count overflows usize".into())
}

#[derive(Clone, Copy)]
struct TileBlockSpec {
    index: usize,
    plane: usize,
    x: usize,
    y: usize,
    cols_in_tile: usize,
    rows_in_tile: usize,
    offset: u64,
    byte_count: u64,
    tile_width: usize,
    tile_height: usize,
}

fn read_tile_block(
    source: &dyn TiffSource,
    ifd: &Ifd,
    byte_order: ByteOrder,
    cache: &BlockCache,
    spec: TileBlockSpec,
    layout: &RasterLayout,
    gdal_structural_metadata: Option<&GdalStructuralMetadata>,
) -> Result<Arc<Vec<u8>>> {
    let cache_key = BlockKey {
        ifd_index: ifd.index,
        kind: BlockKind::Tile,
        block_index: spec.index,
    };
    if let Some(cached) = cache.get(&cache_key) {
        return Ok(cached);
    }

    let jpeg_tables = ifd
        .tag(TAG_JPEG_TABLES)
        .and_then(|tag| tag.value.as_bytes());
    let byte_count_limit =
        block_decode::compressed_block_byte_count_limit(&block_decode::BlockDecodeRequest {
            ifd,
            layout: *layout,
            byte_order,
            compressed: &[],
            index: spec.index,
            jpeg_tables,
            block_width: spec.tile_width,
            block_height: spec.tile_height,
        })?;
    let compressed = match gdal_structural_metadata {
        Some(metadata) => read_gdal_block_payload(
            source,
            metadata,
            byte_order,
            spec.offset,
            spec.byte_count,
            byte_count_limit,
            spec.index,
        )?,
        None => read_block_payload(
            source,
            spec.offset,
            spec.byte_count,
            byte_count_limit,
            spec.index,
        )?,
    };

    let decoded = block_decode::decode_compressed_block(block_decode::BlockDecodeRequest {
        ifd,
        layout: *layout,
        byte_order,
        compressed: &compressed,
        index: spec.index,
        jpeg_tables,
        block_width: spec.tile_width,
        block_height: spec.tile_height,
    })?;
    Ok(cache.insert(cache_key, decoded))
}
