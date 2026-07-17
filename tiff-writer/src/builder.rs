//! Image builder for configuring a single TIFF IFD.

use tiff_core::*;

use crate::encoder;
use crate::sample::TiffWriteSample;

/// LERC encoding options for the TIFF writer.
///
/// Controls the LERC2 error tolerance and optional additional compression
/// applied to the encoded LERC blob before storage in the TIFF block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LercOptions {
    /// Maximum encoding error per sample value. Set to `0.0` for lossless.
    pub max_z_error: f64,
    /// Optional additional compression applied to the LERC blob.
    pub additional_compression: LercAdditionalCompression,
}

impl Default for LercOptions {
    fn default() -> Self {
        Self {
            max_z_error: 0.0,
            additional_compression: LercAdditionalCompression::None,
        }
    }
}

/// JPEG encoding options for the TIFF writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegOptions {
    /// Quality in the range 1..=100.
    pub quality: u8,
}

impl Default for JpegOptions {
    fn default() -> Self {
        Self { quality: 75 }
    }
}

/// Describes how image data is organized: strips or tiles.
#[derive(Debug, Clone, Copy)]
pub enum DataLayout {
    /// Strip-based: each strip contains `rows_per_strip` rows.
    Strips { rows_per_strip: u32 },
    /// Tile-based: each tile is `width x height` pixels.
    Tiles { width: u32, height: u32 },
}

/// Builder for configuring a single image (IFD) within a TIFF file.
#[derive(Debug, Clone)]
pub struct ImageBuilder {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) samples_per_pixel: u16,
    pub(crate) bits_per_sample: u16,
    pub(crate) sample_format: SampleFormat,
    pub(crate) compression: Compression,
    pub(crate) predictor: Predictor,
    pub(crate) photometric: PhotometricInterpretation,
    pub(crate) extra_samples: Vec<ExtraSample>,
    pub(crate) color_map: Option<ColorMap>,
    pub(crate) ink_set: Option<InkSet>,
    pub(crate) ycbcr_subsampling: Option<[u16; 2]>,
    pub(crate) ycbcr_positioning: Option<YCbCrPositioning>,
    pub(crate) planar_configuration: PlanarConfiguration,
    pub(crate) layout: DataLayout,
    pub(crate) extra_tags: Vec<Tag>,
    pub(crate) subfile_type: u32,
    pub(crate) lerc_options: Option<LercOptions>,
    pub(crate) jpeg_options: Option<JpegOptions>,
    pub(crate) deflate_level: Option<u32>,
}

impl ImageBuilder {
    /// Create a new image builder with required dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            samples_per_pixel: 1,
            bits_per_sample: 8,
            sample_format: SampleFormat::Uint,
            compression: Compression::None,
            predictor: Predictor::None,
            photometric: PhotometricInterpretation::MinIsBlack,
            extra_samples: Vec::new(),
            color_map: None,
            ink_set: None,
            ycbcr_subsampling: None,
            ycbcr_positioning: None,
            planar_configuration: PlanarConfiguration::Chunky,
            layout: DataLayout::Strips {
                rows_per_strip: height.min(256),
            },
            extra_tags: Vec::new(),
            subfile_type: 0,
            lerc_options: None,
            jpeg_options: None,
            deflate_level: None,
        }
    }

    pub fn samples_per_pixel(mut self, spp: u16) -> Self {
        self.samples_per_pixel = spp;
        self
    }

    pub fn bits_per_sample(mut self, bps: u16) -> Self {
        self.bits_per_sample = bps;
        self
    }

    pub fn sample_format(mut self, fmt: SampleFormat) -> Self {
        self.sample_format = fmt;
        self
    }

    /// Configure from a TiffWriteSample type. Sets bits_per_sample and sample_format.
    pub fn sample_type<T: TiffWriteSample>(mut self) -> Self {
        self.bits_per_sample = T::BITS_PER_SAMPLE;
        self.sample_format =
            SampleFormat::from_code(T::SAMPLE_FORMAT).unwrap_or(SampleFormat::Uint);
        self
    }

    pub fn compression(mut self, c: Compression) -> Self {
        self.compression = c;
        if !matches!(c, Compression::Lerc) {
            self.lerc_options = None;
        }
        if !matches!(c, Compression::Jpeg) {
            self.jpeg_options = None;
        }
        if matches!(c, Compression::Lerc | Compression::Jpeg) {
            self.predictor = Predictor::None;
        }
        self
    }

    pub fn predictor(mut self, p: Predictor) -> Self {
        // LERC and JPEG do not use TIFF predictors; ignore the request.
        if !matches!(self.compression, Compression::Lerc | Compression::Jpeg) {
            self.predictor = p;
        }
        self
    }

    /// Set the Deflate compression level (0-9).
    ///
    /// Applies to `Compression::Deflate`/`Compression::DeflateOld` blocks.
    /// The additional Deflate layer of `LERC+Deflate` always uses the codec
    /// default level.
    pub fn deflate_level(mut self, level: u32) -> Self {
        self.deflate_level = Some(level);
        self
    }

    pub fn photometric(mut self, p: PhotometricInterpretation) -> Self {
        self.photometric = p;
        self
    }

    /// Set TIFF ExtraSamples semantics for channels beyond the base color model.
    pub fn extra_samples(mut self, extra_samples: Vec<ExtraSample>) -> Self {
        self.extra_samples = extra_samples;
        self
    }

    /// Set a palette ColorMap for `PhotometricInterpretation::Palette`.
    pub fn color_map(mut self, color_map: ColorMap) -> Self {
        self.color_map = Some(color_map);
        self
    }

    /// Set the InkSet tag for separated photometric data.
    pub fn ink_set(mut self, ink_set: InkSet) -> Self {
        self.ink_set = Some(ink_set);
        self
    }

    /// Set TIFF YCbCr chroma subsampling factors.
    pub fn ycbcr_subsampling(mut self, subsampling: [u16; 2]) -> Self {
        self.ycbcr_subsampling = Some(subsampling);
        self
    }

    /// Set TIFF YCbCr sample positioning.
    pub fn ycbcr_positioning(mut self, positioning: YCbCrPositioning) -> Self {
        self.ycbcr_positioning = Some(positioning);
        self
    }

    /// Set chunky (interleaved) or separate planar sample layout for multi-band images.
    pub fn planar_configuration(mut self, p: PlanarConfiguration) -> Self {
        self.planar_configuration = p;
        self
    }

    /// Configure strip-based layout.
    pub fn strips(mut self, rows_per_strip: u32) -> Self {
        self.layout = DataLayout::Strips { rows_per_strip };
        self
    }

    /// Configure tile-based layout.
    pub fn tiles(mut self, tile_width: u32, tile_height: u32) -> Self {
        self.layout = DataLayout::Tiles {
            width: tile_width,
            height: tile_height,
        };
        self
    }

    /// Add an arbitrary extra tag to the IFD.
    pub fn tag(mut self, tag: Tag) -> Self {
        self.extra_tags.push(tag);
        self
    }

    /// Mark this IFD as a reduced-resolution overview.
    pub fn overview(mut self) -> Self {
        self.subfile_type = 1;
        self
    }

    /// Set LERC compression with the given options.
    ///
    /// This sets `compression = Lerc` and `predictor = None` (LERC performs
    /// its own quantization and does not use TIFF predictors).
    pub fn lerc_options(mut self, options: LercOptions) -> Self {
        self.compression = Compression::Lerc;
        self.predictor = Predictor::None;
        self.lerc_options = Some(options);
        self.jpeg_options = None;
        self
    }

    /// Set JPEG compression with the given options.
    ///
    /// This sets `compression = Jpeg` and `predictor = None` (JPEG uses its
    /// own transform and entropy coding pipeline rather than TIFF predictors).
    ///
    /// Multi-band JPEG requires `planar_configuration(Planar)` so each encoded
    /// strip/tile is a single grayscale component.
    pub fn jpeg_options(mut self, options: JpegOptions) -> Self {
        self.compression = Compression::Jpeg;
        self.predictor = Predictor::None;
        self.jpeg_options = Some(options);
        self.lerc_options = None;
        self
    }

    /// Total number of blocks (strips or tiles) for this image configuration.
    ///
    /// This legacy infallible helper is best-effort for invalid configurations.
    /// Use [`Self::checked_block_count`] when invalid layouts should be reported
    /// as errors.
    #[deprecated(
        since = "0.6.0",
        note = "use ImageBuilder::checked_block_count() to handle invalid layouts without best-effort fallback"
    )]
    pub fn block_count(&self) -> usize {
        let blocks_per_plane = self.legacy_blocks_per_plane();
        if matches!(self.planar_configuration, PlanarConfiguration::Planar) {
            blocks_per_plane.saturating_mul(self.samples_per_pixel as usize)
        } else {
            blocks_per_plane
        }
    }

    /// Checked total number of blocks (strips or tiles) for this image configuration.
    pub fn checked_block_count(&self) -> crate::error::Result<usize> {
        let blocks_per_plane = match self.checked_layout()? {
            DataLayout::Strips { rows_per_strip } => {
                let rps = rows_per_strip as usize;
                (self.height as usize).div_ceil(rps)
            }
            DataLayout::Tiles { width, height } => {
                let tw = width as usize;
                let th = height as usize;
                let tiles_across = (self.width as usize).div_ceil(tw);
                let tiles_down = (self.height as usize).div_ceil(th);
                tiles_across
                    .checked_mul(tiles_down)
                    .ok_or_else(|| layout_overflow("tile count"))?
            }
        };
        if matches!(self.planar_configuration, PlanarConfiguration::Planar) {
            blocks_per_plane
                .checked_mul(self.samples_per_pixel as usize)
                .ok_or_else(|| layout_overflow("planar block count"))
        } else {
            Ok(blocks_per_plane)
        }
    }

    /// Expected number of samples for the block at `index`.
    ///
    /// This legacy infallible helper is best-effort for invalid configurations.
    /// Use [`Self::checked_block_sample_count`] when invalid layouts should be
    /// reported as errors.
    #[deprecated(
        since = "0.6.0",
        note = "use ImageBuilder::checked_block_sample_count() to handle invalid layouts without best-effort fallback"
    )]
    pub fn block_sample_count(&self, index: usize) -> usize {
        let samples_per_pixel = self.block_samples_per_pixel() as usize;
        let plane_block_index = self.block_plane_index(index);
        match self.layout {
            DataLayout::Strips { rows_per_strip } => {
                let rps = rows_per_strip.max(1) as usize;
                let start_row = plane_block_index.saturating_mul(rps);
                let end_row = plane_block_index
                    .saturating_add(1)
                    .saturating_mul(rps)
                    .min(self.height as usize);
                let rows = end_row.saturating_sub(start_row);
                rows.saturating_mul(self.width as usize)
                    .saturating_mul(samples_per_pixel)
            }
            DataLayout::Tiles { width, height } => {
                // Tiles are always full-sized (padded at edges).
                (width.max(1) as usize)
                    .saturating_mul(height.max(1) as usize)
                    .saturating_mul(samples_per_pixel)
            }
        }
    }

    /// Checked expected number of samples for the block at `index`.
    pub fn checked_block_sample_count(&self, index: usize) -> crate::error::Result<usize> {
        let samples_per_pixel = self.block_samples_per_pixel() as usize;
        let plane_block_index = self.checked_block_plane_index(index)?;
        match self.checked_layout()? {
            DataLayout::Strips { rows_per_strip } => {
                let rps = rows_per_strip as usize;
                let start_row = plane_block_index
                    .checked_mul(rps)
                    .ok_or_else(|| layout_overflow("strip start row"))?;
                let end_row = plane_block_index
                    .checked_add(1)
                    .and_then(|value| value.checked_mul(rps))
                    .ok_or_else(|| layout_overflow("strip end row"))?
                    .min(self.height as usize);
                let rows = end_row.saturating_sub(start_row);
                rows.checked_mul(self.width as usize)
                    .and_then(|value| value.checked_mul(samples_per_pixel))
                    .ok_or_else(|| layout_overflow("strip sample count"))
            }
            DataLayout::Tiles { width, height } => {
                // Tiles are always full-sized (padded at edges)
                (width as usize)
                    .checked_mul(height as usize)
                    .and_then(|value| value.checked_mul(samples_per_pixel))
                    .ok_or_else(|| layout_overflow("tile sample count"))
            }
        }
    }

    /// Estimated uncompressed image bytes.
    ///
    /// This legacy infallible helper saturates on overflow. Use
    /// [`Self::checked_estimated_uncompressed_bytes`] when invalid layouts should
    /// be reported as errors.
    #[deprecated(
        since = "0.6.0",
        note = "use ImageBuilder::checked_estimated_uncompressed_bytes() to handle overflow without saturation"
    )]
    pub fn estimated_uncompressed_bytes(&self) -> u64 {
        let bps = (self.bits_per_sample / 8).max(1) as u64;
        (self.width as u64)
            .saturating_mul(self.height as u64)
            .saturating_mul(self.samples_per_pixel as u64)
            .saturating_mul(bps)
    }

    /// Checked estimated uncompressed image bytes.
    pub fn checked_estimated_uncompressed_bytes(&self) -> crate::error::Result<u64> {
        let bps = (self.bits_per_sample / 8).max(1) as u64;
        (self.width as u64)
            .checked_mul(self.height as u64)
            .and_then(|value| value.checked_mul(self.samples_per_pixel as u64))
            .and_then(|value| value.checked_mul(bps))
            .ok_or_else(|| layout_overflow("estimated uncompressed byte count"))
    }

    /// The TIFF tag codes for offset and bytecount arrays.
    pub fn offset_tag_codes(&self) -> (u16, u16) {
        match self.layout {
            DataLayout::Strips { .. } => (TAG_STRIP_OFFSETS, TAG_STRIP_BYTE_COUNTS),
            DataLayout::Tiles { .. } => (TAG_TILE_OFFSETS, TAG_TILE_BYTE_COUNTS),
        }
    }

    /// Build the layout-specific tags (RowsPerStrip or TileWidth/TileLength).
    ///
    /// This legacy infallible helper preserves the configured tag values even
    /// when the layout is invalid. Use [`Self::checked_layout_tags`] when invalid
    /// layouts should be reported as errors.
    #[deprecated(
        since = "0.6.0",
        note = "use ImageBuilder::checked_layout_tags() to handle invalid layouts without best-effort fallback"
    )]
    pub fn layout_tags(&self) -> Vec<Tag> {
        self.legacy_layout_tags()
    }

    /// Checked build of the layout-specific tags.
    pub fn checked_layout_tags(&self) -> crate::error::Result<Vec<Tag>> {
        match self.checked_layout()? {
            DataLayout::Strips { rows_per_strip } => Ok(vec![Tag::new(
                TAG_ROWS_PER_STRIP,
                TagValue::Long(vec![rows_per_strip]),
            )]),
            DataLayout::Tiles { width, height } => Ok(vec![
                Tag::new(TAG_TILE_WIDTH, TagValue::Long(vec![width])),
                Tag::new(TAG_TILE_LENGTH, TagValue::Long(vec![height])),
            ]),
        }
    }

    /// Build the serialized TIFF tags for this image definition.
    ///
    /// This legacy infallible helper is best-effort for invalid configurations.
    /// Use [`Self::checked_build_tags`] when invalid layouts and color models
    /// should be reported as errors.
    #[deprecated(
        since = "0.6.0",
        note = "use ImageBuilder::checked_build_tags() to handle invalid layouts and color models without best-effort fallback"
    )]
    pub fn build_tags(&self, is_bigtiff: bool) -> Vec<Tag> {
        let mut extra_tags = self.extra_tags.clone();
        if let Some(lerc_tag) = self.lerc_parameters_tag() {
            extra_tags.push(lerc_tag);
        }
        if let Ok(extra_samples) = self.effective_extra_samples() {
            if !extra_samples.is_empty() {
                extra_tags.push(Tag::new(
                    TAG_EXTRA_SAMPLES,
                    TagValue::Short(
                        extra_samples
                            .iter()
                            .copied()
                            .map(ExtraSample::to_code)
                            .collect(),
                    ),
                ));
            }
        }
        if let Some(color_map) = &self.color_map {
            extra_tags.push(Tag::new(
                TAG_COLOR_MAP,
                TagValue::Short(color_map.encode_tag_values()),
            ));
        }
        if let Some(ink_set) = self.ink_set {
            extra_tags.push(Tag::new(
                TAG_INK_SET,
                TagValue::Short(vec![ink_set.to_code()]),
            ));
        }
        if let Some([h, v]) = self.ycbcr_subsampling {
            extra_tags.push(Tag::new(TAG_YCBCR_SUBSAMPLING, TagValue::Short(vec![h, v])));
        }
        if let Some(positioning) = self.ycbcr_positioning {
            extra_tags.push(Tag::new(
                TAG_YCBCR_POSITIONING,
                TagValue::Short(vec![positioning.to_code()]),
            ));
        }

        let (offsets_tag_code, byte_counts_tag_code) = self.offset_tag_codes();
        let layout_tags = self.legacy_layout_tags();
        let num_blocks = self.checked_block_count().unwrap_or(0);

        encoder::build_image_tags(&encoder::ImageTagParams {
            width: self.width,
            height: self.height,
            samples_per_pixel: self.samples_per_pixel,
            bits_per_sample: self.bits_per_sample,
            sample_format: self.sample_format.to_code(),
            compression: self.compression.to_code(),
            photometric: self.photometric.to_code(),
            predictor: self.predictor.to_code(),
            planar_configuration: self.planar_configuration.to_code(),
            subfile_type: self.subfile_type,
            extra_tags: &extra_tags,
            offsets_tag_code,
            byte_counts_tag_code,
            num_blocks,
            layout_tags: &layout_tags,
            is_bigtiff,
        })
    }

    /// Checked build of the serialized TIFF tags for this image definition.
    pub fn checked_build_tags(&self, is_bigtiff: bool) -> crate::error::Result<Vec<Tag>> {
        let mut extra_tags = self.extra_tags.clone();
        if let Some(lerc_tag) = self.lerc_parameters_tag() {
            extra_tags.push(lerc_tag);
        }
        self.validate()?;
        let extra_samples = self.effective_extra_samples()?;
        if !extra_samples.is_empty() {
            extra_tags.push(Tag::new(
                TAG_EXTRA_SAMPLES,
                TagValue::Short(
                    extra_samples
                        .iter()
                        .copied()
                        .map(ExtraSample::to_code)
                        .collect(),
                ),
            ));
        }
        if let Some(color_map) = &self.color_map {
            extra_tags.push(Tag::new(
                TAG_COLOR_MAP,
                TagValue::Short(color_map.encode_tag_values()),
            ));
        }
        if let Some(ink_set) = self.ink_set {
            extra_tags.push(Tag::new(
                TAG_INK_SET,
                TagValue::Short(vec![ink_set.to_code()]),
            ));
        }
        if let Some([h, v]) = self.ycbcr_subsampling {
            extra_tags.push(Tag::new(TAG_YCBCR_SUBSAMPLING, TagValue::Short(vec![h, v])));
        }
        if let Some(positioning) = self.ycbcr_positioning {
            extra_tags.push(Tag::new(
                TAG_YCBCR_POSITIONING,
                TagValue::Short(vec![positioning.to_code()]),
            ));
        }

        let (offsets_tag_code, byte_counts_tag_code) = self.offset_tag_codes();
        let layout_tags = self.checked_layout_tags()?;

        Ok(encoder::build_image_tags(&encoder::ImageTagParams {
            width: self.width,
            height: self.height,
            samples_per_pixel: self.samples_per_pixel,
            bits_per_sample: self.bits_per_sample,
            sample_format: self.sample_format.to_code(),
            compression: self.compression.to_code(),
            photometric: self.photometric.to_code(),
            predictor: self.predictor.to_code(),
            planar_configuration: self.planar_configuration.to_code(),
            subfile_type: self.subfile_type,
            extra_tags: &extra_tags,
            offsets_tag_code,
            byte_counts_tag_code,
            num_blocks: self.checked_block_count()?,
            layout_tags: &layout_tags,
            is_bigtiff,
        }))
    }

    /// Row width in pixels for compression pipeline (tile_width or image_width).
    pub fn block_row_width(&self) -> usize {
        match self.layout {
            DataLayout::Strips { .. } => self.width as usize,
            DataLayout::Tiles { width, .. } => width as usize,
        }
    }

    /// Samples per pixel represented in a single block.
    pub fn block_samples_per_pixel(&self) -> u16 {
        if matches!(self.planar_configuration, PlanarConfiguration::Planar) {
            1
        } else {
            self.samples_per_pixel
        }
    }

    fn block_plane_index(&self, index: usize) -> usize {
        if matches!(self.planar_configuration, PlanarConfiguration::Planar) {
            let blocks_per_plane = self.legacy_blocks_per_plane();
            if blocks_per_plane == 0 {
                0
            } else {
                index % blocks_per_plane
            }
        } else {
            index
        }
    }

    fn checked_block_plane_index(&self, index: usize) -> crate::error::Result<usize> {
        if matches!(self.planar_configuration, PlanarConfiguration::Planar) {
            let blocks_per_plane = self.checked_blocks_per_plane()?;
            if blocks_per_plane == 0 {
                return Err(crate::error::Error::InvalidConfig(
                    "block count must be greater than zero".into(),
                ));
            }
            Ok(index % blocks_per_plane)
        } else {
            Ok(index)
        }
    }

    fn checked_blocks_per_plane(&self) -> crate::error::Result<usize> {
        match self.checked_layout()? {
            DataLayout::Strips { rows_per_strip } => {
                let rps = rows_per_strip as usize;
                Ok((self.height as usize).div_ceil(rps))
            }
            DataLayout::Tiles { width, height } => {
                let tw = width as usize;
                let th = height as usize;
                let tiles_across = (self.width as usize).div_ceil(tw);
                let tiles_down = (self.height as usize).div_ceil(th);
                tiles_across
                    .checked_mul(tiles_down)
                    .ok_or_else(|| layout_overflow("tile count"))
            }
        }
    }

    fn legacy_blocks_per_plane(&self) -> usize {
        match self.layout {
            DataLayout::Strips { rows_per_strip } => {
                let rps = rows_per_strip.max(1) as usize;
                (self.height as usize).div_ceil(rps)
            }
            DataLayout::Tiles { width, height } => {
                let tw = width.max(1) as usize;
                let th = height.max(1) as usize;
                let tiles_across = (self.width as usize).div_ceil(tw);
                let tiles_down = (self.height as usize).div_ceil(th);
                tiles_across.saturating_mul(tiles_down)
            }
        }
    }

    fn legacy_layout_tags(&self) -> Vec<Tag> {
        match self.layout {
            DataLayout::Strips { rows_per_strip } => {
                vec![Tag::new(
                    TAG_ROWS_PER_STRIP,
                    TagValue::Long(vec![rows_per_strip]),
                )]
            }
            DataLayout::Tiles { width, height } => {
                vec![
                    Tag::new(TAG_TILE_WIDTH, TagValue::Long(vec![width])),
                    Tag::new(TAG_TILE_LENGTH, TagValue::Long(vec![height])),
                ]
            }
        }
    }

    /// Height of the block at `index` in pixels.
    ///
    /// Tiles are always full-sized (padded at edges). Strips may be shorter
    /// for the final strip.
    pub fn block_height(&self, index: usize) -> u32 {
        match self.layout {
            DataLayout::Tiles { height, .. } => height,
            DataLayout::Strips { rows_per_strip } => {
                let plane_index = self.block_plane_index(index);
                let rps = rows_per_strip.max(1) as usize;
                let start_row = plane_index.saturating_mul(rps);
                let remaining = (self.height as usize).saturating_sub(start_row);
                remaining.min(rps) as u32
            }
        }
    }

    /// Build the `TAG_LERC_PARAMETERS` tag if LERC compression is configured.
    pub fn lerc_parameters_tag(&self) -> Option<Tag> {
        if !matches!(self.compression, Compression::Lerc) {
            return None;
        }
        let opts = self.lerc_options.unwrap_or_default();
        Some(Tag::new(
            TAG_LERC_PARAMETERS,
            TagValue::Long(vec![
                LERC_VERSION_2_4,
                opts.additional_compression.to_code(),
            ]),
        ))
    }

    /// Validate the configuration.
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(crate::error::Error::InvalidConfig(
                "image dimensions must be positive".into(),
            ));
        }
        if self.samples_per_pixel == 0 {
            return Err(crate::error::Error::InvalidConfig(
                "samples_per_pixel must be greater than zero".into(),
            ));
        }
        if !matches!(self.bits_per_sample, 8 | 16 | 32 | 64) {
            return Err(crate::error::Error::InvalidConfig(format!(
                "bits_per_sample must be 8, 16, 32, or 64, got {}",
                self.bits_per_sample
            )));
        }
        match self.layout {
            DataLayout::Strips { rows_per_strip: 0 } => {
                return Err(crate::error::Error::InvalidConfig(
                    "rows_per_strip must be greater than zero".into(),
                ));
            }
            DataLayout::Tiles { width, height } => {
                if width == 0 || height == 0 {
                    return Err(crate::error::Error::InvalidConfig(format!(
                        "tile_width and tile_height must be greater than zero, got {}x{}",
                        width, height
                    )));
                }
                if width % 16 != 0 || height % 16 != 0 {
                    return Err(crate::error::Error::InvalidConfig(format!(
                        "tile dimensions must be multiples of 16, got {}x{}",
                        width, height
                    )));
                }
            }
            _ => {}
        }
        self.checked_block_count()?;
        self.checked_block_sample_count(0)?;
        self.checked_estimated_uncompressed_bytes()?;
        if matches!(self.compression, Compression::Lerc)
            && !matches!(self.predictor, Predictor::None)
        {
            return Err(crate::error::Error::InvalidConfig(
                "LERC compression does not support TIFF predictors".into(),
            ));
        }
        if matches!(self.sample_format, SampleFormat::Float) && self.bits_per_sample < 32 {
            return Err(crate::error::Error::InvalidConfig(format!(
                "float sample format requires 32 or 64 bits per sample, got {}",
                self.bits_per_sample
            )));
        }
        match self.predictor {
            Predictor::Horizontal => {
                if matches!(self.sample_format, SampleFormat::Float) {
                    return Err(crate::error::Error::InvalidConfig(
                        "horizontal predictor requires integer sample formats; \
                         use Predictor::FloatingPoint for float samples"
                            .into(),
                    ));
                }
            }
            Predictor::FloatingPoint => {
                if !matches!(self.sample_format, SampleFormat::Float) {
                    return Err(crate::error::Error::InvalidConfig(
                        "floating-point predictor requires float sample formats".into(),
                    ));
                }
            }
            Predictor::None => {}
        }
        if matches!(self.compression, Compression::OldJpeg) {
            return Err(crate::error::Error::InvalidConfig(
                "Old-style JPEG compression is not supported for writing; use Compression::Jpeg"
                    .into(),
            ));
        }
        if let Some(level) = self.deflate_level {
            if level > 9 {
                return Err(crate::error::Error::InvalidConfig(format!(
                    "deflate_level must be 0-9, got {level}"
                )));
            }
            if !matches!(
                self.compression,
                Compression::Deflate | Compression::DeflateOld
            ) {
                return Err(crate::error::Error::InvalidConfig(
                    "deflate_level requires Deflate compression".into(),
                ));
            }
        }
        self.validate_color_model()?;
        if matches!(self.compression, Compression::Jpeg) {
            self.validate_jpeg_config()?;
        }
        Ok(())
    }

    fn checked_layout(&self) -> crate::error::Result<DataLayout> {
        match self.layout {
            DataLayout::Strips { rows_per_strip: 0 } => Err(crate::error::Error::InvalidConfig(
                "rows_per_strip must be greater than zero".into(),
            )),
            DataLayout::Tiles { width, height } if width == 0 || height == 0 => {
                Err(crate::error::Error::InvalidConfig(format!(
                    "tile_width and tile_height must be greater than zero, got {}x{}",
                    width, height
                )))
            }
            DataLayout::Tiles { width, height } if width % 16 != 0 || height % 16 != 0 => {
                Err(crate::error::Error::InvalidConfig(format!(
                    "tile dimensions must be multiples of 16, got {}x{}",
                    width, height
                )))
            }
            layout => Ok(layout),
        }
    }

    fn validate_color_model(&self) -> crate::error::Result<()> {
        if !matches!(self.photometric, PhotometricInterpretation::Palette)
            && self.color_map.is_some()
        {
            return Err(crate::error::Error::InvalidConfig(
                "ColorMap is only valid with palette photometric interpretation".into(),
            ));
        }

        if !matches!(self.photometric, PhotometricInterpretation::Separated)
            && self.ink_set.is_some()
        {
            return Err(crate::error::Error::InvalidConfig(
                "InkSet is only valid with separated photometric interpretation".into(),
            ));
        }

        let base_samples: u16 = match self.photometric {
            PhotometricInterpretation::MinIsWhite | PhotometricInterpretation::MinIsBlack => 1,
            PhotometricInterpretation::Rgb => 3,
            PhotometricInterpretation::Palette => {
                let color_map =
                    self.color_map
                        .as_ref()
                        .ok_or(crate::error::Error::InvalidConfig(
                            "palette photometric interpretation requires a ColorMap".into(),
                        ))?;
                let expected_entries =
                    1usize
                        .checked_shl(self.bits_per_sample as u32)
                        .ok_or_else(|| {
                            crate::error::Error::InvalidConfig(format!(
                                "palette BitsPerSample {} exceeds usize shift width",
                                self.bits_per_sample
                            ))
                        })?;
                if color_map.len() != expected_entries {
                    return Err(crate::error::Error::InvalidConfig(format!(
                        "palette ColorMap has {} entries but BitsPerSample={} requires {}",
                        color_map.len(),
                        self.bits_per_sample,
                        expected_entries
                    )));
                }
                1
            }
            PhotometricInterpretation::Mask => 1,
            PhotometricInterpretation::Separated => match self.ink_set.unwrap_or(InkSet::Cmyk) {
                InkSet::Cmyk => 4,
                InkSet::NotCmyk | InkSet::Unknown(_) => {
                    return Err(crate::error::Error::InvalidConfig(
                        "separated photometric interpretation currently requires InkSet::Cmyk"
                            .into(),
                    ))
                }
            },
            PhotometricInterpretation::YCbCr => 3,
            PhotometricInterpretation::CieLab => 3,
        };

        let _ = self.effective_extra_samples_for_base(base_samples)?;

        if matches!(self.photometric, PhotometricInterpretation::YCbCr) {
            if !matches!(self.sample_format, SampleFormat::Uint) || self.bits_per_sample != 8 {
                return Err(crate::error::Error::InvalidConfig(
                    "YCbCr photometric interpretation requires 8-bit unsigned samples".into(),
                ));
            }
            if let Some(subsampling) = self.ycbcr_subsampling {
                if subsampling != [1, 1] {
                    return Err(crate::error::Error::InvalidConfig(format!(
                        "YCbCr subsampling {:?} is not supported by the current writer",
                        subsampling
                    )));
                }
            }
        } else if self.ycbcr_subsampling.is_some() || self.ycbcr_positioning.is_some() {
            return Err(crate::error::Error::InvalidConfig(
                "YCbCr-specific tags require YCbCr photometric interpretation".into(),
            ));
        }

        Ok(())
    }

    fn effective_extra_samples(&self) -> crate::error::Result<Vec<ExtraSample>> {
        let base_samples = match self.photometric {
            PhotometricInterpretation::MinIsWhite | PhotometricInterpretation::MinIsBlack => 1,
            PhotometricInterpretation::Rgb => 3,
            PhotometricInterpretation::Palette => 1,
            PhotometricInterpretation::Mask => 1,
            PhotometricInterpretation::Separated => 4,
            PhotometricInterpretation::YCbCr => 3,
            PhotometricInterpretation::CieLab => 3,
        };
        self.effective_extra_samples_for_base(base_samples)
    }

    fn effective_extra_samples_for_base(
        &self,
        base_samples: u16,
    ) -> crate::error::Result<Vec<ExtraSample>> {
        let implied_extra_samples = self
            .samples_per_pixel
            .checked_sub(base_samples)
            .ok_or_else(|| {
                crate::error::Error::InvalidConfig(format!(
                    "{} photometric interpretation requires at least {} samples, got {}",
                    photometric_name(self.photometric),
                    base_samples,
                    self.samples_per_pixel
                ))
            })?;
        if self.extra_samples.len() > implied_extra_samples as usize {
            return Err(crate::error::Error::InvalidConfig(format!(
                "{} photometric interpretation has {} total channels but {} ExtraSamples",
                photometric_name(self.photometric),
                self.samples_per_pixel,
                self.extra_samples.len()
            )));
        }

        let mut extra_samples = self.extra_samples.clone();
        extra_samples.resize(implied_extra_samples as usize, ExtraSample::Unspecified);
        Ok(extra_samples)
    }

    fn validate_jpeg_config(&self) -> crate::error::Result<()> {
        let options = self.jpeg_options.unwrap_or_default();
        if !(1..=100).contains(&options.quality) {
            return Err(crate::error::Error::InvalidConfig(format!(
                "JPEG quality must be in the range 1..=100, got {}",
                options.quality
            )));
        }
        if self.bits_per_sample != 8 {
            return Err(crate::error::Error::InvalidConfig(format!(
                "JPEG compression requires 8-bit samples, got {} bits",
                self.bits_per_sample
            )));
        }
        if !matches!(self.sample_format, SampleFormat::Uint) {
            return Err(crate::error::Error::InvalidConfig(format!(
                "JPEG compression requires unsigned integer samples, got {:?}",
                self.sample_format
            )));
        }
        if !matches!(self.predictor, Predictor::None) {
            return Err(crate::error::Error::InvalidConfig(
                "JPEG compression does not support TIFF predictors".into(),
            ));
        }

        let block_width = self.block_row_width();
        if block_width > u16::MAX as usize {
            return Err(crate::error::Error::InvalidConfig(format!(
                "JPEG block width must be <= {}, got {}",
                u16::MAX,
                block_width
            )));
        }
        let max_block_height = match self.layout {
            DataLayout::Strips { rows_per_strip } => rows_per_strip.max(1),
            DataLayout::Tiles { height, .. } => height,
        };
        if max_block_height > u16::MAX as u32 {
            return Err(crate::error::Error::InvalidConfig(format!(
                "JPEG block height must be <= {}, got {}",
                u16::MAX,
                max_block_height
            )));
        }

        let block_samples_per_pixel = self.block_samples_per_pixel();
        if block_samples_per_pixel != 1 {
            return Err(crate::error::Error::InvalidConfig(format!(
                "JPEG write currently supports one sample per encoded block, got {}; use planar configuration for multi-band JPEG",
                block_samples_per_pixel
            )));
        }

        if matches!(
            self.photometric,
            PhotometricInterpretation::Palette | PhotometricInterpretation::Mask
        ) {
            return Err(crate::error::Error::InvalidConfig(format!(
                "{:?} photometric interpretation is not supported with JPEG compression",
                self.photometric
            )));
        }

        Ok(())
    }
}

fn photometric_name(photometric: PhotometricInterpretation) -> &'static str {
    match photometric {
        PhotometricInterpretation::MinIsWhite => "MinIsWhite",
        PhotometricInterpretation::MinIsBlack => "MinIsBlack",
        PhotometricInterpretation::Rgb => "RGB",
        PhotometricInterpretation::Palette => "Palette",
        PhotometricInterpretation::Mask => "TransparencyMask",
        PhotometricInterpretation::Separated => "Separated",
        PhotometricInterpretation::YCbCr => "YCbCr",
        PhotometricInterpretation::CieLab => "CIELab",
    }
}

fn layout_overflow(context: &'static str) -> crate::error::Error {
    crate::error::Error::InvalidConfig(format!("{context} overflows layout size limits"))
}

#[cfg(test)]
mod tests {
    use super::ImageBuilder;
    use std::panic;
    use tiff_core::{PhotometricInterpretation, PlanarConfiguration};

    #[test]
    fn validate_rejects_zero_strip_and_tile_dimensions() {
        let err = ImageBuilder::new(16, 16).strips(0).validate().unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("rows_per_strip"))
        );

        let err = ImageBuilder::new(16, 16)
            .tiles(0, 16)
            .validate()
            .unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("tile_width"))
        );

        let err = ImageBuilder::new(16, 16)
            .tiles(16, 0)
            .validate()
            .unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("tile_height"))
        );

        let err = ImageBuilder::new(16, 16)
            .tiles(0, 0)
            .validate()
            .unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("tile_width") && message.contains("tile_height"))
        );
    }

    #[test]
    fn checked_helpers_reject_zero_strip_and_tile_dimensions() {
        let builder = ImageBuilder::new(16, 16).strips(0);
        let err = builder.checked_block_count().unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("rows_per_strip"))
        );
        let err = builder.checked_layout_tags().unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("rows_per_strip"))
        );
        let err = builder.checked_build_tags(false).unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("rows_per_strip"))
        );

        let builder = ImageBuilder::new(16, 16).tiles(0, 16);
        let err = builder.checked_block_count().unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("tile_width"))
        );
        let err = builder.checked_layout_tags().unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("tile_width"))
        );
        let err = builder.checked_build_tags(false).unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("tile_width"))
        );

        let builder = ImageBuilder::new(16, 16).tiles(16, 0);
        let err = builder.checked_block_count().unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("tile_height"))
        );
        let err = builder.checked_layout_tags().unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("tile_height"))
        );
        let err = builder.checked_build_tags(false).unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("tile_height"))
        );

        let builder = ImageBuilder::new(16, 16).tiles(15, 16);
        let err = builder.checked_layout_tags().unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("multiples of 16"))
        );
    }

    #[test]
    fn checked_build_tags_returns_color_model_errors() {
        let err = ImageBuilder::new(16, 16)
            .photometric(PhotometricInterpretation::Rgb)
            .samples_per_pixel(1)
            .checked_build_tags(false)
            .unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("requires at least 3 samples"))
        );
    }

    #[test]
    #[allow(deprecated)]
    fn legacy_infallible_helpers_do_not_panic_on_invalid_builders() {
        let builders = vec![
            ImageBuilder::new(16, 16).strips(0),
            ImageBuilder::new(16, 16).tiles(0, 16),
            ImageBuilder::new(16, 16).tiles(16, 0),
            ImageBuilder::new(16, 16).tiles(15, 16),
            ImageBuilder::new(16, 16)
                .photometric(PhotometricInterpretation::Rgb)
                .samples_per_pixel(1),
            ImageBuilder::new(u32::MAX, u32::MAX)
                .sample_type::<u8>()
                .samples_per_pixel(u16::MAX)
                .planar_configuration(PlanarConfiguration::Planar)
                .tiles(16, 16),
        ];

        for builder in builders {
            let result = panic::catch_unwind(|| {
                let _ = builder.block_count();
                let _ = builder.block_sample_count(usize::MAX);
                let _ = builder.estimated_uncompressed_bytes();
                let _ = builder.layout_tags();
                let _ = builder.build_tags(false);
                let _ = builder.build_tags(true);
                let _ = builder.block_height(usize::MAX);
            });
            assert!(result.is_ok());
        }
    }

    #[test]
    fn validate_rejects_mismatched_predictor_and_sample_format() {
        let err = ImageBuilder::new(4, 4)
            .sample_type::<f32>()
            .compression(tiff_core::Compression::Deflate)
            .predictor(tiff_core::Predictor::Horizontal)
            .validate()
            .unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("horizontal predictor"))
        );

        let err = ImageBuilder::new(4, 4)
            .sample_type::<u16>()
            .compression(tiff_core::Compression::Deflate)
            .predictor(tiff_core::Predictor::FloatingPoint)
            .validate()
            .unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("floating-point predictor"))
        );

        let err = ImageBuilder::new(4, 4)
            .bits_per_sample(16)
            .sample_format(tiff_core::SampleFormat::Float)
            .validate()
            .unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("32 or 64 bits"))
        );

        assert!(ImageBuilder::new(4, 4)
            .sample_type::<u16>()
            .compression(tiff_core::Compression::Deflate)
            .predictor(tiff_core::Predictor::Horizontal)
            .validate()
            .is_ok());
        assert!(ImageBuilder::new(4, 4)
            .sample_type::<f32>()
            .compression(tiff_core::Compression::Deflate)
            .predictor(tiff_core::Predictor::FloatingPoint)
            .validate()
            .is_ok());
    }

    #[test]
    fn validate_rejects_overflowing_layout_sizes() {
        let err = ImageBuilder::new(u32::MAX, u32::MAX)
            .sample_type::<u8>()
            .samples_per_pixel(u16::MAX)
            .planar_configuration(PlanarConfiguration::Planar)
            .tiles(16, 16)
            .validate()
            .unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("block count"))
        );

        let large_multiple_of_16 = u32::MAX - 15;
        let err = ImageBuilder::new(1, 1)
            .sample_type::<u8>()
            .samples_per_pixel(2)
            .tiles(large_multiple_of_16, large_multiple_of_16)
            .validate()
            .unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("sample count"))
        );

        let err = ImageBuilder::new(u32::MAX, u32::MAX)
            .sample_type::<u64>()
            .samples_per_pixel(2)
            .strips(256)
            .validate()
            .unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(message) if message.contains("byte count"))
        );
    }
}
