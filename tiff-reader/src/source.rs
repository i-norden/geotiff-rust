//! Random-access source abstraction used by the TIFF decoder.

use std::fs::File;
use std::io;
#[cfg(not(any(unix, windows)))]
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
#[cfg(not(any(unix, windows)))]
use std::sync::Mutex;

use memmap2::Mmap;

use crate::error::{Error, Result};

/// Random-access byte source for TIFF decoding.
pub trait TiffSource: Send + Sync {
    /// Total object length in bytes.
    fn len(&self) -> u64;

    /// Returns `true` when the source has no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read exactly `len` bytes starting at `offset`.
    fn read_exact_at(&self, offset: u64, len: usize) -> Result<Vec<u8>>;

    /// Expose a whole-object slice when the source is fully resident in memory.
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }
}

/// Shared source handle used by `TiffFile`.
pub type SharedSource = Arc<dyn TiffSource>;

/// Safe file-backed source using random-access reads.
pub struct FileSource {
    #[cfg(any(unix, windows))]
    file: File,
    #[cfg(not(any(unix, windows)))]
    file: Mutex<File>,
    len: u64,
    path: String,
}

impl FileSource {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| Error::Io(e, path.display().to_string()))?;
        let len = file
            .metadata()
            .map_err(|e| Error::Io(e, path.display().to_string()))?
            .len();
        let path = path.display().to_string();
        #[cfg(any(unix, windows))]
        {
            Ok(Self { file, len, path })
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(Self {
                file: Mutex::new(file),
                len,
                path,
            })
        }
    }
}

impl TiffSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_exact_at(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        checked_read_range(offset, len, self.len)?;
        let mut bytes = vec![0; len];
        read_file_exact_at(self, offset, &mut bytes)
            .map_err(|e| Error::Io(e, self.path.clone()))?;
        Ok(bytes)
    }
}

/// Memory-mapped file source.
pub struct MmapSource {
    mmap: Mmap,
}

impl MmapSource {
    /// Open a file-backed source by memory-mapping the whole file.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that the mapped file is not mutated or
    /// truncated for the lifetime of the returned source. This includes writes
    /// through other file handles and writes from other processes.
    pub unsafe fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| Error::Io(e, path.display().to_string()))?;
        let mmap =
            unsafe { Mmap::map(&file) }.map_err(|e| Error::Io(e, path.display().to_string()))?;
        Ok(Self { mmap })
    }
}

impl TiffSource for MmapSource {
    fn len(&self) -> u64 {
        self.mmap.len() as u64
    }

    fn read_exact_at(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let range = checked_slice_range(offset, len, self.len())?;
        Ok(self.mmap[range].to_vec())
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(&self.mmap)
    }
}

/// In-memory byte-vector source.
pub struct BytesSource {
    bytes: Vec<u8>,
}

impl BytesSource {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

impl TiffSource for BytesSource {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_exact_at(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let range = checked_slice_range(offset, len, self.len())?;
        Ok(self.bytes[range].to_vec())
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(&self.bytes)
    }
}

fn checked_read_range(offset: u64, len: usize, data_len: u64) -> Result<u64> {
    let length = u64::try_from(len).map_err(|_| Error::OffsetOutOfBounds {
        offset,
        length: u64::MAX,
        data_len,
    })?;
    let end = offset.checked_add(length).ok_or(Error::OffsetOutOfBounds {
        offset,
        length,
        data_len,
    })?;
    if end > data_len {
        return Err(Error::OffsetOutOfBounds {
            offset,
            length,
            data_len,
        });
    }
    Ok(end)
}

fn checked_slice_range(offset: u64, len: usize, data_len: u64) -> Result<Range<usize>> {
    checked_read_range(offset, len, data_len)?;
    let start = usize::try_from(offset).map_err(|_| Error::OffsetOutOfBounds {
        offset,
        length: len as u64,
        data_len,
    })?;
    let end = start.checked_add(len).ok_or(Error::OffsetOutOfBounds {
        offset,
        length: len as u64,
        data_len,
    })?;
    Ok(start..end)
}

#[cfg(unix)]
fn read_file_exact_at(source: &FileSource, offset: u64, bytes: &mut [u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt;

    read_with_offset_loop(bytes, offset, |buf, current_offset| {
        source.file.read_at(buf, current_offset)
    })
}

#[cfg(windows)]
fn read_file_exact_at(source: &FileSource, offset: u64, bytes: &mut [u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;

    read_with_offset_loop(bytes, offset, |buf, current_offset| {
        source.file.seek_read(buf, current_offset)
    })
}

#[cfg(not(any(unix, windows)))]
fn read_file_exact_at(source: &FileSource, offset: u64, bytes: &mut [u8]) -> io::Result<()> {
    let mut file = source
        .file
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "file source lock poisoned"))?;
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(bytes)
}

#[cfg(any(unix, windows))]
fn read_with_offset_loop(
    bytes: &mut [u8],
    offset: u64,
    mut read_at: impl FnMut(&mut [u8], u64) -> io::Result<usize>,
) -> io::Result<()> {
    let mut total_read = 0usize;
    while total_read < bytes.len() {
        let current_offset = offset + total_read as u64;
        match read_at(&mut bytes[total_read..], current_offset) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "failed to fill whole buffer",
                ));
            }
            Ok(read) => total_read += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
