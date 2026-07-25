use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TIFF writer error: {0}")]
    Tiff(#[from] tiff_writer::Error),

    #[error("GeoKey serialization error: {0}")]
    GeoKey(#[from] geotiff_core::GeoKeySerializeError),

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("tile ({x_off},{y_off}) out of bounds for {width}x{height} raster")]
    TileOutOfBounds {
        x_off: usize,
        y_off: usize,
        width: u32,
        height: u32,
    },

    #[error(
        "tile ({x_off},{y_off}) has shape {actual_height}x{actual_width}; expected {expected_height}x{expected_width}"
    )]
    TileShapeMismatch {
        x_off: usize,
        y_off: usize,
        expected_height: usize,
        expected_width: usize,
        actual_height: usize,
        actual_width: usize,
    },

    #[error("tile ({x_off},{y_off}) has already been written")]
    TileAlreadyWritten { x_off: usize, y_off: usize },

    #[error("data size mismatch: expected {expected}, got {actual}")]
    DataSizeMismatch { expected: usize, actual: usize },

    #[error("{0}")]
    Other(String),
}
