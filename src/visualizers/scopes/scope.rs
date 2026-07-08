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
}

impl ScopeViz {
    pub fn new() -> Self {
        Self { gain: 1.0, duration: DURATION_DEFAULT, mono: false, thickness: 1.5 }
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

    fn tick(&mut self, _audio: &AudioFrame, _dt: f32, _size: PixelSize) {}

    fn shader_params(&self) -> [f32; 16] {
        let n_samples =
            ((self.duration * SAMPLE_RATE as f32) as usize).clamp(2, FFT_SIZE) as f32;
        let mut p = [0.0f32; 16];
        p[0] = self.gain;
        p[1] = n_samples;
        p[2] = if self.mono { 1.0 } else { 0.0 };
        p[3] = self.thickness;
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
                    _ => {}
                }
            }
        }
        Ok(merged)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(ScopeViz::new())]
}
