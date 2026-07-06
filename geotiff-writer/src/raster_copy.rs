//! Row-slice copies from `ndarray` views into row-major block buffers.
//!
//! These helpers replace per-element indexed copies in the tile/strip
//! extraction paths. When the source view is contiguous the row copy is a
//! single `copy_from_slice`; otherwise it falls back to iterator order,
//! which matches the logical row-major layout of the views.

use ndarray::{s, ArrayView2, ArrayView3};

/// Source rectangle copied by the region helpers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Region {
    pub row_start: usize,
    pub col_start: usize,
    pub rows: usize,
    pub cols: usize,
}

fn copy_row<T: Copy>(dest: &mut [T], src: ndarray::ArrayView1<'_, T>) {
    if let Some(src) = src.as_slice() {
        dest.copy_from_slice(src);
    } else {
        for (dest_value, src_value) in dest.iter_mut().zip(src.iter()) {
            *dest_value = *src_value;
        }
    }
}

/// Copy `rows x cols` starting at `(row_start, col_start)` into a
/// `dest_width`-wide row-major buffer, leaving other positions untouched.
pub(crate) fn copy_2d_region_into<T: Copy>(
    data: &ArrayView2<'_, T>,
    region: Region,
    dest: &mut [T],
    dest_width: usize,
) {
    for row in 0..region.rows {
        let src = data.slice(s![
            region.row_start + row,
            region.col_start..region.col_start + region.cols
        ]);
        copy_row(
            &mut dest[row * dest_width..row * dest_width + region.cols],
            src,
        );
    }
}

/// Copy a band-interleaved `rows x cols x bands` region into a row-major
/// buffer whose rows are `dest_row_len` samples wide.
pub(crate) fn copy_3d_chunky_region_into<T: Copy>(
    data: &ArrayView3<'_, T>,
    region: Region,
    dest: &mut [T],
    dest_row_len: usize,
) {
    let bands = data.dim().2;
    for row in 0..region.rows {
        let src = data.slice(s![
            region.row_start + row,
            region.col_start..region.col_start + region.cols,
            ..
        ]);
        let dest_row = &mut dest[row * dest_row_len..row * dest_row_len + region.cols * bands];
        if let Some(src) = src.as_slice() {
            dest_row.copy_from_slice(src);
        } else {
            for (dest_value, src_value) in dest_row.iter_mut().zip(src.iter()) {
                *dest_value = *src_value;
            }
        }
    }
}

/// Copy one band of a `rows x cols` region into a `dest_width`-wide
/// row-major buffer, leaving other positions untouched.
pub(crate) fn copy_3d_band_region_into<T: Copy>(
    data: &ArrayView3<'_, T>,
    band: usize,
    region: Region,
    dest: &mut [T],
    dest_width: usize,
) {
    for row in 0..region.rows {
        let src = data.slice(s![
            region.row_start + row,
            region.col_start..region.col_start + region.cols,
            band
        ]);
        copy_row(
            &mut dest[row * dest_width..row * dest_width + region.cols],
            src,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        copy_2d_region_into, copy_3d_band_region_into, copy_3d_chunky_region_into, Region,
    };
    use ndarray::{Array2, Array3};

    fn region(row_start: usize, col_start: usize, rows: usize, cols: usize) -> Region {
        Region {
            row_start,
            col_start,
            rows,
            cols,
        }
    }

    #[test]
    fn copies_2d_regions_for_standard_and_transposed_layouts() {
        let data = Array2::from_shape_fn((4, 5), |(row, col)| (row * 10 + col) as i32);

        let mut dest = vec![-1; 3 * 3];
        copy_2d_region_into(&data.view(), region(1, 2, 2, 3), &mut dest, 3);
        assert_eq!(dest, vec![12, 13, 14, 22, 23, 24, -1, -1, -1]);

        // Transposed view (5x4): exercises the non-contiguous fallback path.
        let transposed = data.t();
        let mut dest = vec![-1; 3 * 3];
        copy_2d_region_into(&transposed, region(1, 1, 2, 3), &mut dest, 3);
        assert_eq!(dest, vec![11, 21, 31, 12, 22, 32, -1, -1, -1]);
    }

    #[test]
    fn copies_3d_chunky_and_band_regions() {
        let data = Array3::from_shape_fn((3, 3, 2), |(row, col, band)| {
            (row * 100 + col * 10 + band) as i32
        });

        let mut dest = vec![-1; 2 * 2 * 2];
        copy_3d_chunky_region_into(&data.view(), region(1, 1, 2, 2), &mut dest, 4);
        assert_eq!(dest, vec![110, 111, 120, 121, 210, 211, 220, 221]);

        let mut dest = vec![-1; 2 * 2];
        copy_3d_band_region_into(&data.view(), 1, region(1, 1, 2, 2), &mut dest, 2);
        assert_eq!(dest, vec![111, 121, 211, 221]);
    }
}
