//! GeoKey directory parsing (TIFF tag 34735).
//!
//! The GeoKey directory is stored as a TIFF SHORT array with the structure:
//! - Header: KeyDirectoryVersion, KeyRevision, MinorRevision, NumberOfKeys
//! - Entries: KeyID, TIFFTagLocation, Count, ValueOffset (repeated)
//!
//! GeoKeys reference values either inline (location=0), from the
//! GeoDoubleParams tag (34736), or from the GeoAsciiParams tag (34737).

// Well-known GeoKey IDs.
pub const GT_MODEL_TYPE: u16 = 1024;
pub const GT_RASTER_TYPE: u16 = 1025;
pub const GEOGRAPHIC_TYPE: u16 = 2048;
pub const GEOG_CITATION: u16 = 2049;
pub const GEOG_GEODETIC_DATUM: u16 = 2050;
pub const GEOG_ANGULAR_UNITS: u16 = 2054;
pub const PROJECTED_CS_TYPE: u16 = 3072;
pub const PROJ_CITATION: u16 = 3073;
pub const PROJECTION: u16 = 3074;
pub const PROJ_COORD_TRANS: u16 = 3075;
pub const PROJ_LINEAR_UNITS: u16 = 3076;
pub const VERTICAL_CS_TYPE: u16 = 4096;
pub const VERTICAL_DATUM: u16 = 4098;
pub const VERTICAL_UNITS: u16 = 4099;

/// A parsed GeoKey entry.
#[derive(Debug, Clone)]
pub struct GeoKey {
    pub id: u16,
    pub value: GeoKeyValue,
}

/// The value of a GeoKey.
#[derive(Debug, Clone)]
pub enum GeoKeyValue {
    /// Short value stored inline.
    Short(u16),
    /// Double value(s) from GeoDoubleParams.
    Double(Vec<f64>),
    /// ASCII string from GeoAsciiParams.
    Ascii(String),
}

/// Parsed GeoKey directory.
#[derive(Debug, Clone)]
pub struct GeoKeyDirectory {
    pub version: u16,
    pub major_revision: u16,
    pub minor_revision: u16,
    pub keys: Vec<GeoKey>,
}

impl GeoKeyDirectory {
    /// Parse the GeoKey directory from the three GeoTIFF tags.
    ///
    /// - `directory`: contents of tag 34735 (SHORT array)
    /// - `double_params`: contents of tag 34736 (DOUBLE array), may be empty
    /// - `ascii_params`: contents of tag 34737 (ASCII), may be empty
    pub fn parse(
        directory: &[u16],
        double_params: &[f64],
        ascii_params: &str,
    ) -> Option<Self> {
        if directory.len() < 4 {
            return None;
        }

        let version = directory[0];
        let major_revision = directory[1];
        let minor_revision = directory[2];
        let num_keys = directory[3] as usize;

        if directory.len() < 4 + num_keys * 4 {
            return None;
        }

        let mut keys = Vec::with_capacity(num_keys);
        for i in 0..num_keys {
            let base = 4 + i * 4;
            let key_id = directory[base];
            let location = directory[base + 1];
            let count = directory[base + 2] as usize;
            let value_offset = directory[base + 3];

            let value = match location {
                0 => {
                    // Value is the offset itself (short).
                    GeoKeyValue::Short(value_offset)
                }
                34736 => {
                    // Value is in GeoDoubleParams.
                    let start = value_offset as usize;
                    let end = start + count;
                    if end <= double_params.len() {
                        GeoKeyValue::Double(double_params[start..end].to_vec())
                    } else {
                        continue;
                    }
                }
                34737 => {
                    // Value is in GeoAsciiParams.
                    let start = value_offset as usize;
                    let end = start + count;
                    if end <= ascii_params.len() {
                        let s = ascii_params[start..end]
                            .trim_end_matches('|')
                            .trim_end_matches('\0')
                            .to_string();
                        GeoKeyValue::Ascii(s)
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };

            keys.push(GeoKey { id: key_id, value });
        }

        Some(Self {
            version,
            major_revision,
            minor_revision,
            keys,
        })
    }

    /// Look up a GeoKey by ID.
    pub fn get(&self, id: u16) -> Option<&GeoKey> {
        self.keys.iter().find(|k| k.id == id)
    }

    /// Get a short value for a key.
    pub fn get_short(&self, id: u16) -> Option<u16> {
        self.get(id).and_then(|k| match &k.value {
            GeoKeyValue::Short(v) => Some(*v),
            _ => None,
        })
    }

    /// Get an ASCII value for a key.
    pub fn get_ascii(&self, id: u16) -> Option<&str> {
        self.get(id).and_then(|k| match &k.value {
            GeoKeyValue::Ascii(s) => Some(s.as_str()),
            _ => None,
        })
    }

    /// Get double value(s) for a key.
    pub fn get_double(&self, id: u16) -> Option<&[f64]> {
        self.get(id).and_then(|k| match &k.value {
            GeoKeyValue::Double(v) => Some(v.as_slice()),
            _ => None,
        })
    }
}
