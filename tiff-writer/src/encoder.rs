//! Low-level TIFF byte emission: header writing, IFD serialization, offset patching.

use std::io::{Seek, SeekFrom, Write};

use tiff_core::{ByteOrder, Tag, TagValue};

use crate::error::Result;

pub const CLASSIC_HEADER_LEN: u64 = 8;
pub const BIGTIFF_HEADER_LEN: u64 = 16;

fn classic_offset_u32(offset: u64) -> Result<u32> {
    u32::try_from(offset).map_err(|_| crate::error::Error::ClassicOffsetOverflow { offset })
}

fn classic_byte_count_u32(byte_count: u64) -> Result<u32> {
    u32::try_from(byte_count)
        .map_err(|_| crate::error::Error::ClassicByteCountOverflow { byte_count })
}

/// Write the TIFF header. Classic = 8 bytes, BigTIFF = 16 bytes.
/// The first-IFD offset is set to 0 and must be patched later.
pub const fn header_len(is_bigtiff: bool) -> u64 {
    if is_bigtiff {
        BIGTIFF_HEADER_LEN
    } else {
        CLASSIC_HEADER_LEN
    }
}

pub fn write_header<W: Write + Seek>(
    sink: &mut W,
    byte_order: ByteOrder,
    is_bigtiff: bool,
) -> Result<u64> {
    let pos = sink.stream_position()?;
    sink.write_all(&byte_order.magic())?;
    if is_bigtiff {
        sink.write_all(&byte_order.write_u16(43))?;
        sink.write_all(&byte_order.write_u16(8))?; // offset size
        sink.write_all(&byte_order.write_u16(0))?; // reserved
        sink.write_all(&byte_order.write_u64(0))?; // placeholder
    } else {
        sink.write_all(&byte_order.write_u16(42))?;
        sink.write_all(&byte_order.write_u32(0))?; // placeholder
    }
    Ok(pos)
}

/// Patch the first-IFD offset in the file header.
pub fn patch_first_ifd<W: Write + Seek>(
    sink: &mut W,
    header_offset: u64,
    byte_order: ByteOrder,
    is_bigtiff: bool,
    ifd_offset: u64,
) -> Result<()> {
    if is_bigtiff {
        let pointer_offset = header_offset
            .checked_add(8)
            .ok_or_else(|| crate::error::Error::Other("BigTIFF header offset overflow".into()))?;
        sink.seek(SeekFrom::Start(pointer_offset))?;
        sink.write_all(&byte_order.write_u64(ifd_offset))?;
    } else {
        let pointer_offset = header_offset
            .checked_add(4)
            .ok_or_else(|| crate::error::Error::Other("TIFF header offset overflow".into()))?;
        sink.seek(SeekFrom::Start(pointer_offset))?;
        sink.write_all(&byte_order.write_u32(classic_offset_u32(ifd_offset)?))?;
    }
    Ok(())
}

/// State returned after writing an IFD, used for patching.
#[derive(Debug)]
pub struct IfdWriteResult {
    /// File offset where this IFD starts.
    pub ifd_offset: u64,
    /// File offset of the "next IFD" pointer.
    pub next_ifd_pointer_offset: u64,
    /// File offsets where the offset-array and bytecount-array deferred data reside.
    pub offsets_tag_data_offset: Option<u64>,
    pub byte_counts_tag_data_offset: Option<u64>,
    /// Whether this IFD was written in BigTIFF format.
    pub is_bigtiff: bool,
}

/// Estimate the encoded size of an IFD, including deferred tag payloads.
pub fn estimate_ifd_size(_byte_order: ByteOrder, is_bigtiff: bool, tags: &[Tag]) -> Result<u64> {
    let entry_size: u64 = if is_bigtiff { 20 } else { 12 };
    let inline_max: usize = if is_bigtiff { 8 } else { 4 };
    let next_ptr_size: u64 = if is_bigtiff { 8 } else { 4 };
    let count_size: u64 = if is_bigtiff { 8 } else { 2 };
    let tag_count = u64::try_from(tags.len())
        .map_err(|_| crate::error::Error::Other("IFD tag count exceeds u64::MAX".into()))?;
    let entries_len = tag_count
        .checked_mul(entry_size)
        .ok_or_else(|| crate::error::Error::Other("IFD entry size overflow".into()))?;
    let deferred_len = tags.iter().try_fold(0u64, |total, tag| {
        let encoded_len = tag.value.encoded_len();
        if encoded_len <= inline_max {
            return Ok(total);
        }
        let encoded_len = u64::try_from(encoded_len)
            .map_err(|_| crate::error::Error::Other("IFD value size exceeds u64::MAX".into()))?;
        total
            .checked_add(encoded_len)
            .ok_or_else(|| crate::error::Error::Other("IFD deferred value size overflow".into()))
    })?;

    count_size
        .checked_add(entries_len)
        .and_then(|size| size.checked_add(next_ptr_size))
        .and_then(|size| size.checked_add(deferred_len))
        .ok_or_else(|| crate::error::Error::Other("IFD encoded size overflow".into()))
}

/// Write an IFD (Classic or BigTIFF). Tags must be sorted by code.
pub fn write_ifd<W: Write + Seek>(
    sink: &mut W,
    byte_order: ByteOrder,
    is_bigtiff: bool,
    tags: &[Tag],
    offsets_tag_code: u16,
    byte_counts_tag_code: u16,
    _num_blocks: usize,
) -> Result<IfdWriteResult> {
    for tags in tags.windows(2) {
        if tags[0].code >= tags[1].code {
            return Err(crate::error::Error::InvalidConfig(format!(
                "IFD tags must be strictly sorted and unique; tag {} precedes tag {}",
                tags[0].code, tags[1].code
            )));
        }
    }
    for tag in tags {
        let value_type = tag.value.tag_type();
        let value_count = tag.value.count();
        if tag.tag_type != value_type || tag.count != value_count {
            return Err(crate::error::Error::InvalidConfig(format!(
                "tag {} metadata does not match its value: type {:?}/count {} vs {:?}/{}",
                tag.code, tag.tag_type, tag.count, value_type, value_count
            )));
        }
    }
    if !is_bigtiff && tags.len() > u16::MAX as usize {
        return Err(crate::error::Error::Other(
            "classic TIFF IFD entry count exceeds u16::MAX".into(),
        ));
    }

    let ifd_offset = sink.stream_position()?;

    // Sizes depend on format
    let entry_size: u64 = if is_bigtiff { 20 } else { 12 };
    let inline_max: usize = if is_bigtiff { 8 } else { 4 };
    let next_ptr_size: u64 = if is_bigtiff { 8 } else { 4 };
    let count_size: u64 = if is_bigtiff { 8 } else { 2 };

    // Entry count
    if is_bigtiff {
        sink.write_all(&byte_order.write_u64(tags.len() as u64))?;
    } else {
        sink.write_all(&byte_order.write_u16(tags.len() as u16))?;
    }

    // Encode every tag value once; entries and the deferred data area both
    // reuse these buffers.
    let encoded_values: Vec<Vec<u8>> = tags
        .iter()
        .map(|tag| tag.value.encode(byte_order))
        .collect();

    // Calculate deferred data area start
    let tag_count = u64::try_from(tags.len())
        .map_err(|_| crate::error::Error::Other("IFD tag count exceeds u64::MAX".into()))?;
    let entries_total = tag_count
        .checked_mul(entry_size)
        .ok_or_else(|| crate::error::Error::Other("IFD entry size overflow".into()))?;
    let deferred_start = ifd_offset
        .checked_add(count_size)
        .and_then(|offset| offset.checked_add(entries_total))
        .and_then(|offset| offset.checked_add(next_ptr_size))
        .ok_or_else(|| crate::error::Error::Other("IFD layout offset overflow".into()))?;
    let mut deferred_offset = deferred_start;

    let mut deferred_offsets: Vec<Option<u64>> = Vec::with_capacity(tags.len());
    let mut offsets_data_offset = None;
    let mut byte_counts_data_offset = None;

    // First pass: determine which tags are deferred and their offsets
    for (tag, encoded) in tags.iter().zip(&encoded_values) {
        if encoded.len() > inline_max {
            if tag.code == offsets_tag_code {
                offsets_data_offset = Some(deferred_offset);
            } else if tag.code == byte_counts_tag_code {
                byte_counts_data_offset = Some(deferred_offset);
            }
            deferred_offsets.push(Some(deferred_offset));
            let encoded_len = u64::try_from(encoded.len()).map_err(|_| {
                crate::error::Error::Other("IFD value size exceeds u64::MAX".into())
            })?;
            deferred_offset = deferred_offset
                .checked_add(encoded_len)
                .ok_or_else(|| crate::error::Error::Other("IFD value offset overflow".into()))?;
        } else {
            deferred_offsets.push(None);
        }
    }

    // Second pass: write entries
    for ((tag, encoded), deferred) in tags.iter().zip(&encoded_values).zip(&deferred_offsets) {
        sink.write_all(&byte_order.write_u16(tag.code))?;
        sink.write_all(&byte_order.write_u16(tag.tag_type.to_code()))?;

        if is_bigtiff {
            sink.write_all(&byte_order.write_u64(tag.count))?;
        } else {
            let tag_count = u32::try_from(tag.count).map_err(|_| {
                crate::error::Error::Other(format!(
                    "classic TIFF tag {} count exceeds u32::MAX",
                    tag.code
                ))
            })?;
            sink.write_all(&byte_order.write_u32(tag_count))?;
        }

        match deferred {
            None => {
                let mut inline = vec![0u8; inline_max];
                inline[..encoded.len()].copy_from_slice(encoded);
                sink.write_all(&inline)?;
            }
            Some(offset) => {
                if is_bigtiff {
                    sink.write_all(&byte_order.write_u64(*offset))?;
                } else {
                    sink.write_all(&byte_order.write_u32(classic_offset_u32(*offset)?))?;
                }
            }
        }
    }

    // Next-IFD pointer
    let next_ifd_pointer_offset = sink.stream_position()?;
    if is_bigtiff {
        sink.write_all(&byte_order.write_u64(0))?;
    } else {
        sink.write_all(&byte_order.write_u32(0))?;
    }

    // Write deferred data
    for (encoded, deferred) in encoded_values.iter().zip(&deferred_offsets) {
        if let Some(offset) = deferred {
            debug_assert_eq!(sink.stream_position()?, *offset);
            sink.write_all(encoded)?;
        }
    }

    Ok(IfdWriteResult {
        ifd_offset,
        next_ifd_pointer_offset,
        offsets_tag_data_offset: offsets_data_offset,
        byte_counts_tag_data_offset: byte_counts_data_offset,
        is_bigtiff,
    })
}

/// Patch the block offsets array in a previously written IFD.
pub fn patch_block_offsets<W: Write + Seek>(
    sink: &mut W,
    byte_order: ByteOrder,
    is_bigtiff: bool,
    data_offset: u64,
    offsets: &[u64],
) -> Result<()> {
    sink.seek(SeekFrom::Start(data_offset))?;
    for &offset in offsets {
        if is_bigtiff {
            sink.write_all(&byte_order.write_u64(offset))?;
        } else {
            sink.write_all(&byte_order.write_u32(classic_offset_u32(offset)?))?;
        }
    }
    Ok(())
}

/// Patch the block byte-counts array in a previously written IFD.
pub fn patch_block_byte_counts<W: Write + Seek>(
    sink: &mut W,
    byte_order: ByteOrder,
    is_bigtiff: bool,
    data_offset: u64,
    byte_counts: &[u64],
) -> Result<()> {
    sink.seek(SeekFrom::Start(data_offset))?;
    for &count in byte_counts {
        if is_bigtiff {
            sink.write_all(&byte_order.write_u64(count))?;
        } else {
            sink.write_all(&byte_order.write_u32(classic_byte_count_u32(count)?))?;
        }
    }
    Ok(())
}

/// Patch the next-IFD pointer.
pub fn patch_next_ifd<W: Write + Seek>(
    sink: &mut W,
    byte_order: ByteOrder,
    is_bigtiff: bool,
    pointer_offset: u64,
    next_ifd: u64,
) -> Result<()> {
    sink.seek(SeekFrom::Start(pointer_offset))?;
    if is_bigtiff {
        sink.write_all(&byte_order.write_u64(next_ifd))?;
    } else {
        sink.write_all(&byte_order.write_u32(classic_offset_u32(next_ifd)?))?;
    }
    Ok(())
}

/// Parameters for building image tags.
#[derive(Debug)]
pub struct ImageTagParams<'a> {
    pub width: u32,
    pub height: u32,
    pub samples_per_pixel: u16,
    pub bits_per_sample: u16,
    pub sample_format: u16,
    pub compression: u16,
    pub photometric: u16,
    pub predictor: u16,
    pub planar_configuration: u16,
    pub subfile_type: u32,
    pub extra_tags: &'a [Tag],
    pub offsets_tag_code: u16,
    pub byte_counts_tag_code: u16,
    pub num_blocks: usize,
    pub layout_tags: &'a [Tag],
    pub is_bigtiff: bool,
}

/// Build standard TIFF tags for an image.
/// For BigTIFF, offset/bytecount arrays use Long8 instead of Long.
pub fn build_image_tags(p: &ImageTagParams<'_>) -> Vec<Tag> {
    let ImageTagParams {
        width,
        height,
        samples_per_pixel,
        bits_per_sample,
        sample_format,
        compression,
        photometric,
        predictor,
        planar_configuration,
        subfile_type,
        extra_tags,
        offsets_tag_code,
        byte_counts_tag_code,
        num_blocks,
        layout_tags,
        is_bigtiff,
    } = p;
    let mut tags = Vec::with_capacity(16 + extra_tags.len());

    if *subfile_type != 0 {
        tags.push(Tag::new(
            tiff_core::TAG_NEW_SUBFILE_TYPE,
            TagValue::Long(vec![*subfile_type]),
        ));
    }
    tags.push(Tag::new(
        tiff_core::TAG_IMAGE_WIDTH,
        TagValue::Long(vec![*width]),
    ));
    tags.push(Tag::new(
        tiff_core::TAG_IMAGE_LENGTH,
        TagValue::Long(vec![*height]),
    ));
    tags.push(Tag::new(
        tiff_core::TAG_BITS_PER_SAMPLE,
        TagValue::Short(vec![*bits_per_sample; *samples_per_pixel as usize]),
    ));
    tags.push(Tag::new(
        tiff_core::TAG_COMPRESSION,
        TagValue::Short(vec![*compression]),
    ));
    tags.push(Tag::new(
        tiff_core::TAG_PHOTOMETRIC_INTERPRETATION,
        TagValue::Short(vec![*photometric]),
    ));
    tags.push(Tag::new(
        tiff_core::TAG_SAMPLES_PER_PIXEL,
        TagValue::Short(vec![*samples_per_pixel]),
    ));
    if *planar_configuration != 1 {
        tags.push(Tag::new(
            tiff_core::TAG_PLANAR_CONFIGURATION,
            TagValue::Short(vec![*planar_configuration]),
        ));
    }
    if *predictor != 1 {
        tags.push(Tag::new(
            tiff_core::TAG_PREDICTOR,
            TagValue::Short(vec![*predictor]),
        ));
    }
    tags.push(Tag::new(
        tiff_core::TAG_SAMPLE_FORMAT,
        TagValue::Short(vec![*sample_format; *samples_per_pixel as usize]),
    ));

    for lt in *layout_tags {
        tags.push(lt.clone());
    }

    // Offset and bytecount placeholder arrays
    if *is_bigtiff {
        tags.push(Tag::new(
            *offsets_tag_code,
            TagValue::Long8(vec![0u64; *num_blocks]),
        ));
        tags.push(Tag::new(
            *byte_counts_tag_code,
            TagValue::Long8(vec![0u64; *num_blocks]),
        ));
    } else {
        tags.push(Tag::new(
            *offsets_tag_code,
            TagValue::Long(vec![0u32; *num_blocks]),
        ));
        tags.push(Tag::new(
            *byte_counts_tag_code,
            TagValue::Long(vec![0u32; *num_blocks]),
        ));
    }

    for tag in *extra_tags {
        tags.push(tag.clone());
    }

    tags.sort_by_key(|t| t.code);
    tags
}

/// Find where a tag's value is stored within a written IFD: the inline
/// value field for values that fit, or the deferred data area otherwise.
///
/// Assumes the IFD was produced by [`write_ifd`] with the same tag list.
pub fn find_tag_value_offset(
    ifd_offset: u64,
    is_bigtiff: bool,
    tags: &[Tag],
    target_code: u16,
) -> Option<u64> {
    let entry_size: u64 = if is_bigtiff { 20 } else { 12 };
    let inline_max: usize = if is_bigtiff { 8 } else { 4 };
    let next_ptr_size: u64 = if is_bigtiff { 8 } else { 4 };
    let count_size: u64 = if is_bigtiff { 8 } else { 2 };
    let value_field_offset: u64 = if is_bigtiff { 12 } else { 8 };
    let mut deferred_offset =
        ifd_offset + count_size + tags.len() as u64 * entry_size + next_ptr_size;

    for (index, tag) in tags.iter().enumerate() {
        let encoded_len = tag.value.encoded_len();
        if tag.code == target_code {
            return if encoded_len <= inline_max {
                Some(ifd_offset + count_size + index as u64 * entry_size + value_field_offset)
            } else {
                Some(deferred_offset)
            };
        }
        if encoded_len > inline_max {
            deferred_offset += encoded_len as u64;
        }
    }

    None
}
