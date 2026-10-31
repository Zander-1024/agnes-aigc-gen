use anyhow::Result;

use crate::ratio::{self, AspectRatio, ImageResolutionTier, RatioOption};

pub struct ImagePreview {
    #[allow(dead_code)]
    pub ratio_label: String,
    #[allow(dead_code)]
    pub size: String,
    #[allow(dead_code)]
    pub input_count: usize,
    #[allow(dead_code)]
    pub seed_note: String,
    #[allow(dead_code)]
    pub error: Option<String>,
}

pub struct VideoPreview {
    #[allow(dead_code)]
    pub ratio_label: String,
    pub size: String,
    pub tier: String,
    #[allow(dead_code)]
    pub ratio_inferred: bool,
    pub timing: String,
    pub max_duration: String,
    #[allow(dead_code)]
    pub input_count: usize,
    pub input_note: String,
    pub error: Option<String>,
}

pub fn build_image_preview(
    ratio_options: &[RatioOption],
    ratio_index: usize,
    size_tier_index: usize,
    input_count: usize,
    count: u32,
    seed: &str,
) -> ImagePreview {
    let mut error = None;
    let ratio_label = if let Some(opt) = ratio_options.get(ratio_index) {
        opt.label.clone()
    } else {
        error = Some("invalid ratio selection".into());
        "-".into()
    };

    let size = ratio::image_resolution_tiers()
        .get(size_tier_index)
        .map(|s| (*s).to_string())
        .unwrap_or_else(|| {
            error = Some("invalid size selection".into());
            "-".into()
        });

    let seed_note = if count > 1 {
        "disabled (batch)".into()
    } else if seed.trim().is_empty() {
        "random 0–999".into()
    } else {
        format!("fixed {seed}")
    };

    ImagePreview { ratio_label, size, input_count, seed_note, error }
}

pub fn build_video_preview(
    ratio_options: &[RatioOption],
    ratio_index: usize,
    duration: &str,
    size: &str,
    model: &str,
    input_count: usize,
) -> VideoPreview {
    let mut error = None;
    let ratio_label = ratio_options
        .get(ratio_index)
        .map(|opt| opt.label.clone())
        .unwrap_or_default();
    let dimensions = AspectRatio::parse(&ratio_label)
        .and_then(|ratio| ratio::video_dimensions_for_size(&ratio, size, model == "agnes-video-2.5-flash"));
    let size_label = match dimensions {
        Ok(dimensions) => dimensions.size_string(),
        Err(err) => {
            error = Some(format!("{err:#}"));
            "-".into()
        }
    };
    let timing = match duration
        .trim()
        .parse::<f64>()
        .map_err(anyhow::Error::from)
        .and_then(ratio::validate_video_duration)
    {
        Ok(seconds) => format!("{seconds}s"),
        Err(err) => {
            error = Some(format!("{err:#}"));
            "-".into()
        }
    };
    let input_note = match input_count {
        0 => "text",
        1 => "keyframe",
        _ => "reference",
    }
    .to_string();
    VideoPreview {
        ratio_label,
        size: size_label,
        tier: size.to_string(),
        ratio_inferred: false,
        timing,
        max_duration: "12s".into(),
        input_count,
        input_note,
        error,
    }
}

pub fn default_ratio_index(options: &[RatioOption], label: &str) -> usize {
    options.iter().position(|o| o.label == label).unwrap_or(0)
}

pub fn ratio_from_index(options: &[RatioOption], index: usize) -> Result<AspectRatio> {
    let label = options
        .get(index)
        .map(|o| o.label.as_str())
        .ok_or_else(|| anyhow::anyhow!("invalid ratio index"))?;
    AspectRatio::parse(label)
}

pub fn size_tier_from_index(index: usize) -> Result<ImageResolutionTier> {
    let label = ratio::image_resolution_tiers()
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("invalid size index"))?;
    ImageResolutionTier::parse(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_preview_shows_size_tier() {
        let options = ratio::image_ratio_options();
        let index = options.iter().position(|o| o.label == "1:1").unwrap_or(0);
        let preview = build_image_preview(&options, index, 0, 0, 1, "");
        assert_eq!(preview.ratio_label, "1:1");
        assert_eq!(preview.size, "1K");
    }

    #[test]
    fn video_preview_timing() {
        let options = ratio::video_ratio_options();
        let preview = build_video_preview(&options, 4, "5", "720P", "agnes-video-2.5", 0);
        assert_eq!(preview.timing, "5s");
        assert!(preview.max_duration.contains("12"));
    }

    #[test]
    fn video_preview_preserves_selected_ratio_with_inputs() {
        let options = ratio::video_ratio_options();
        let preview = build_video_preview(&options, 0, "5", "720P", "agnes-video-2.5", 1);
        assert!(!preview.ratio_inferred);
        assert_eq!(preview.input_note, "keyframe");
    }
}
