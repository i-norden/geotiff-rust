use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error reading {1}: {0}")]
    Io(#[source] std::io::Error, String),

    #[error("TIFF error: {0}")]
    #[cfg(feature = "local")]
    Tiff(#[from] tiff_reader::TiffError),

    #[error("not a GeoTIFF: missing GeoKey directory (tag 34735)")]
    NotGeoTiff,

    #[error("unsupported GeoKey model type: {0}")]
    UnsupportedModelType(u16),

    #[error("EPSG code {0} not recognized")]
    UnknownEpsg(u32),

    #[error("no pixel scale or transformation matrix found")]
    NoGeoTransform,

    #[error("{0}")]
    Other(String),
}
