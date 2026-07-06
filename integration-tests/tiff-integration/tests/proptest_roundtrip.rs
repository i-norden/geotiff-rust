//! Property-based writer -> reader roundtrips.
//!
//! Every generated configuration writes deterministic per-block sample data
//! through `TiffWriter` and checks that `TiffFile` recovers each pixel
//! bit-exactly, using an independent reimplementation of the block layout
//! spec to compute expected values.

use std::io::Cursor;

use proptest::prelude::*;
use tiff_core::{ByteOrder, Compression, PlanarConfiguration, Predictor};
use tiff_reader::{TiffFile, TiffSample};
use tiff_writer::{ImageBuilder, TiffVariant, TiffWriter, WriteOptions};

#[derive(Debug, Clone, Copy, PartialEq)]
enum SampleType {
    U8,
    U16,
    I16,
    U32,
    F32,
    F64,
}

#[derive(Debug, Clone, Copy)]
enum Layout {
    Strips { rows_per_strip: u32 },
    Tiles { width: u32, height: u32 },
}

#[derive(Debug, Clone, Copy)]
struct Config {
    width: u32,
    height: u32,
    bands: u16,
    sample_type: SampleType,
    compression: Compression,
    predictor: Predictor,
    planar: PlanarConfiguration,
    layout: Layout,
    byte_order: ByteOrder,
}

fn config_strategy() -> impl Strategy<Value = Config> {
    let sample_type = prop_oneof![
        Just(SampleType::U8),
        Just(SampleType::U16),
        Just(SampleType::I16),
        Just(SampleType::U32),
        Just(SampleType::F32),
        Just(SampleType::F64),
    ];
    let compression = prop_oneof![
        Just(Compression::None),
        Just(Compression::Lzw),
        Just(Compression::Deflate),
        Just(Compression::Zstd),
        Just(Compression::Lerc),
    ];
    let planar = prop_oneof![
        Just(PlanarConfiguration::Chunky),
        Just(PlanarConfiguration::Planar),
    ];
    let byte_order = prop_oneof![Just(ByteOrder::LittleEndian), Just(ByteOrder::BigEndian)];

    (
        1u32..40,
        1u32..40,
        1u16..=3,
        sample_type,
        compression,
        0u8..3, // predictor choice, resolved per sample type below
        planar,
        prop_oneof![Just(0u8), Just(1u8)], // strips vs tiles
        1u32..=8,                          // rows per strip (clamped to height)
        prop_oneof![Just(16u32), Just(32u32)],
        byte_order,
    )
        .prop_map(
            |(
                width,
                height,
                bands,
                sample_type,
                compression,
                predictor_choice,
                planar,
                layout_choice,
                rows_per_strip,
                tile_size,
                byte_order,
            )| {
                let is_float = matches!(sample_type, SampleType::F32 | SampleType::F64);
                // LERC ignores predictors; otherwise pick one valid for the type.
                let predictor = if matches!(compression, Compression::Lerc) {
                    Predictor::None
                } else {
                    match predictor_choice {
                        0 => Predictor::None,
                        _ if is_float => Predictor::FloatingPoint,
                        _ => Predictor::Horizontal,
                    }
                };
                let layout = if layout_choice == 0 {
                    Layout::Strips {
                        rows_per_strip: rows_per_strip.min(height),
                    }
                } else {
                    Layout::Tiles {
                        width: tile_size,
                        height: tile_size,
                    }
                };
                Config {
                    width,
                    height,
                    bands,
                    sample_type,
                    compression,
                    predictor,
                    planar,
                    layout,
                    byte_order,
                }
            },
        )
}

/// Deterministic sample value for a (block, offset) position.
fn sample_seed(block_index: usize, offset: usize) -> u64 {
    (block_index as u64)
        .wrapping_mul(1_000_003)
        .wrapping_add((offset as u64).wrapping_mul(7919))
}

trait RoundtripSample: TiffSample + tiff_writer::TiffWriteSample + PartialEq + std::fmt::Debug {
    fn from_seed(seed: u64) -> Self;
}

impl RoundtripSample for u8 {
    fn from_seed(seed: u64) -> Self {
        (seed % 251) as u8
    }
}
impl RoundtripSample for u16 {
    fn from_seed(seed: u64) -> Self {
        (seed % 61_001) as u16
    }
}
impl RoundtripSample for i16 {
    fn from_seed(seed: u64) -> Self {
        ((seed % 61_001) as i64 - 30_500) as i16
    }
}
impl RoundtripSample for u32 {
    fn from_seed(seed: u64) -> Self {
        (seed % 4_000_000_007) as u32
    }
}
impl RoundtripSample for f32 {
    fn from_seed(seed: u64) -> Self {
        ((seed % 100_000) as f32) * 0.25 - 12_500.0
    }
}
impl RoundtripSample for f64 {
    fn from_seed(seed: u64) -> Self {
        ((seed % 10_000_000) as f64) * 0.125 - 625_000.0
    }
}

/// Independent block-layout mapping: pixel (row, col, band) -> (block, offset).
fn expected_position(config: &Config, row: usize, col: usize, band: usize) -> (usize, usize) {
    let width = config.width as usize;
    let height = config.height as usize;
    let bands = config.bands as usize;
    let planar = matches!(config.planar, PlanarConfiguration::Planar);
    let block_bands = if planar { 1 } else { bands };

    match config.layout {
        Layout::Strips { rows_per_strip } => {
            let rps = rows_per_strip as usize;
            let strips_per_plane = height.div_ceil(rps);
            let strip = row / rps;
            let block = if planar {
                band * strips_per_plane + strip
            } else {
                strip
            };
            let offset = ((row % rps) * width + col) * block_bands + if planar { 0 } else { band };
            (block, offset)
        }
        Layout::Tiles {
            width: tw,
            height: th,
        } => {
            let tw = tw as usize;
            let th = th as usize;
            let tiles_across = width.div_ceil(tw);
            let tiles_down = height.div_ceil(th);
            let tile = (row / th) * tiles_across + col / tw;
            let block = if planar {
                band * (tiles_across * tiles_down) + tile
            } else {
                tile
            };
            let offset = ((row % th) * tw + col % tw) * block_bands + if planar { 0 } else { band };
            (block, offset)
        }
    }
}

fn roundtrip<T: RoundtripSample>(config: &Config) {
    let mut ib = ImageBuilder::new(config.width, config.height)
        .sample_type::<T>()
        .samples_per_pixel(config.bands)
        .photometric(if config.bands >= 3 {
            tiff_core::PhotometricInterpretation::Rgb
        } else {
            tiff_core::PhotometricInterpretation::MinIsBlack
        })
        .compression(config.compression)
        .predictor(config.predictor)
        .planar_configuration(config.planar);
    ib = match config.layout {
        Layout::Strips { rows_per_strip } => ib.strips(rows_per_strip),
        Layout::Tiles { width, height } => ib.tiles(width, height),
    };
    if config.bands == 2 {
        ib = ib.extra_samples(vec![tiff_core::ExtraSample::Unspecified]);
    }

    let mut writer = TiffWriter::new(
        Cursor::new(Vec::new()),
        WriteOptions {
            byte_order: config.byte_order,
            variant: TiffVariant::Auto,
        },
    )
    .unwrap();
    let block_count = ib.checked_block_count().unwrap();
    let handle = writer.add_image(ib).unwrap();
    for block in 0..block_count {
        let len = writer_block_len(config, block);
        let samples: Vec<T> = (0..len)
            .map(|offset| T::from_seed(sample_seed(block, offset)))
            .collect();
        writer.write_block(&handle, block, &samples).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();

    let file = TiffFile::from_bytes(bytes).unwrap();
    let image = file.read_image::<T>(0).unwrap();
    let expected_shape: &[usize] = if config.bands == 1 {
        &[config.height as usize, config.width as usize]
    } else {
        &[
            config.height as usize,
            config.width as usize,
            config.bands as usize,
        ]
    };
    assert_eq!(image.shape(), expected_shape, "{config:?}");

    for row in 0..config.height as usize {
        for col in 0..config.width as usize {
            for band in 0..config.bands as usize {
                let (block, offset) = expected_position(config, row, col, band);
                let expected = T::from_seed(sample_seed(block, offset));
                let actual = if config.bands == 1 {
                    image[[row, col]]
                } else {
                    image[[row, col, band]]
                };
                assert_eq!(actual, expected, "{config:?} at ({row},{col},{band})");
            }
        }
    }

    // Window read through the same layout must agree with the full image.
    let row_off = (config.height as usize) / 3;
    let col_off = (config.width as usize) / 3;
    let rows = (config.height as usize).div_ceil(2) - row_off / 2;
    let cols = (config.width as usize).div_ceil(2) - col_off / 2;
    let window = file
        .read_window::<T>(0, row_off, col_off, rows, cols)
        .unwrap();
    for row in 0..rows {
        for col in 0..cols {
            for band in 0..config.bands as usize {
                let expected = if config.bands == 1 {
                    image[[row_off + row, col_off + col]]
                } else {
                    image[[row_off + row, col_off + col, band]]
                };
                let actual = if config.bands == 1 {
                    window[[row, col]]
                } else {
                    window[[row, col, band]]
                };
                assert_eq!(actual, expected, "{config:?} window ({row},{col},{band})");
            }
        }
    }
}

fn writer_block_len(config: &Config, block: usize) -> usize {
    let width = config.width as usize;
    let height = config.height as usize;
    let block_bands = if matches!(config.planar, PlanarConfiguration::Planar) {
        1
    } else {
        config.bands as usize
    };
    match config.layout {
        Layout::Strips { rows_per_strip } => {
            let rps = rows_per_strip as usize;
            let strips_per_plane = height.div_ceil(rps);
            let strip = block % strips_per_plane;
            let rows = rps.min(height - strip * rps);
            rows * width * block_bands
        }
        Layout::Tiles {
            width: tw,
            height: th,
        } => (tw as usize) * (th as usize) * block_bands,
    }
}

fn run_roundtrip(config: Config) {
    match config.sample_type {
        SampleType::U8 => roundtrip::<u8>(&config),
        SampleType::U16 => roundtrip::<u16>(&config),
        SampleType::I16 => roundtrip::<i16>(&config),
        SampleType::U32 => roundtrip::<u32>(&config),
        SampleType::F32 => roundtrip::<f32>(&config),
        SampleType::F64 => roundtrip::<f64>(&config),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        ..ProptestConfig::default()
    })]

    #[test]
    fn writer_reader_roundtrip_is_bit_exact(config in config_strategy()) {
        run_roundtrip(config);
    }
}
