//! Re-export WriteSample from tiff-writer and add lightweight numeric helpers.

pub use tiff_writer::TiffWriteSample as WriteSample;

/// Numeric conversions used internally by overview generation and fill handling.
#[doc(hidden)]
pub trait NumericSample: WriteSample + PartialEq {
    fn zero() -> Self;
    fn to_f64(self) -> f64;
    /// Lossy conversion used by overview resampling; integer types round to
    /// the nearest value.
    fn from_f64(value: f64) -> Self;
    /// Checked conversion that returns `None` when `value` cannot be stored
    /// exactly: out of range for the target type, or fractional for integer
    /// types.
    fn try_from_f64(value: f64) -> Option<Self>;
    /// Parse an exact textual representation before falling back to `f64`.
    fn parse_exact(_value: &str) -> Option<Self> {
        None
    }
}

pub(crate) fn parse_nodata_value<T: NumericSample>(
    nodata: &Option<String>,
) -> crate::error::Result<Option<T>> {
    let Some(nd) = nodata.as_ref() else {
        return Ok(None);
    };
    let trimmed = nd.trim();
    if let Some(value) = T::parse_exact(trimmed) {
        return Ok(Some(value));
    }
    let Ok(value) = trimmed.parse::<f64>() else {
        // Non-numeric nodata text stays metadata-only; fills default to zero.
        return Ok(None);
    };
    match T::try_from_f64(value) {
        Some(value) => Ok(Some(value)),
        None => Err(crate::error::Error::InvalidConfig(format!(
            "nodata value {nd:?} is not representable as {}",
            std::any::type_name::<T>()
        ))),
    }
}

pub(crate) fn nodata_fill_or_zero<T: NumericSample>(
    nodata: &Option<String>,
) -> crate::error::Result<T> {
    Ok(parse_nodata_value(nodata)?.unwrap_or_else(T::zero))
}

macro_rules! impl_numeric_sample_int {
    ($ty:ty) => {
        impl NumericSample for $ty {
            fn zero() -> Self {
                0
            }

            fn to_f64(self) -> f64 {
                self as f64
            }

            fn from_f64(value: f64) -> Self {
                value.round() as $ty
            }

            fn try_from_f64(value: f64) -> Option<Self> {
                if !value.is_finite() {
                    return None;
                }
                // `as` saturates, so converting back detects values outside
                // the target range; comparing against the original value also
                // rejects fractional inputs.
                let converted = value.round() as $ty;
                (converted as f64 == value).then_some(converted)
            }

            fn parse_exact(value: &str) -> Option<Self> {
                value.parse::<$ty>().ok()
            }
        }
    };
}

macro_rules! impl_numeric_sample_float {
    ($ty:ty) => {
        impl NumericSample for $ty {
            fn zero() -> Self {
                0.0
            }

            fn to_f64(self) -> f64 {
                self as f64
            }

            fn from_f64(value: f64) -> Self {
                value as $ty
            }

            fn try_from_f64(value: f64) -> Option<Self> {
                let converted = value as $ty;
                if value.is_finite() && !converted.is_finite() {
                    return None; // magnitude exceeds the target float range
                }
                Some(converted)
            }
        }
    };
}

impl_numeric_sample_int!(u8);
impl_numeric_sample_int!(i8);
impl_numeric_sample_int!(u16);
impl_numeric_sample_int!(i16);
impl_numeric_sample_int!(u32);
impl_numeric_sample_int!(i32);
impl NumericSample for u64 {
    fn zero() -> Self {
        0
    }

    fn to_f64(self) -> f64 {
        self as f64
    }

    fn from_f64(value: f64) -> Self {
        value.round() as Self
    }

    fn try_from_f64(value: f64) -> Option<Self> {
        const EXCLUSIVE_UPPER_BOUND: f64 = 18_446_744_073_709_551_616.0;
        if !value.is_finite()
            || !(0.0..EXCLUSIVE_UPPER_BOUND).contains(&value)
            || value.fract() != 0.0
        {
            return None;
        }
        Some(value as Self)
    }

    fn parse_exact(value: &str) -> Option<Self> {
        value.parse().ok()
    }
}

impl NumericSample for i64 {
    fn zero() -> Self {
        0
    }

    fn to_f64(self) -> f64 {
        self as f64
    }

    fn from_f64(value: f64) -> Self {
        value.round() as Self
    }

    fn try_from_f64(value: f64) -> Option<Self> {
        const INCLUSIVE_LOWER_BOUND: f64 = -9_223_372_036_854_775_808.0;
        const EXCLUSIVE_UPPER_BOUND: f64 = 9_223_372_036_854_775_808.0;
        if !value.is_finite()
            || !(INCLUSIVE_LOWER_BOUND..EXCLUSIVE_UPPER_BOUND).contains(&value)
            || value.fract() != 0.0
        {
            return None;
        }
        Some(value as Self)
    }

    fn parse_exact(value: &str) -> Option<Self> {
        value.parse().ok()
    }
}
#[cfg(feature = "f16")]
impl NumericSample for half::f16 {
    fn zero() -> Self {
        half::f16::ZERO
    }

    fn to_f64(self) -> f64 {
        half::f16::to_f64(self)
    }

    fn from_f64(value: f64) -> Self {
        half::f16::from_f64(value)
    }

    fn try_from_f64(value: f64) -> Option<Self> {
        let converted = half::f16::from_f64(value);
        if value.is_finite() && !converted.is_finite() {
            return None;
        }
        Some(converted)
    }
}
impl_numeric_sample_float!(f32);
impl_numeric_sample_float!(f64);

#[cfg(test)]
mod tests {
    use super::{nodata_fill_or_zero, parse_nodata_value, NumericSample};

    #[test]
    fn from_f64_rounds_integer_types_to_nearest() {
        assert_eq!(u8::from_f64(1.5), 2);
        assert_eq!(u8::from_f64(1.4), 1);
        assert_eq!(i16::from_f64(-2.5), -3);
        assert_eq!(f32::from_f64(1.5), 1.5);
    }

    #[test]
    fn try_from_f64_rejects_out_of_range_and_fractional_integers() {
        assert_eq!(u8::try_from_f64(-9999.0), None);
        assert_eq!(u8::try_from_f64(256.0), None);
        assert_eq!(u8::try_from_f64(42.4), None);
        assert_eq!(u8::try_from_f64(255.0), Some(255));
        assert_eq!(i16::try_from_f64(-9999.0), Some(-9999));
        assert_eq!(i32::try_from_f64(f64::NAN), None);
    }

    #[test]
    fn try_from_f64_bounds_float_conversions() {
        assert_eq!(f32::try_from_f64(1e39), None);
        assert_eq!(f32::try_from_f64(-9999.0), Some(-9999.0));
        assert!(f32::try_from_f64(f64::NAN).unwrap().is_nan());
        assert_eq!(f64::try_from_f64(1e308), Some(1e308));
    }

    #[test]
    fn parse_nodata_value_errors_on_unrepresentable_values() {
        let nodata = Some("-9999".to_string());
        assert_eq!(parse_nodata_value::<i16>(&nodata).unwrap(), Some(-9999));
        assert!(parse_nodata_value::<u8>(&nodata).is_err());

        assert_eq!(parse_nodata_value::<u8>(&None).unwrap(), None);
        assert_eq!(
            parse_nodata_value::<u8>(&Some("not a number".to_string())).unwrap(),
            None
        );
        assert_eq!(nodata_fill_or_zero::<u8>(&None).unwrap(), 0);

        assert_eq!(
            parse_nodata_value::<u64>(&Some(u64::MAX.to_string())).unwrap(),
            Some(u64::MAX)
        );
        assert_eq!(
            parse_nodata_value::<i64>(&Some(i64::MIN.to_string())).unwrap(),
            Some(i64::MIN)
        );
        assert!(parse_nodata_value::<u64>(&Some("18446744073709551616".to_string())).is_err());
        assert!(parse_nodata_value::<i64>(&Some("9223372036854775808".to_string())).is_err());
    }
}
