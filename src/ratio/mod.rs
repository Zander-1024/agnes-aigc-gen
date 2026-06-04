use std::collections::HashMap;
use std::sync::LazyLock;

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

    pub fn from_dimensions(width: u32, height: u32) -> Self {
        fn gcd(mut a: u32, mut b: u32) -> u32 {
            while b != 0 {
                let t = b;
                b = a % b;
                a = t;
            }
            a
        }
        let g = gcd(width, height).max(1);
        Self { w: width / g, h: height / g }
    }
}

/// Image output sizes (1K tier).
static IMAGE_RATIO_PRESETS: LazyLock<HashMap<&'static str, Dimensions>> = LazyLock::new(|| {
    HashMap::from([
        ("1:1", Dimensions::new(1024, 1024)),
        ("4:3", Dimensions::new(1152, 864)),
        ("3:4", Dimensions::new(864, 1152)),
        ("16:9", Dimensions::new(1280, 720)),
        ("9:16", Dimensions::new(720, 1280)),
    ])
});

/// Video output sizes (720p tier, shortest edge 768, multiples of 64).
static VIDEO_RATIO_PRESETS: LazyLock<HashMap<&'static str, Dimensions>> = LazyLock::new(|| {
    HashMap::from([
        ("1:1", Dimensions::new(768, 768)),
        ("4:3", Dimensions::new(960, 768)),
        ("3:4", Dimensions::new(768, 960)),
        ("16:9", Dimensions::new(1280, 768)),
        ("9:16", Dimensions::new(768, 1280)),
    ])
});

fn lookup_preset(presets: &HashMap<&'static str, Dimensions>, ratio: &AspectRatio) -> Option<Dimensions> {
    presets.get(ratio.label().as_str()).copied()
}

fn supported_labels(presets: &HashMap<&'static str, Dimensions>) -> String {
    let mut keys: Vec<_> = presets.keys().copied().collect();
    keys.sort_unstable();
    keys.join(", ")
}

/// Fixed image dimensions per supported aspect ratio.
pub fn image_dimensions(ratio: &AspectRatio) -> Result<Dimensions> {
    lookup_preset(&IMAGE_RATIO_PRESETS, ratio).with_context(|| {
        format!(
            "unsupported image ratio {}, supported: {}",
            ratio.label(),
            supported_labels(&IMAGE_RATIO_PRESETS)
        )
    })
}

/// Video dimensions from preset table; unknown ratios snap to 64-multiple 720p tier.
pub fn video_dimensions(ratio: &AspectRatio) -> Dimensions {
    lookup_preset(&VIDEO_RATIO_PRESETS, ratio).unwrap_or_else(|| compute_video_dimensions(ratio))
}

fn compute_video_dimensions(ratio: &AspectRatio) -> Dimensions {
    let w = ratio.w as f64;
    let h = ratio.h as f64;
    let short = 768.0;
    let (width, height) = if w >= h {
        let height = snap64(short);
        let width = snap64(height * w / h);
        (width, height)
    } else {
        let width = snap64(short);
        let height = snap64(width * h / w);
        (width, height)
    };
    Dimensions { width: width as u32, height: height as u32 }
}

fn snap64(v: f64) -> f64 {
    let n = (v / 64.0).round() as i64;
    (n * 64).max(64) as f64
}

/// Maximum video frame count (8n+1 rule, capped at 441).
pub const MAX_VIDEO_FRAMES: u32 = 441;

/// Minimum valid frame count (8n+1 rule).
pub const MIN_VIDEO_FRAMES: u32 = 9;

/// Snap frame count to nearest valid 8n+1 (max [`MAX_VIDEO_FRAMES`]).
pub fn snap_num_frames(requested: u32) -> u32 {
    let clamped = requested.min(MAX_VIDEO_FRAMES).max(MIN_VIDEO_FRAMES);
    let n = ((clamped as i32 - 1) / 8) as u32;
    let lower = 8 * n + 1;
    let upper = 8 * (n + 1) + 1;
    if upper <= MAX_VIDEO_FRAMES && clamped.saturating_sub(lower) > upper.saturating_sub(clamped) {
        upper
    } else {
        lower
    }
}

pub fn duration_from_frames(num_frames: u32, frame_rate: u32) -> f64 {
    num_frames as f64 / frame_rate as f64
}

pub fn validate_frame_rate(frame_rate: u32) -> Result<()> {
    anyhow::ensure!(
        (1..=60).contains(&frame_rate),
        "frame_rate must be 1–60, got {frame_rate}"
    );
    Ok(())
}

pub fn max_video_duration(frame_rate: u32) -> Result<u32> {
    validate_frame_rate(frame_rate)?;
    Ok(MAX_VIDEO_FRAMES / frame_rate)
}

/// Validate requested duration and return snapped frame count + actual duration.
pub fn resolve_video_timing(duration_secs: f64, frame_rate: u32) -> Result<(u32, f64)> {
    validate_frame_rate(frame_rate)?;
    anyhow::ensure!(duration_secs > 0.0, "duration must be positive");
    let max_dur = max_video_duration(frame_rate)?;
    anyhow::ensure!(
        duration_secs <= max_dur as f64,
        "duration {duration_secs}s exceeds maximum {max_dur}s at {frame_rate} fps (max {MAX_VIDEO_FRAMES} frames)"
    );
    let num_frames = frames_from_duration(duration_secs, frame_rate);
    Ok((num_frames, duration_from_frames(num_frames, frame_rate)))
}

pub fn frames_from_duration(duration_secs: f64, frame_rate: u32) -> u32 {
    snap_num_frames((duration_secs * frame_rate as f64).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_presets_from_map() {
        assert_eq!(
            image_dimensions(&AspectRatio { w: 1, h: 1 }).unwrap().size_string(),
            "1024x1024"
        );
        assert_eq!(
            image_dimensions(&AspectRatio { w: 16, h: 9 }).unwrap().size_string(),
            "1280x720"
        );
        assert_eq!(IMAGE_RATIO_PRESETS.len(), 5);
    }

    #[test]
    fn video_presets_differ_from_image() {
        assert_eq!(video_dimensions(&AspectRatio { w: 1, h: 1 }).size_string(), "768x768");
        assert_eq!(video_dimensions(&AspectRatio { w: 16, h: 9 }).size_string(), "1280x768");
        assert_ne!(
            image_dimensions(&AspectRatio { w: 1, h: 1 }).unwrap(),
            video_dimensions(&AspectRatio { w: 1, h: 1 })
        );
        assert_eq!(VIDEO_RATIO_PRESETS.len(), 5);
    }

    #[test]
    fn snap_frames() {
        assert_eq!(snap_num_frames(120), 121);
        assert_eq!(snap_num_frames(500), 441);
    }

    #[test]
    fn max_duration_at_24fps() {
        assert_eq!(max_video_duration(24).unwrap(), 18);
        assert!(resolve_video_timing(18.0, 24).is_ok());
        assert!(resolve_video_timing(18.1, 24).is_err());
    }

    #[test]
    fn frame_rate_range() {
        assert!(validate_frame_rate(1).is_ok());
        assert!(validate_frame_rate(60).is_ok());
        assert!(validate_frame_rate(0).is_err());
        assert!(validate_frame_rate(61).is_err());
    }
}
