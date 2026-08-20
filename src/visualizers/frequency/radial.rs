/// radial.rs — Polar spectrum radiating from the centre (GPU shader).
///
/// Native port of the terminal "radial" visualizer.  The angular coordinate
/// selects a (log-spaced) frequency band and the normalised radius is
/// compared against that band's energy: a pixel lights when it falls inside
/// its band's reach, so the spectrum fans out from the core like a sunburst.
/// The fragment shader (radial.wgsl) reads the FFT directly from the audio
/// texture and draws the disc analytically, adding a soft rim glow, a beat
/// flash, and feedback-texture persistence for smooth temporal motion — all
/// essentially free on the GPU and rendered at full window resolution.
///
/// This file is only the config wrapper; all drawing lives in the .wgsl.
///
/// Config:
///   gain          — band-energy multiplier
///   persistence   — fraction of phosphor brightness retained per second
///   color_scheme  — spectrum / heat / ice / phosphor
///   symmetry      — mirror the disc left↔right for a symmetric bloom
///   rotate        — angular drift of the whole disc (rad/s)

use crate::config::merge_config;
use crate::visualizer::{AudioFrame, PixelSize, RenderMode, Visualizer};

const CONFIG_VERSION: u64 = 1;
const FRAGMENT_WGSL: &str = include_str!("radial.wgsl");

pub struct RadialViz {
    gain: f32,
    persistence: f32,
    color_scheme: String,
    symmetry: bool,
    rotate: f32,
}

impl RadialViz {
    pub fn new() -> Self {
        Self {
            gain: 2.0,
            persistence: 0.5,
            color_scheme: "spectrum".to_string(),
            symmetry: true,
            rotate: 0.0,
        }
    }

    fn scheme_index(&self) -> f32 {
        match self.color_scheme.as_str() {
            "heat" => 1.0,
            "ice" => 2.0,
            "phosphor" => 3.0,
            _ => 0.0, // spectrum
        }
    }
}

impl Visualizer for RadialViz {
    fn name(&self) -> &str {
        "radial"
    }
    fn description(&self) -> &str {
        "Polar spectrum radiating from the centre (GPU shader)"
    }
    fn mode(&self) -> RenderMode {
        RenderMode::Shader { fragment_wgsl: FRAGMENT_WGSL }
    }

    fn tick(&mut self, _audio: &AudioFrame, _dt: f32, _size: PixelSize) {
        // All per-frame state lives on the GPU (feedback texture); the audio
        // spectrum itself is uploaded to the audio texture by the engine.
    }

    fn shader_params(&self) -> [f32; 16] {
        let mut p = [0.0f32; 16];
        p[0] = self.gain;
        p[1] = self.persistence;
        p[2] = self.scheme_index();
        p[3] = if self.symmetry { 1.0 } else { 0.0 };
        p[4] = self.rotate;
        p
    }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "radial",
            "version": CONFIG_VERSION,
            "config": [
                {
                    "name": "gain",
                    "display_name": "Gain",
                    "type": "float",
                    "value": 2.0,
                    "min": 0.0,
                    "max": 4.0
                },
                {
                    "name": "persistence",
                    "display_name": "Persistence",
                    "type": "float",
                    "value": 0.5,
                    "min": 0.0,
                    "max": 0.99
                },
                {
                    "name": "color_scheme",
                    "display_name": "Color Scheme",
                    "type": "enum",
                    "value": "spectrum",
                    "variants": ["spectrum", "heat", "ice", "phosphor"]
                },
                {
                    "name": "symmetry",
                    "display_name": "Symmetry",
                    "type": "bool",
                    "value": true
                },
                {
                    "name": "rotate",
                    "display_name": "Rotate (rad/s)",
                    "type": "float",
                    "value": 0.0,
                    "min": -2.0,
                    "max": 2.0
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
                    "gain" => self.gain = entry["value"].as_f64().unwrap_or(2.0) as f32,
                    "persistence" => {
                        self.persistence = entry["value"].as_f64().unwrap_or(0.5) as f32
                    }
                    "color_scheme" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.color_scheme = s.to_string();
                        }
                    }
                    "symmetry" => self.symmetry = entry["value"].as_bool().unwrap_or(true),
                    "rotate" => self.rotate = entry["value"].as_f64().unwrap_or(0.0) as f32,
                    _ => {}
                }
            }
        }
        Ok(merged)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(RadialViz::new())]
}
