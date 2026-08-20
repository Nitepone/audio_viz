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
///   window      — length of the traced window in milliseconds
///   orientation — left-right (mono on the diagonal) / mid-side (mono vertical)

use crate::config::merge_config;
use crate::visualizer::{AudioFrame, PixelSize, RenderMode, Visualizer, FFT_SIZE, SAMPLE_RATE};

const CONFIG_VERSION: u64 = 1;
const FRAGMENT_WGSL: &str = include_str!("classic_lissajous.wgsl");
/// Control points along the path — must match N_POINTS in the shader so the
/// CPU-computed bounding box matches the drawn geometry.
const N_POINTS: usize = 512;

pub struct ClassicLissajousViz {
    gain: f32,
    persistence: f32,
    theme: String,
    focus: f32,
    window_ms: f32,
    orientation: String,
    /// Trace bounding box [min_x, min_y, max_x, max_y] in square coords,
    /// recomputed each tick for the shader's early-out.
    bbox: [f32; 4],
}

impl ClassicLissajousViz {
    pub fn new() -> Self {
        Self {
            gain: 1.0,
            persistence: 0.5,
            theme: "green".to_string(),
            focus: 1.0,
            window_ms: 23.0,
            orientation: "left-right".to_string(),
            bbox: [-1.2, -1.2, 1.2, 1.2],
        }
    }

    fn theme_index(&self) -> f32 {
        match self.theme.as_str() {
            "amber" => 1.0,
            "white" => 2.0,
            _ => 0.0, // green
        }
    }

    fn n_samples(&self) -> usize {
        ((self.window_ms / 1000.0 * SAMPLE_RATE as f32) as usize).clamp(64, FFT_SIZE)
    }

    fn is_mid_side(&self) -> bool {
        self.orientation == "mid-side"
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

    fn tick(&mut self, audio: &AudioFrame, _dt: f32, _size: PixelSize) {
        // Recompute the trace bounding box (square coords) so the shader can
        // skip pixels the beam never reaches.  Mirrors path_pt() in the WGSL.
        let n = self.n_samples();
        let base = FFT_SIZE - n;
        let stride = n as f32 / N_POINTS as f32;
        let mid_side = self.is_mid_side();
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for i in 0..N_POINTS {
            let j = (base + (i as f32 * stride) as usize).min(FFT_SIZE - 1);
            let l = audio.left[j] * self.gain;
            let r = audio.right[j] * self.gain;
            let (x, y) = if mid_side {
                ((l - r) * 0.707_106_78, -(l + r) * 0.707_106_78)
            } else {
                (l, -r)
            };
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        if min_x <= max_x {
            self.bbox = [min_x, min_y, max_x, max_y];
        }
    }

    fn shader_params(&self) -> [f32; 16] {
        let mut p = [0.0f32; 16];
        p[0] = self.gain;
        p[1] = self.persistence;
        p[2] = self.theme_index();
        p[3] = self.focus;
        p[4] = self.n_samples() as f32;
        p[5] = if self.is_mid_side() { 1.0 } else { 0.0 };
        p[6] = self.bbox[0];
        p[7] = self.bbox[1];
        p[8] = self.bbox[2];
        p[9] = self.bbox[3];
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
                    "value": 0.5,
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
                },
                {
                    "name": "window",
                    "display_name": "Window (ms)",
                    "type": "float",
                    "value": 23.0,
                    "min": 5.0,
                    "max": 50.0
                },
                {
                    "name": "orientation",
                    "display_name": "Orientation",
                    "type": "enum",
                    "value": "left-right",
                    "variants": ["left-right", "mid-side"]
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
                        self.persistence = entry["value"].as_f64().unwrap_or(0.5) as f32
                    }
                    "theme" => {
                        self.theme = entry["value"].as_str().unwrap_or("green").to_string()
                    }
                    "focus" => self.focus = entry["value"].as_f64().unwrap_or(1.0) as f32,
                    "window" => {
                        self.window_ms = entry["value"].as_f64().unwrap_or(23.0) as f32
                    }
                    "orientation" => {
                        self.orientation =
                            entry["value"].as_str().unwrap_or("left-right").to_string()
                    }
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
