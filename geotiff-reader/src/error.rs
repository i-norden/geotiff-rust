use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error reading {1}: {0}")]
    Io(#[source] std::io::Error, String),

    #[error("TIFF error: {0}")]
    #[cfg(feature = "local")]
    Tiff(#[from] tiff_reader::TiffError),

    #[error("HTTP error: {0}")]
    #[cfg(any(feature = "cog", feature = "cog-async"))]
    Http(#[from] reqwest::Error),

    #[error("not a GeoTIFF: missing GeoTIFF metadata")]
    NotGeoTiff,

    #[error("invalid GeoKey directory")]
    InvalidGeoKeyDirectory,

    #[error("invalid GeoTIFF tag {tag}: {reason}")]
    InvalidGeoTiffTag { tag: u16, reason: String },

    #[error("overview index {0} not found")]
    OverviewNotFound(usize),

    #[error("overview index {0} is stored in a SubIFD and has no top-level TIFF IFD index")]
    OverviewHasNoTopLevelIfdIndex(usize),

    #[error("{0}")]
    Other(String),
}
