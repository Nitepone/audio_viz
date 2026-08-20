/// polar.rs — Polar waveform oscilloscope (GPU shader).
///
/// The mono waveform is bent into a circle: time maps to angle (one audio
/// window per revolution) and amplitude modulates the radius outward from a
/// base ring — silence draws a perfect circle, loud signals push the
/// perimeter in and out in rhythmic pulses.  The WGSL fragment shader
/// (polar.wgsl) draws the beam analytically with the same phosphor model as
/// classic_lissajous (dwell-weighted Gaussian deposits, feedback-texture
/// persistence).  A dim reference ring marks the zero-amplitude radius so
/// the deformation is always visible even at low gain.
///
/// Native port of the terminal app's `polar` visualizer; its config schema
/// (gain / base_radius / theme) is kept verbatim, with the scope family's
/// persistence and focus settings added.
///
/// This file is only the config wrapper; all drawing lives in the .wgsl.
///
/// Config:
///   gain        — amplitude multiplier before radius modulation
///   base_radius — zero-amplitude ring radius, fraction of the usable radius
///   persistence — fraction of phosphor brightness retained per second
///   theme       — phosphor color: green (P31) / amber (P3) / white (P4)
///   focus       — beam width (lower = sharper trace)
///   window      — length of the traced window in milliseconds (waveform mode)
///   source      — waveform (mono around the ring) / spectrum (circular radar)

use crate::config::merge_config;
use crate::dsp::mag_to_frac;
use crate::visualizer::{AudioFrame, PixelSize, RenderMode, Visualizer, FFT_SIZE, SAMPLE_RATE};

const CONFIG_VERSION: u64 = 1;
const FRAGMENT_WGSL: &str = include_str!("polar.wgsl");
/// Ring geometry, kept in sync with polar.wgsl for the radial cull.
const N_POINTS: usize = 512;
const R_MAX: f32 = 0.93;
const SPEC_LO_HZ: f32 = 30.0;
const SPEC_HI_HZ: f32 = 16000.0;

pub struct PolarViz {
    gain: f32,
    base_radius: f32,
    persistence: f32,
    theme: String,
    focus: f32,
    window_ms: f32,
    source: String,
    /// Radial extent of the trace [inner, outer] in square coords, recomputed
    /// each tick for the shader's early-out.
    r_inner: f32,
    r_outer: f32,
}

impl PolarViz {
    pub fn new() -> Self {
        Self {
            gain: 1.0,
            base_radius: 0.55,
            persistence: 0.5,
            theme: "green".to_string(),
            focus: 1.0,
            window_ms: 23.0,
            source: "waveform".to_string(),
            r_inner: 0.0,
            r_outer: R_MAX,
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

    fn is_spectrum(&self) -> bool {
        self.source == "spectrum"
    }
}

impl Visualizer for PolarViz {
    fn name(&self) -> &str {
        "polar"
    }
    fn description(&self) -> &str {
        "Polar waveform — circular oscilloscope (GPU shader)"
    }
    fn mode(&self) -> RenderMode {
        RenderMode::Shader { fragment_wgsl: FRAGMENT_WGSL }
    }

    fn tick(&mut self, audio: &AudioFrame, _dt: f32, _size: PixelSize) {
        // Recompute the ring's radial extent (square coords) so the shader can
        // cull pixels in the hole or outside the rim.  Mirrors polar_pt().
        let base_frac = self.base_radius.clamp(0.05, 0.95);
        let r_base = base_frac * R_MAX;
        let r_amp = (R_MAX - r_base) * 0.85;

        // Track the amplitude/magnitude range actually present this frame.
        let (mut lo, mut hi) = (0.0f32, 0.0f32); // always include the zero ring
        if self.is_spectrum() {
            let n = audio.fft.len();
            for i in 0..N_POINTS {
                let frac = i as f32 / N_POINTS as f32;
                let m = 1.0 - (2.0 * frac - 1.0).abs();
                let freq = SPEC_LO_HZ * (SPEC_HI_HZ / SPEC_LO_HZ).powf(m);
                let bin = ((freq * FFT_SIZE as f32 / SAMPLE_RATE as f32) as usize).min(n - 1);
                let v = mag_to_frac(audio.fft[bin], -60.0, -12.0);
                hi = hi.max(v);
            }
        } else {
            let n = self.n_samples();
            let base = FFT_SIZE - n;
            let stride = n as f32 / N_POINTS as f32;
            for i in 0..N_POINTS {
                let j = (base + (i as f32 * stride) as usize).min(FFT_SIZE - 1);
                let amp = (audio.mono[j] * self.gain).clamp(-1.0, 1.0);
                lo = lo.min(amp);
                hi = hi.max(amp);
            }
        }
        self.r_inner = (r_base + lo * r_amp).max(0.0);
        self.r_outer = r_base + hi * r_amp;
    }

    fn shader_params(&self) -> [f32; 16] {
        let mut p = [0.0f32; 16];
        p[0] = self.gain;
        p[1] = self.persistence;
        p[2] = self.theme_index();
        p[3] = self.focus;
        p[4] = self.base_radius;
        p[5] = self.n_samples() as f32;
        p[6] = if self.is_spectrum() { 1.0 } else { 0.0 };
        p[7] = self.r_inner;
        p[8] = self.r_outer;
        p
    }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "polar",
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
                    "name": "base_radius",
                    "display_name": "Base Radius",
                    "type": "float",
                    "value": 0.55,
                    "min": 0.2,
                    "max": 0.9
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
                    "name": "source",
                    "display_name": "Source",
                    "type": "enum",
                    "value": "waveform",
                    "variants": ["waveform", "spectrum"]
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
                    "base_radius" => {
                        self.base_radius = entry["value"].as_f64().unwrap_or(0.55) as f32
                    }
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
                    "source" => {
                        self.source =
                            entry["value"].as_str().unwrap_or("waveform").to_string()
                    }
                    _ => {}
                }
            }
        }
        Ok(merged)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(PolarViz::new())]
}
