//! Pure-Rust, read-only TIFF and BigTIFF file decoder.
//!
//! Supports:
//! - **TIFF** (classic): `II`/`MM` byte order mark + version 42
//! - **BigTIFF**: `II`/`MM` byte order mark + version 43
//!
//! # Example
//!
//! ```no_run
//! use tiff_reader::TiffFile;
//!
//! let file = TiffFile::open("image.tif").unwrap();
//! println!("byte order: {:?}", file.byte_order());
//! println!("IFD count: {}", file.ifd_count());
//!
//! let ifd = file.ifd(0).unwrap();
//! println!("  width: {}", ifd.width());
//! println!("  height: {}", ifd.height());
//! println!("  bits per sample: {:?}", ifd.bits_per_sample());
//! ```

pub mod error;
pub mod io;

// Core TIFF structures
pub mod header;
pub mod ifd;
pub mod tag;

// Data access
pub mod strip;
pub mod tile;

// Compression filters
pub mod filters;

// Utilities
pub mod cache;

use std::path::Path;

use error::{Error, Result};
use memmap2::Mmap;

// Re-exports
pub use error::Error as TiffError;
pub use header::ByteOrder;
pub use ifd::Ifd;
pub use tag::{Tag, TagValue};

/// A memory-mapped TIFF file handle.
pub struct TiffFile {
    data: TiffData,
    header: header::TiffHeader,
    ifds: Vec<ifd::Ifd>,
}

/// Backing storage for TIFF data: either memory-mapped or owned bytes.
enum TiffData {
    Mmap(Mmap),
    Bytes(Vec<u8>),
}

impl TiffData {
    fn as_bytes(&self) -> &[u8] {
        match self {
            TiffData::Mmap(m) => m,
            TiffData::Bytes(b) => b,
        }
    }
}

impl TiffFile {
    /// Open a TIFF file from disk using memory-mapped I/O.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = std::fs::File::open(path.as_ref())
            .map_err(|e| Error::Io(e, path.as_ref().display().to_string()))?;
        let mmap = unsafe { Mmap::map(&file) }
            .map_err(|e| Error::Io(e, path.as_ref().display().to_string()))?;
        Self::from_data(TiffData::Mmap(mmap))
    }

    /// Open a TIFF file from an owned byte buffer (WASM-compatible).
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        Self::from_data(TiffData::Bytes(data))
    }

    fn from_data(data: TiffData) -> Result<Self> {
        let bytes = data.as_bytes();
        let header = header::TiffHeader::parse(bytes)?;
        let ifds = ifd::parse_ifd_chain(bytes, &header)?;
        Ok(Self { data, header, ifds })
    }

    /// Returns the byte order of the TIFF file.
    pub fn byte_order(&self) -> ByteOrder {
        self.header.byte_order
    }

    /// Returns `true` if this is a BigTIFF file.
    pub fn is_bigtiff(&self) -> bool {
        self.header.is_bigtiff()
    }

    /// Returns the number of IFDs (images/pages) in the file.
    pub fn ifd_count(&self) -> usize {
        self.ifds.len()
    }

    /// Returns the IFD at the given index.
    pub fn ifd(&self, index: usize) -> Result<&Ifd> {
        self.ifds.get(index).ok_or(Error::IfdNotFound(index))
    }

    /// Returns the raw file bytes.
    pub fn raw_bytes(&self) -> &[u8] {
        self.data.as_bytes()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        // TODO: add tests once core parsing is implemented
    }
}
