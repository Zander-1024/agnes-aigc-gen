use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl Dimensions {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn size_string(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AspectRatio {
    pub w: u32,
    pub h: u32,
}

impl AspectRatio {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let (w, h) = s
            .split_once(':')
            .with_context(|| format!("invalid ratio {s:?}, expected w:h"))?;
        let w: u32 = w.trim().parse().context("ratio width")?;
        let h: u32 = h.trim().parse().context("ratio height")?;
        if w == 0 || h == 0 {
            bail!("ratio components must be non-zero");
        }
        Ok(Self { w, h })
    }

    pub fn label(&self) -> String {
        format!("{}:{}", self.w, self.h)
    }
}

/// Supported image aspect ratios (wire values for API `ratio` field).
static IMAGE_SUPPORTED_RATIOS: &[&str] = &["1:1", "3:4", "4:3", "16:9", "9:16", "2:3", "3:2", "21:9"];

/// Image resolution tiers (wire values for API `size` field).
static IMAGE_RESOLUTION_TIERS: &[&str] = &["1K", "2K", "3K", "4K"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageResolutionTier {
    #[default]
    K1,
    K2,
    K3,
    K4,
}

impl ImageResolutionTier {
    pub const DEFAULT: Self = Self::K1;

    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "1k" => Ok(Self::K1),
            "2k" => Ok(Self::K2),
            "3k" => Ok(Self::K3),
            "4k" => Ok(Self::K4),
            other => bail!("unsupported image size {other:?}, supported: {}", image_tier_labels()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::K1 => "1K",
            Self::K2 => "2K",
            Self::K3 => "3K",
            Self::K4 => "4K",
        }
    }
}

fn image_tier_labels() -> String {
    IMAGE_RESOLUTION_TIERS.join(", ")
}

fn image_ratio_labels() -> String {
    IMAGE_SUPPORTED_RATIOS.join(", ")
}

static VIDEO_SUPPORTED_RATIOS: &[&str] = &["21:9", "16:9", "4:3", "1:1", "3:4", "9:16"];
static VIDEO_SIZES: &[&str] = &["720P", "1080P", "1K", "2K"];
static FLASH_VIDEO_SIZES: &[&str] = &["720P"];

/// One selectable aspect ratio with resolved pixel dimensions (for UI pickers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatioOption {
    pub label: String,
    pub dimensions: Dimensions,
    pub tier: &'static str,
}

pub const VIDEO_RESOLUTION_TIER: &str = "720P";

/// Supported image aspect ratios for UI pickers.
pub fn image_ratio_options() -> Vec<RatioOption> {
    IMAGE_SUPPORTED_RATIOS
        .iter()
        .map(|label| RatioOption { label: (*label).to_string(), dimensions: Dimensions::new(0, 0), tier: "" })
        .collect()
}

/// Selectable image resolution tiers (`1K`–`4K`).
pub fn image_resolution_tiers() -> &'static [&'static str] {
    IMAGE_RESOLUTION_TIERS
}

/// Known video aspect ratios with standard-model 720P dimensions.
pub fn video_ratio_options() -> Vec<RatioOption> {
    VIDEO_SUPPORTED_RATIOS
        .iter()
        .map(|label| RatioOption {
            label: (*label).to_string(),
            dimensions: video_dimensions(&AspectRatio::parse(label).expect("static video ratio")),
            tier: VIDEO_RESOLUTION_TIER,
        })
        .collect()
}

pub fn video_size_options(model: &str) -> &'static [&'static str] {
    if model == "agnes-video-2.5-flash" {
        FLASH_VIDEO_SIZES
    } else {
        VIDEO_SIZES
    }
}

pub fn ratio_option_display(option: &RatioOption) -> String {
    if option.dimensions.width == 0 && option.dimensions.height == 0 {
        option.label.clone()
    } else {
        format!("{} · {}", option.label, option.dimensions.size_string())
    }
}

/// Validate image aspect ratio against the API-supported whitelist.
pub fn validate_image_ratio(ratio: &AspectRatio) -> Result<()> {
    let label = ratio.label();
    if IMAGE_SUPPORTED_RATIOS.contains(&label.as_str()) {
        Ok(())
    } else {
        bail!("unsupported image ratio {label}, supported: {}", image_ratio_labels())
    }
}

pub fn validate_video_ratio(ratio: &AspectRatio) -> Result<()> {
    anyhow::ensure!(
        VIDEO_SUPPORTED_RATIOS.contains(&ratio.label().as_str()),
        "unsupported video ratio {}, supported: {}",
        ratio.label(),
        VIDEO_SUPPORTED_RATIOS.join(", ")
    );
    Ok(())
}

/// Standard-model 720P dimensions for the ratio picker.
pub fn video_dimensions(ratio: &AspectRatio) -> Dimensions {
    video_dimensions_for_size(ratio, "720P", false).expect("supported video ratio")
}

/// Documented output dimensions; actual generated file dimensions may vary.
pub fn video_dimensions_for_size(ratio: &AspectRatio, size: &str, flash: bool) -> Result<Dimensions> {
    validate_video_ratio(ratio)?;
    let sizes = if flash { FLASH_VIDEO_SIZES } else { VIDEO_SIZES };
    anyhow::ensure!(
        sizes.contains(&size),
        "unsupported video size {size}, supported: {}",
        sizes.join(", ")
    );
    if size == "1K" {
        return Ok(Dimensions::new(1024, 1024));
    }
    let label = ratio.label();
    let index = VIDEO_SUPPORTED_RATIOS
        .iter()
        .position(|value| *value == label)
        .expect("validated ratio");
    let standard = [(1470, 630), (1280, 720), (1112, 834), (960, 960), (834, 1112), (720, 1280)];
    let hd = [(2206, 946), (1920, 1080), (1664, 1248), (1440, 1440), (1248, 1664), (1080, 1920)];
    let flash_sizes = [(1680, 720), (1280, 704), (960, 720), (720, 720), (720, 960), (720, 1280)];
    let (width, height) = if flash {
        flash_sizes[index]
    } else if size == "1080P" {
        hd[index]
    } else {
        standard[index]
    };
    let scale = if size == "2K" { 2 } else { 1 };
    Ok(Dimensions::new(width * scale, height * scale))
}

/// Agnes Video 2.5 accepts integer seconds from 4 through 12.
pub fn validate_video_duration(duration_secs: f64) -> Result<u32> {
    anyhow::ensure!(
        duration_secs.is_finite() && (4.0..=12.0).contains(&duration_secs) && duration_secs.fract() == 0.0,
        "video duration must be an integer from 4 to 12 seconds"
    );
    Ok(duration_secs as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_ratio_whitelist() {
        assert!(validate_image_ratio(&AspectRatio { w: 1, h: 1 }).is_ok());
        assert!(validate_image_ratio(&AspectRatio { w: 21, h: 9 }).is_ok());
        assert!(validate_image_ratio(&AspectRatio { w: 2, h: 3 }).is_ok());
        assert!(validate_image_ratio(&AspectRatio { w: 5, h: 4 }).is_err());
        assert_eq!(IMAGE_SUPPORTED_RATIOS.len(), 8);
    }

    #[test]
    fn image_resolution_tier_parse() {
        assert_eq!(ImageResolutionTier::K2.as_str(), "2K");
        assert_eq!(ImageResolutionTier::parse("1k").unwrap(), ImageResolutionTier::K1);
        assert_eq!(ImageResolutionTier::parse("2K").unwrap(), ImageResolutionTier::K2);
        assert_eq!(ImageResolutionTier::DEFAULT, ImageResolutionTier::K1);
        assert!(ImageResolutionTier::parse("5k").is_err());
        assert_eq!(image_resolution_tiers().len(), 4);
    }

    #[test]
    fn video_sizes_and_model_limits() {
        let ratio = AspectRatio { w: 16, h: 9 };
        assert_eq!(video_dimensions(&ratio).size_string(), "1280x720");
        assert_eq!(
            video_dimensions_for_size(&ratio, "1080P", false).unwrap().size_string(),
            "1920x1080"
        );
        assert_eq!(
            video_dimensions_for_size(&ratio, "2K", false).unwrap().size_string(),
            "2560x1440"
        );
        assert_eq!(
            video_dimensions_for_size(&ratio, "1K", false).unwrap().size_string(),
            "1024x1024"
        );
        assert_eq!(
            video_dimensions_for_size(&ratio, "720P", true).unwrap().size_string(),
            "1280x704"
        );
        assert!(video_dimensions_for_size(&ratio, "1080P", true).is_err());
        assert!(validate_video_ratio(&AspectRatio { w: 2, h: 3 }).is_err());
        assert_eq!(video_ratio_options().len(), 6);
    }

    #[test]
    fn video_duration_requires_supported_integer_seconds() {
        assert_eq!(validate_video_duration(4.0).unwrap(), 4);
        assert_eq!(validate_video_duration(12.0).unwrap(), 12);
        for invalid in [0.0, 3.0, 4.5, 13.0, f64::NAN, f64::INFINITY] {
            assert!(validate_video_duration(invalid).is_err());
        }
    }

    #[test]
    fn ratio_option_catalog_lists_image_presets() {
        let options = image_ratio_options();
        assert_eq!(options.len(), 8);
        assert!(options.iter().any(|o| o.label == "1:1"));
        assert!(options.iter().any(|o| o.label == "21:9"));
    }
}
