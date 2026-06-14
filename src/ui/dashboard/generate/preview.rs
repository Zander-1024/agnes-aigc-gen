use anyhow::Result;

use crate::ratio::{
    self, AspectRatio, IMAGE_RESOLUTION_TIER, RatioOption, VIDEO_RESOLUTION_TIER, video_timing_preview,
};

pub struct ImagePreview {
    #[allow(dead_code)]
    pub ratio_label: String,
    pub size: String,
    pub tier: String,
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
    input_count: usize,
    count: u32,
    seed: &str,
) -> ImagePreview {
    let mut error = None;
    let (ratio_label, size, tier) = if let Some(opt) = ratio_options.get(ratio_index) {
        (opt.label.clone(), opt.dimensions.size_string(), opt.tier.to_string())
    } else {
        error = Some("invalid ratio selection".into());
        ("-".into(), "-".into(), IMAGE_RESOLUTION_TIER.into())
    };

    let seed_note = if count > 1 {
        "disabled (batch)".into()
    } else if seed.trim().is_empty() {
        "random 0–999".into()
    } else {
        format!("fixed {seed}")
    };

    ImagePreview { ratio_label, size, tier, input_count, seed_note, error }
}

pub fn build_video_preview(
    ratio_options: &[RatioOption],
    ratio_index: usize,
    ratio_disabled: bool,
    duration: &str,
    frame_rate: &str,
    input_count: usize,
) -> VideoPreview {
    let mut error = None;
    let fps: u32 = frame_rate.trim().parse().unwrap_or(24);
    let dur: f64 = duration.trim().parse().unwrap_or(5.0);

    let (ratio_label, size, tier, ratio_inferred) = if ratio_disabled {
        (
            "(from inputs)".into(),
            "(from inputs)".into(),
            VIDEO_RESOLUTION_TIER.into(),
            true,
        )
    } else if let Some(opt) = ratio_options.get(ratio_index) {
        (
            opt.label.clone(),
            opt.dimensions.size_string(),
            opt.tier.to_string(),
            false,
        )
    } else {
        error = Some("invalid ratio selection".into());
        ("-".into(), "-".into(), VIDEO_RESOLUTION_TIER.into(), false)
    };

    let max_duration = match ratio::video_max_duration_secs(fps) {
        Ok(max) => format!("{max:.0}s @ {fps}fps"),
        Err(err) => {
            error = Some(format!("{err:#}"));
            "-".into()
        }
    };

    let timing = match video_timing_preview(dur, fps) {
        Ok((frames, actual)) => format!("frames={frames}, actual={actual:.2}s"),
        Err(err) => {
            error = Some(format!("{err:#}"));
            "-".into()
        }
    };

    let input_note = if input_count == 0 {
        "text-to-video".into()
    } else if input_count == 1 {
        "image-to-video".into()
    } else {
        "multi-frame (same ratio)".into()
    };

    VideoPreview { ratio_label, size, tier, ratio_inferred, timing, max_duration, input_count, input_note, error }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_preview_shows_size() {
        let options = ratio::image_ratio_options();
        let index = options.iter().position(|o| o.label == "1:1").unwrap_or(0);
        let preview = build_image_preview(&options, index, 0, 1, "");
        assert_eq!(preview.ratio_label, "1:1");
        assert_eq!(preview.size, "1024x1024");
    }

    #[test]
    fn video_preview_timing() {
        let options = ratio::video_ratio_options();
        let preview = build_video_preview(&options, 4, false, "5", "24", 0);
        assert!(preview.timing.contains("frames="));
        assert!(preview.max_duration.contains("18"));
    }

    #[test]
    fn video_preview_inferred_ratio_when_inputs() {
        let options = ratio::video_ratio_options();
        let preview = build_video_preview(&options, 0, true, "5", "24", 1);
        assert!(preview.ratio_inferred);
    }
}
