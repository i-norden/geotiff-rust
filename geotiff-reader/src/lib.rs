//! Pure-Rust GeoTIFF and Cloud Optimized GeoTIFF (COG) reader.
//!
//! Supports:
//! - **GeoTIFF**: TIFF files with GeoKey metadata (EPSG codes, CRS, tiepoints, pixel scale)
//! - **COG**: Cloud Optimized GeoTIFF with tiled access and overview levels
//!
//! # Example
//!
//! ```ignore
//! use geotiff_reader::GeoTiffFile;
//!
//! let file = GeoTiffFile::open("dem.tif")?;
//! println!("CRS: {:?}", file.crs());
//! println!("bounds: {:?}", file.geo_bounds());
//! println!("size: {}x{}", file.width(), file.height());
//! ```

pub mod error;
pub mod geokeys;
pub mod crs;
pub mod transform;

#[cfg(feature = "cog")]
pub mod cog;

pub use error::{Error, Result};

use std::path::Path;

use memmap2::Mmap;
use ndarray::ArrayD;

/// A GeoTIFF file handle with geospatial metadata.
pub struct GeoTiffFile {
    data: GeoTiffData,
    geo_metadata: GeoMetadata,
}

/// Backing storage: memory-mapped or owned bytes.
enum GeoTiffData {
    Mmap(Mmap),
    Bytes(Vec<u8>),
}

impl GeoTiffData {
    fn as_bytes(&self) -> &[u8] {
        match self {
            GeoTiffData::Mmap(m) => m,
            GeoTiffData::Bytes(b) => b,
        }
    }
}

/// Parsed geospatial metadata from GeoKeys and model tags.
#[derive(Debug, Clone)]
pub struct GeoMetadata {
    /// EPSG code for the coordinate reference system, if present.
    pub epsg: Option<u32>,
    /// Model tiepoints: (I, J, K, X, Y, Z) tuples.
    pub tiepoints: Vec<[f64; 6]>,
    /// Pixel scale: (ScaleX, ScaleY, ScaleZ).
    pub pixel_scale: Option<[f64; 3]>,
    /// 4x4 model transformation matrix (row-major), if present.
    pub transformation: Option<[f64; 16]>,
    /// Nodata value as a string (parsed from GDAL_NODATA tag).
    pub nodata: Option<String>,
    /// Number of bands (samples per pixel).
    pub band_count: u32,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
}

impl GeoTiffFile {
    /// Open a GeoTIFF file from disk using memory-mapped I/O.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = std::fs::File::open(path.as_ref())
            .map_err(|e| Error::Io(e, path.as_ref().display().to_string()))?;
        let mmap = unsafe { Mmap::map(&file) }
            .map_err(|e| Error::Io(e, path.as_ref().display().to_string()))?;
        Self::from_data(GeoTiffData::Mmap(mmap))
    }

    /// Open a GeoTIFF from an owned byte buffer (WASM-compatible).
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        Self::from_data(GeoTiffData::Bytes(data))
    }

    fn from_data(_data: GeoTiffData) -> Result<Self> {
        // TODO: parse TIFF structure, extract GeoKeys, build GeoMetadata
        todo!("GeoTIFF parsing not yet implemented")
    }

    /// Returns the EPSG code of the coordinate reference system, if present.
    pub fn epsg(&self) -> Option<u32> {
        self.geo_metadata.epsg
    }

    /// Returns the CRS information.
    pub fn crs(&self) -> &GeoMetadata {
        &self.geo_metadata
    }

    /// Returns the geographic bounds as (min_x, min_y, max_x, max_y).
    pub fn geo_bounds(&self) -> Option<[f64; 4]> {
        let scale = self.geo_metadata.pixel_scale.as_ref()?;
        let tp = self.geo_metadata.tiepoints.first()?;
        let min_x = tp[3] - tp[0] * scale[0];
        let max_y = tp[4] + tp[1] * scale[1];
        let max_x = min_x + self.geo_metadata.width as f64 * scale[0];
        let min_y = max_y - self.geo_metadata.height as f64 * scale[1];
        Some([min_x, min_y, max_x, max_y])
    }

    /// Returns the image width in pixels.
    pub fn width(&self) -> u32 {
        self.geo_metadata.width
    }

    /// Returns the image height in pixels.
    pub fn height(&self) -> u32 {
        self.geo_metadata.height
    }

    /// Returns the number of bands.
    pub fn band_count(&self) -> u32 {
        self.geo_metadata.band_count
    }

    /// Returns the nodata value, if set.
    pub fn nodata(&self) -> Option<&str> {
        self.geo_metadata.nodata.as_deref()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        // TODO: add tests once core parsing is implemented
    }
}
