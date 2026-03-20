//! Compression filter pipeline for TIFF strip/tile decompression.

use crate::error::{Error, Result};

/// Decompress a strip or tile according to the TIFF compression tag.
pub fn decompress(compression: u16, data: &[u8], _expected_size: usize) -> Result<Vec<u8>> {
    match compression {
        1 => {
            // No compression.
            Ok(data.to_vec())
        }
        8 | 32946 => {
            // Deflate (zlib / Adobe deflate).
            decompress_deflate(data)
        }
        5 => {
            // LZW.
            decompress_lzw(data)
        }
        32773 => {
            // PackBits.
            decompress_packbits(data)
        }
        #[cfg(feature = "jpeg")]
        6 | 7 => {
            // JPEG (old-style 6, new-style 7).
            decompress_jpeg(data)
        }
        #[cfg(feature = "zstd")]
        50000 => {
            // ZSTD.
            decompress_zstd(data)
        }
        _ => Err(Error::UnsupportedCompression(compression)),
    }
}

fn decompress_deflate(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| Error::DecompressionFailed {
            index: 0,
            reason: format!("deflate: {e}"),
        })?;
    Ok(out)
}

fn decompress_lzw(data: &[u8]) -> Result<Vec<u8>> {
    use weezl::decode::Decoder;
    use weezl::BitOrder;

    let mut decoder = Decoder::new(BitOrder::Msb, 8);
    decoder.decode(data).map_err(|e| Error::DecompressionFailed {
        index: 0,
        reason: format!("LZW: {e}"),
    })
}

fn decompress_packbits(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let n = data[i] as i8;
        i += 1;
        if n >= 0 {
            // Copy next (n+1) bytes literally.
            let count = n as usize + 1;
            if i + count > data.len() {
                break;
            }
            out.extend_from_slice(&data[i..i + count]);
            i += count;
        } else if n > -128 {
            // Repeat next byte (1-n) times.
            let count = (1 - n as i32) as usize;
            if i >= data.len() {
                break;
            }
            let byte = data[i];
            i += 1;
            out.resize(out.len() + count, byte);
        }
        // n == -128: no-op
    }
    Ok(out)
}

#[cfg(feature = "jpeg")]
fn decompress_jpeg(data: &[u8]) -> Result<Vec<u8>> {
    use jpeg_decoder::Decoder;

    let mut decoder = Decoder::new(data);
    decoder
        .decode()
        .map_err(|e| Error::DecompressionFailed {
            index: 0,
            reason: format!("JPEG: {e}"),
        })
}

#[cfg(feature = "zstd")]
fn decompress_zstd(data: &[u8]) -> Result<Vec<u8>> {
    zstd::bulk::decompress(data, 64 * 1024 * 1024).map_err(|e| Error::DecompressionFailed {
        index: 0,
        reason: format!("ZSTD: {e}"),
    })
}
