use crate::error::{Error, Result};
use crate::header::{ByteOrder, TiffHeader};
use crate::io::Cursor;
use crate::tag::{Tag, TagValue};

/// A parsed Image File Directory (IFD).
#[derive(Debug, Clone)]
pub struct Ifd {
    /// Tags in this IFD, sorted by tag code.
    tags: Vec<Tag>,
    /// Index of this IFD in the chain (0-based).
    pub index: usize,
}

// Well-known TIFF tag codes.
const TAG_IMAGE_WIDTH: u16 = 256;
const TAG_IMAGE_LENGTH: u16 = 257;
const TAG_BITS_PER_SAMPLE: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_SAMPLES_PER_PIXEL: u16 = 277;
const TAG_ROWS_PER_STRIP: u16 = 278;
const TAG_TILE_WIDTH: u16 = 322;
const TAG_TILE_LENGTH: u16 = 323;
const TAG_SAMPLE_FORMAT: u16 = 339;

impl Ifd {
    /// Look up a tag by its code.
    pub fn tag(&self, code: u16) -> Option<&Tag> {
        self.tags.iter().find(|t| t.code == code)
    }

    /// Returns all tags in this IFD.
    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }

    /// Image width in pixels.
    pub fn width(&self) -> u32 {
        self.tag_u32(TAG_IMAGE_WIDTH).unwrap_or(0)
    }

    /// Image height in pixels.
    pub fn height(&self) -> u32 {
        self.tag_u32(TAG_IMAGE_LENGTH).unwrap_or(0)
    }

    /// Bits per sample for each channel.
    pub fn bits_per_sample(&self) -> Vec<u16> {
        self.tag(TAG_BITS_PER_SAMPLE)
            .and_then(|t| match &t.value {
                TagValue::Short(v) => Some(v.clone()),
                _ => None,
            })
            .unwrap_or_else(|| vec![1])
    }

    /// Compression scheme (1 = no compression, 8 = deflate, etc.).
    pub fn compression(&self) -> u16 {
        self.tag_u16(TAG_COMPRESSION).unwrap_or(1)
    }

    /// Number of samples (bands) per pixel.
    pub fn samples_per_pixel(&self) -> u16 {
        self.tag_u16(TAG_SAMPLES_PER_PIXEL).unwrap_or(1)
    }

    /// Returns `true` if this IFD uses tiled layout.
    pub fn is_tiled(&self) -> bool {
        self.tag(TAG_TILE_WIDTH).is_some()
    }

    /// Tile width (only for tiled IFDs).
    pub fn tile_width(&self) -> Option<u32> {
        self.tag_u32(TAG_TILE_WIDTH)
    }

    /// Tile height (only for tiled IFDs).
    pub fn tile_height(&self) -> Option<u32> {
        self.tag_u32(TAG_TILE_LENGTH)
    }

    /// Rows per strip (only for stripped IFDs).
    pub fn rows_per_strip(&self) -> Option<u32> {
        self.tag_u32(TAG_ROWS_PER_STRIP)
    }

    /// Sample format for each channel.
    pub fn sample_format(&self) -> Vec<u16> {
        self.tag(TAG_SAMPLE_FORMAT)
            .and_then(|t| match &t.value {
                TagValue::Short(v) => Some(v.clone()),
                _ => None,
            })
            .unwrap_or_else(|| vec![1]) // 1 = unsigned integer
    }

    fn tag_u16(&self, code: u16) -> Option<u16> {
        self.tag(code).and_then(|t| t.value.as_u16())
    }

    fn tag_u32(&self, code: u16) -> Option<u32> {
        self.tag(code).and_then(|t| t.value.as_u32())
    }
}

/// Parse the chain of IFDs starting from the header's first IFD offset.
pub fn parse_ifd_chain(data: &[u8], header: &TiffHeader) -> Result<Vec<Ifd>> {
    let mut ifds = Vec::new();
    let mut offset = header.first_ifd_offset;
    let mut index = 0;

    while offset != 0 {
        if offset as usize >= data.len() {
            return Err(Error::Truncated {
                offset,
                needed: 2,
                available: data.len() as u64,
            });
        }

        let mut cursor = Cursor::with_offset(data, offset as usize, header.byte_order)?;

        let (entry_count, next_offset) = if header.is_bigtiff() {
            let count = cursor.read_u64()? as usize;
            let tags = parse_tags_bigtiff(&mut cursor, count, data, header.byte_order)?;
            let next = cursor.read_u64()?;
            (tags, next)
        } else {
            let count = cursor.read_u16()? as usize;
            let tags = parse_tags_classic(&mut cursor, count, data, header.byte_order)?;
            let next = cursor.read_u32()? as u64;
            (tags, next)
        };

        ifds.push(Ifd {
            tags: entry_count,
            index,
        });

        offset = next_offset;
        index += 1;

        // Safety: prevent infinite loops from malformed files.
        if index > 10_000 {
            return Err(Error::Other("IFD chain exceeds 10,000 entries".into()));
        }
    }

    Ok(ifds)
}

/// Parse classic TIFF IFD entries (12 bytes each).
fn parse_tags_classic(
    cursor: &mut Cursor<'_>,
    count: usize,
    data: &[u8],
    byte_order: ByteOrder,
) -> Result<Vec<Tag>> {
    let mut tags = Vec::with_capacity(count);
    for _ in 0..count {
        let code = cursor.read_u16()?;
        let type_code = cursor.read_u16()?;
        let value_count = cursor.read_u32()? as u64;
        let value_offset_bytes = cursor.read_bytes(4)?;
        let tag = Tag::parse_classic(code, type_code, value_count, value_offset_bytes, data, byte_order)?;
        tags.push(tag);
    }
    Ok(tags)
}

/// Parse BigTIFF IFD entries (20 bytes each).
fn parse_tags_bigtiff(
    cursor: &mut Cursor<'_>,
    count: usize,
    data: &[u8],
    byte_order: ByteOrder,
) -> Result<Vec<Tag>> {
    let mut tags = Vec::with_capacity(count);
    for _ in 0..count {
        let code = cursor.read_u16()?;
        let type_code = cursor.read_u16()?;
        let value_count = cursor.read_u64()?;
        let value_offset_bytes = cursor.read_bytes(8)?;
        let tag = Tag::parse_bigtiff(code, type_code, value_count, value_offset_bytes, data, byte_order)?;
        tags.push(tag);
    }
    Ok(tags)
}
