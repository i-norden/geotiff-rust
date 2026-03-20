//! Pure-Rust GeoTIFF reader with optional HTTP range-backed remote access.
//!
//! Supports:
//! - **GeoTIFF**: TIFF files with GeoKey metadata (EPSG codes, CRS, tiepoints, pixel scale)
//! - **COG**: overview discovery plus optional remote open via HTTP range requests
//!
//! # Example
//!
//! ```no_run
//! use geotiff_reader::GeoTiffFile;
//!
//! let file = GeoTiffFile::open("dem.tif")?;
//! println!("EPSG: {:?}", file.epsg());
//! println!("bounds: {:?}", file.geo_bounds());
//! println!("size: {}x{}", file.width(), file.height());
//! # Ok::<(), geotiff_reader::Error>(())
//! ```

pub mod crs;
pub mod error;
pub mod geokeys;
pub mod transform;

#[cfg(feature = "cog")]
pub mod cog;

pub use error::{Error, Result};

use std::path::Path;

use crs::CrsInfo;
use geokeys::GeoKeyDirectory;
use ndarray::ArrayD;
#[cfg(feature = "local")]
use tiff_reader::{TagValue, TiffFile, TiffSample};
use transform::GeoTransform;

const TAG_MODEL_PIXEL_SCALE: u16 = 33550;
const TAG_MODEL_TIEPOINT: u16 = 33922;
const TAG_MODEL_TRANSFORMATION: u16 = 34264;
const TAG_GEO_KEY_DIRECTORY: u16 = 34735;
const TAG_GEO_DOUBLE_PARAMS: u16 = 34736;
const TAG_GEO_ASCII_PARAMS: u16 = 34737;
const TAG_GDAL_NODATA: u16 = 42113;

/// A GeoTIFF file handle with geospatial metadata.
#[cfg(feature = "local")]
pub struct GeoTiffFile {
    tiff: TiffFile,
    geo_metadata: GeoMetadata,
    crs: CrsInfo,
    geokeys: GeoKeyDirectory,
    transform: Option<GeoTransform>,
    overview_ifds: Vec<usize>,
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
    /// Geographic bounds derived from the transform.
    pub geo_bounds: Option<[f64; 4]>,
}

#[cfg(feature = "local")]
impl GeoTiffFile {
    /// Open a GeoTIFF file from disk.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let tiff = TiffFile::open(path)?;
        Self::from_tiff(tiff)
    }

    /// Open a GeoTIFF from an owned byte buffer.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let tiff = TiffFile::from_bytes(data)?;
        Self::from_tiff(tiff)
    }

    pub(crate) fn from_tiff(tiff: TiffFile) -> Result<Self> {
        let ifd = tiff.ifd(0)?;
        let geokeys = parse_geokey_directory(ifd)?;
        let crs = CrsInfo::from_geokeys(&geokeys);
        let epsg = crs.epsg();
        let tiepoints = parse_tiepoints(ifd);
        let pixel_scale = parse_fixed_len_double_tag::<3>(ifd.tag(TAG_MODEL_PIXEL_SCALE).map(|tag| &tag.value));
        let transformation = parse_fixed_len_double_tag::<16>(ifd.tag(TAG_MODEL_TRANSFORMATION).map(|tag| &tag.value));
        let transform = transformation
            .as_ref()
            .map(GeoTransform::from_transformation_matrix)
            .or_else(|| {
                let tiepoint = tiepoints.first()?;
                let scale = pixel_scale.as_ref()?;
                Some(GeoTransform::from_tiepoint_and_scale(tiepoint, scale))
            });
        let geo_bounds = transform.as_ref().map(|gt| gt.bounds(ifd.width(), ifd.height()));
        let overview_ifds = tiff
            .ifds()
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(index, candidate)| {
                (candidate.width() <= ifd.width() && candidate.height() <= ifd.height()).then_some(index)
            })
            .collect();

        let geo_metadata = GeoMetadata {
            epsg,
            tiepoints,
            pixel_scale,
            transformation,
            nodata: parse_nodata(ifd),
            band_count: ifd.samples_per_pixel() as u32,
            width: ifd.width(),
            height: ifd.height(),
            geo_bounds,
        };

        Ok(Self {
            tiff,
            geo_metadata,
            crs,
            geokeys,
            transform,
            overview_ifds,
        })
    }

    /// Returns the underlying TIFF file.
    pub fn tiff(&self) -> &TiffFile {
        &self.tiff
    }

    /// Returns the parsed GeoTIFF metadata.
    pub fn metadata(&self) -> &GeoMetadata {
        &self.geo_metadata
    }

    /// Returns the EPSG code of the coordinate reference system, if present.
    pub fn epsg(&self) -> Option<u32> {
        self.geo_metadata.epsg
    }

    /// Returns the extracted CRS information.
    pub fn crs(&self) -> &CrsInfo {
        &self.crs
    }

    /// Returns the parsed GeoKey directory.
    pub fn geokeys(&self) -> &GeoKeyDirectory {
        &self.geokeys
    }

    /// Returns the affine transform, if present.
    pub fn transform(&self) -> Option<&GeoTransform> {
        self.transform.as_ref()
    }

    /// Returns the geographic bounds as `(min_x, min_y, max_x, max_y)`.
    pub fn geo_bounds(&self) -> Option<[f64; 4]> {
        self.geo_metadata.geo_bounds
    }

    /// Convert a pixel coordinate to map coordinates.
    pub fn pixel_to_geo(&self, col: f64, row: f64) -> Option<(f64, f64)> {
        self.transform.map(|transform| transform.pixel_to_geo(col, row))
    }

    /// Convert map coordinates to pixel coordinates.
    pub fn geo_to_pixel(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        self.transform.and_then(|transform| transform.geo_to_pixel(x, y))
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

    /// Returns the number of internal overview IFDs.
    pub fn overview_count(&self) -> usize {
        self.overview_ifds.len()
    }

    /// Returns the TIFF IFD index of the requested overview.
    pub fn overview_ifd_index(&self, overview_index: usize) -> Result<usize> {
        self.overview_ifds
            .get(overview_index)
            .copied()
            .ok_or(Error::OverviewNotFound(overview_index))
    }

    /// Decode the base-resolution raster into a typed ndarray.
    pub fn read_raster<T: TiffSample>(&self) -> Result<ArrayD<T>> {
        self.tiff.read_image::<T>(0).map_err(Into::into)
    }

    /// Decode an overview raster into a typed ndarray.
    pub fn read_overview<T: TiffSample>(&self, overview_index: usize) -> Result<ArrayD<T>> {
        let ifd_index = self.overview_ifd_index(overview_index)?;
        self.tiff.read_image::<T>(ifd_index).map_err(Into::into)
    }
}

#[cfg(feature = "local")]
fn parse_geokey_directory(ifd: &tiff_reader::Ifd) -> Result<GeoKeyDirectory> {
    let directory = ifd
        .tag(TAG_GEO_KEY_DIRECTORY)
        .and_then(|tag| match &tag.value {
            TagValue::Short(values) => Some(values.as_slice()),
            _ => None,
        })
        .ok_or(Error::NotGeoTiff)?;
    let double_params = ifd
        .tag(TAG_GEO_DOUBLE_PARAMS)
        .and_then(|tag| tag.value.as_f64_vec())
        .unwrap_or_default();
    let ascii_params = ifd
        .tag(TAG_GEO_ASCII_PARAMS)
        .and_then(|tag| tag.value.as_str())
        .unwrap_or("");
    GeoKeyDirectory::parse(directory, &double_params, ascii_params).ok_or(Error::InvalidGeoKeyDirectory)
}

#[cfg(feature = "local")]
fn parse_fixed_len_double_tag<const N: usize>(value: Option<&TagValue>) -> Option<[f64; N]> {
    let values = value.and_then(TagValue::as_f64_vec)?;
    if values.len() < N {
        return None;
    }
    let mut out = [0.0; N];
    out.copy_from_slice(&values[..N]);
    Some(out)
}

#[cfg(feature = "local")]
fn parse_tiepoints(ifd: &tiff_reader::Ifd) -> Vec<[f64; 6]> {
    let values = ifd
        .tag(TAG_MODEL_TIEPOINT)
        .and_then(|tag| tag.value.as_f64_vec())
        .unwrap_or_default();
    values
        .chunks_exact(6)
        .map(|chunk| [chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5]])
        .collect()
}

#[cfg(feature = "local")]
fn parse_nodata(ifd: &tiff_reader::Ifd) -> Option<String> {
    ifd.tag(TAG_GDAL_NODATA)
        .and_then(|tag| tag.value.as_str())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
#[cfg(feature = "local")]
mod tests {
    use super::GeoTiffFile;

    fn le_u16(value: u16) -> [u8; 2] {
        value.to_le_bytes()
    }

    fn le_u32(value: u32) -> [u8; 4] {
        value.to_le_bytes()
    }

    fn le_f64(value: f64) -> [u8; 8] {
        value.to_le_bytes()
    }

    fn build_simple_geotiff() -> Vec<u8> {
        let image_data = vec![10u8, 20, 30, 40];
        let tiepoints = [0.0, 0.0, 0.0, 100.0, 200.0, 0.0];
        let scales = [2.0, 2.0, 0.0];
        let geo_keys: [u16; 12] = [
            1, 1, 0, 2, // header
            1024, 0, 1, 2, // model type = Geographic
            2048, 0, 1, 4326, // EPSG:4326
        ];
        let nodata = b"-9999\0".to_vec();

        let entries = vec![
            (256u16, 4u16, 1u32, le_u32(2).to_vec()),
            (257u16, 4u16, 1u32, le_u32(2).to_vec()),
            (258u16, 3u16, 1u32, [8, 0, 0, 0].to_vec()),
            (259u16, 3u16, 1u32, [1, 0, 0, 0].to_vec()),
            (273u16, 4u16, 1u32, vec![]),
            (277u16, 3u16, 1u32, [1, 0, 0, 0].to_vec()),
            (278u16, 4u16, 1u32, le_u32(2).to_vec()),
            (279u16, 4u16, 1u32, le_u32(image_data.len() as u32).to_vec()),
            (33550u16, 12u16, 3u32, scales.iter().flat_map(|value| le_f64(*value)).collect()),
            (33922u16, 12u16, 6u32, tiepoints.iter().flat_map(|value| le_f64(*value)).collect()),
            (34735u16, 3u16, geo_keys.len() as u32, geo_keys.iter().flat_map(|value| le_u16(*value)).collect()),
            (42113u16, 2u16, nodata.len() as u32, nodata),
        ];

        let ifd_offset = 8u32;
        let ifd_size = 2 + entries.len() * 12 + 4;
        let mut next_data_offset = ifd_offset as usize + ifd_size;
        let image_offset = next_data_offset as u32;
        next_data_offset += image_data.len();

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"II");
        bytes.extend_from_slice(&le_u16(42));
        bytes.extend_from_slice(&le_u32(ifd_offset));
        bytes.extend_from_slice(&le_u16(entries.len() as u16));

        let mut deferred = Vec::new();
        for (tag, ty, count, value) in entries {
            bytes.extend_from_slice(&le_u16(tag));
            bytes.extend_from_slice(&le_u16(ty));
            bytes.extend_from_slice(&le_u32(count));
            if tag == 273 {
                bytes.extend_from_slice(&le_u32(image_offset));
            } else if value.len() <= 4 {
                let mut inline = [0u8; 4];
                inline[..value.len()].copy_from_slice(&value);
                bytes.extend_from_slice(&inline);
            } else {
                bytes.extend_from_slice(&le_u32(next_data_offset as u32));
                next_data_offset += value.len();
                deferred.push(value);
            }
        }
        bytes.extend_from_slice(&le_u32(0));
        bytes.extend_from_slice(&image_data);
        for value in deferred {
            bytes.extend_from_slice(&value);
        }
        bytes
    }

    #[test]
    fn parses_geotiff_metadata_and_reads_raster() {
        let file = GeoTiffFile::from_bytes(build_simple_geotiff()).unwrap();
        assert_eq!(file.epsg(), Some(4326));
        assert_eq!(file.width(), 2);
        assert_eq!(file.height(), 2);
        assert_eq!(file.band_count(), 1);
        assert_eq!(file.nodata(), Some("-9999"));
        assert_eq!(file.geo_bounds(), Some([100.0, 196.0, 104.0, 200.0]));

        let raster = file.read_raster::<u8>().unwrap();
        assert_eq!(raster.shape(), &[2, 2]);
        let (values, offset) = raster.into_raw_vec_and_offset();
        assert_eq!(offset, Some(0));
        assert_eq!(values, vec![10, 20, 30, 40]);
    }
}
