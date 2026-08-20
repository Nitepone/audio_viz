/// scope.rs — Dual-channel time-domain oscilloscope (GPU shader).
///
/// Left and right channels render as separate waveform panels stacked
/// vertically (cyan / orange), or as a single averaged mono trace.  The WGSL
/// fragment shader (scope.wgsl) draws an antialiased, glowing analytic line
/// by measuring each pixel's distance to the waveform polyline; a faint trail
/// comes from the engine's feedback texture.
///
/// This file is only the config wrapper; all drawing lives in the .wgsl.
///
/// Config:
///   gain      — amplitude multiplier
///   duration  — seconds of audio shown across the width
///   mode      — stereo / mono
///   thickness — line width in pixels
///   trigger   — off / rising: align the left edge to a rising zero-crossing
///               so periodic signals hold still, like a real scope

use crate::config::merge_config;
use crate::visualizer::{AudioFrame, PixelSize, RenderMode, Visualizer, FFT_SIZE, SAMPLE_RATE};

const CONFIG_VERSION: u64 = 1;
const FRAGMENT_WGSL: &str = include_str!("scope.wgsl");

/// Default duration: exactly one full FFT window's worth of samples.
const DURATION_DEFAULT: f32 = FFT_SIZE as f32 / SAMPLE_RATE as f32;

pub struct ScopeViz {
    gain: f32,
    duration: f32,
    mono: bool,
    thickness: f32,
    trigger: bool,
    /// Sample index mapped to the left edge, recomputed each tick.
    trig_base: f32,
}

impl ScopeViz {
    pub fn new() -> Self {
        Self {
            gain: 1.0,
            duration: DURATION_DEFAULT,
            mono: false,
            thickness: 1.5,
            trigger: true,
            trig_base: 0.0,
        }
    }

    fn n_samples(&self) -> usize {
        ((self.duration * SAMPLE_RATE as f32) as usize).clamp(2, FFT_SIZE)
    }
}

impl Visualizer for ScopeViz {
    fn name(&self) -> &str {
        "scope"
    }
    fn description(&self) -> &str {
        "Dual-channel time-domain oscilloscope (GPU shader)"
    }
    fn mode(&self) -> RenderMode {
        RenderMode::Shader { fragment_wgsl: FRAGMENT_WGSL }
    }

    fn tick(&mut self, audio: &AudioFrame, _dt: f32, _size: PixelSize) {
        let n_show = self.n_samples();
        let default_base = (FFT_SIZE - n_show) as f32;
        self.trig_base = if self.trigger {
            trigger_base(&audio.mono, n_show).unwrap_or(default_base)
        } else {
            default_base
        };
    }

    fn shader_params(&self) -> [f32; 16] {
        let mut p = [0.0f32; 16];
        p[0] = self.gain;
        p[1] = self.n_samples() as f32;
        p[2] = if self.mono { 1.0 } else { 0.0 };
        p[3] = self.thickness;
        p[4] = self.trig_base;
        p
    }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "scope",
            "version": CONFIG_VERSION,
            "config": [
                {
                    "name": "gain",
                    "display_name": "Gain",
                    "type": "float",
                    "value": 1.0,
                    "min": 0.0,
                    "max": 4.0
                },
                {
                    "name": "duration",
                    "display_name": "Duration (s)",
                    "type": "float",
                    "value": DURATION_DEFAULT,
                    "min": 0.005,
                    "max": DURATION_DEFAULT
                },
                {
                    "name": "mode",
                    "display_name": "Mode",
                    "type": "enum",
                    "value": "stereo",
                    "variants": ["stereo", "mono"]
                },
                {
                    "name": "thickness",
                    "display_name": "Line Width (px)",
                    "type": "float",
                    "value": 1.5,
                    "min": 0.5,
                    "max": 6.0
                },
                {
                    "name": "trigger",
                    "display_name": "Trigger",
                    "type": "enum",
                    "value": "rising",
                    "variants": ["off", "rising"]
                }
            ]
        })
        .to_string()
    }

    fn set_config(&mut self, json: &str) -> Result<String, String> {
        let merged = merge_config(&self.get_default_config(), json);
        let val: serde_json::Value =
            serde_json::from_str(&merged).map_err(|e| format!("JSON parse error: {e}"))?;
        if let Some(config) = val["config"].as_array() {
            for entry in config {
                match entry["name"].as_str().unwrap_or("") {
                    "gain" => self.gain = entry["value"].as_f64().unwrap_or(1.0) as f32,
                    "duration" => {
                        self.duration =
                            entry["value"].as_f64().unwrap_or(DURATION_DEFAULT as f64) as f32
                    }
                    "mode" => self.mono = entry["value"].as_str() == Some("mono"),
                    "thickness" => {
                        self.thickness = entry["value"].as_f64().unwrap_or(1.5) as f32
                    }
                    "trigger" => self.trigger = entry["value"].as_str() != Some("off"),
                    _ => {}
                }
            }
        }
        Ok(merged)
    }
}

/// Find the sample index to map to the left edge so a periodic signal holds
/// still: the latest rising zero-crossing at or before the plain trailing
/// window start.  Returns `None` when the signal is too quiet to trigger
/// reliably (caller falls back to the untriggered window).
fn trigger_base(mono: &[f32], n_show: usize) -> Option<f32> {
    let base0 = FFT_SIZE.saturating_sub(n_show);
    if base0 < 2 {
        return None;
    }
    // Don't chase noise: require a bit of level in the visible region.
    let peak = mono[base0.saturating_sub(n_show)..]
        .iter()
        .fold(0.0f32, |m, &v| m.max(v.abs()));
    if peak < 0.02 {
        return None;
    }
    // Hysteresis threshold scaled to signal level: cross from below -eps to
    // at/above 0 counts as a rising edge.
    let eps = (peak * 0.1).min(0.05);
    // Look back at most one window for the most recent rising crossing.
    let search = n_show.min(base0);
    for k in 1..search {
        let i = base0 - k;
        if mono[i] >= 0.0 && mono[i - 1] < -eps {
            return Some(i as f32);
        }
    }
    None
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(ScopeViz::new())]
}
