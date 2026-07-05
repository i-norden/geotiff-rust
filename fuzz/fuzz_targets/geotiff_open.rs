#![no_main]

use geotiff_reader::GeoTiffFile;
use libfuzzer_sys::fuzz_target;

const MAX_DECODED_BYTES: usize = 8 * 1024 * 1024;
const MAX_OVERVIEWS: usize = 8;

fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }

    let file = match GeoTiffFile::from_bytes(data.to_vec()) {
        Ok(file) => file,
        Err(_) => return,
    };

    let _ = file.metadata();
    let _ = file.epsg();
    let _ = file.geo_bounds();
    let _ = file.transform().map(|transform| {
        let _ = transform.pixel_to_geo(0.0, 0.0);
        let _ = transform.geo_to_pixel(0.0, 0.0);
    });
    for overview_index in 0..file.overview_count().min(MAX_OVERVIEWS) {
        let _ = file.overview_ifd_index(overview_index);
        let Ok(ifd) = file.overview_ifd(overview_index) else {
            continue;
        };
        let Ok(layout) = ifd.raster_layout() else {
            continue;
        };
        let Ok(row_bytes) = layout.checked_row_bytes() else {
            continue;
        };
        let Some(decoded_len) = row_bytes.checked_mul(layout.height) else {
            continue;
        };
        if decoded_len > MAX_DECODED_BYTES {
            continue;
        }

        let _ = file.read_overview::<u8>(overview_index);
    }

    let Ok(ifd) = file.tiff().ifd(0) else {
        return;
    };
    let Ok(layout) = ifd.raster_layout() else {
        return;
    };
    let Ok(row_bytes) = layout.checked_row_bytes() else {
        return;
    };
    let Some(decoded_len) = row_bytes.checked_mul(layout.height) else {
        return;
    };
    if decoded_len > MAX_DECODED_BYTES {
        return;
    }

    let _ = file.tiff().read_image_bytes(0);
});
