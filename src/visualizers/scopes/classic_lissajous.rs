/// classic_lissajous.rs — Classic XY phosphor oscilloscope (GPU shader).
///
/// Left channel drives X, right channel drives Y.  The WGSL fragment shader
/// (classic_lissajous.wgsl) draws the beam as an analytic line — for every
/// pixel it accumulates Gaussian falloff from each consecutive sample-pair
/// segment, weighted by beam speed so slow beam passages glow brighter, just
/// like a real CRT.  Persistence comes from the engine's feedback texture:
/// each frame decays the previous frame instead of clearing it.
///
/// This file is only the config wrapper; all drawing lives in the .wgsl.
///
/// Config:
///   gain        — amplitude multiplier applied to both channels
///   persistence — fraction of phosphor brightness retained per second
///   theme       — phosphor color: green (P31) / amber (P3) / white (P4)
///   focus       — beam width (lower = sharper trace)

use crate::config::merge_config;
use crate::visualizer::{AudioFrame, PixelSize, RenderMode, Visualizer};

const CONFIG_VERSION: u64 = 1;
const FRAGMENT_WGSL: &str = include_str!("classic_lissajous.wgsl");

pub struct ClassicLissajousViz {
    gain: f32,
    persistence: f32,
    theme: String,
    focus: f32,
}

impl ClassicLissajousViz {
    pub fn new() -> Self {
        Self { gain: 1.0, persistence: 0.85, theme: "green".to_string(), focus: 1.0 }
    }

    fn theme_index(&self) -> f32 {
        match self.theme.as_str() {
            "amber" => 1.0,
            "white" => 2.0,
            _ => 0.0, // green
        }
    }
}

impl Visualizer for ClassicLissajousViz {
    fn name(&self) -> &str {
        "classic_lissajous"
    }
    fn description(&self) -> &str {
        "Classic XY phosphor oscilloscope — Lissajous figure (GPU shader)"
    }
    fn mode(&self) -> RenderMode {
        RenderMode::Shader { fragment_wgsl: FRAGMENT_WGSL }
    }

    fn tick(&mut self, _audio: &AudioFrame, _dt: f32, _size: PixelSize) {
        // All per-frame state lives on the GPU (feedback texture); the audio
        // window itself is uploaded by the engine.
    }

    fn shader_params(&self) -> [f32; 16] {
        let mut p = [0.0f32; 16];
        p[0] = self.gain;
        p[1] = self.persistence;
        p[2] = self.theme_index();
        p[3] = self.focus;
        p
    }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "classic_lissajous",
            "version": CONFIG_VERSION,
            "config": [
                {
                    "name": "gain",
                    "display_name": "Gain",
                    "type": "float",
                    "value": 1.0,
                    "min": 0.1,
                    "max": 4.0
                },
                {
                    "name": "persistence",
                    "display_name": "Persistence",
                    "type": "float",
                    "value": 0.85,
                    "min": 0.0,
                    "max": 0.99
                },
                {
                    "name": "theme",
                    "display_name": "Phosphor Color",
                    "type": "enum",
                    "value": "green",
                    "variants": ["green", "amber", "white"]
                },
                {
                    "name": "focus",
                    "display_name": "Beam Focus",
                    "type": "float",
                    "value": 1.0,
                    "min": 0.3,
                    "max": 3.0
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
                    "persistence" => {
                        self.persistence = entry["value"].as_f64().unwrap_or(0.85) as f32
                    }
                    "theme" => {
                        self.theme = entry["value"].as_str().unwrap_or("green").to_string()
                    }
                    "focus" => self.focus = entry["value"].as_f64().unwrap_or(1.0) as f32,
                    _ => {}
                }
            }
        }
        Ok(merged)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(ClassicLissajousViz::new())]
}
