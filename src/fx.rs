/// fx.rs — Post-processing effect registry and per-visualizer config wrapper.
///
/// Post effects are fragment shaders applied after the visualizer renders,
/// chained into the final blit to the swapchain (last pass runs at display
/// resolution inside the viz rect; see src/gpu/fx_prelude.wgsl for the
/// shader-side contract).  They work on top of every visualizer — shader,
/// software, and terminal-rendered alike.
///
/// The `EFFECTS` table is the single source of truth.  `FxViz` (the same
/// wrapper pattern as `term::TermViz`) turns it into config entries appended
/// to the wrapped visualizer's schema — an on/off toggle plus one slider per
/// parameter — so the settings UI and per-visualizer persistence come for
/// free.  app.rs reads the enabled entries back as the GPU chain each frame
/// via `FxViz::chain()`.
///
/// Adding an effect:
///   1. Create src/gpu/fx/<name>.wgsl (fragment only, against fx_prelude)
///   2. Add a row to EFFECTS with its parameter specs (max 8 params)
/// Everything else — settings UI, persistence, pipeline setup — is generic.

use crate::gpu::FxInstance;
use crate::visualizer::{AudioFrame, Framebuffer, PixelSize, RenderMode, Visualizer};

pub struct FxParam {
    pub name: &'static str,
    pub display: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
}

pub struct FxEffect {
    pub name: &'static str,
    pub display: &'static str,
    pub wgsl: &'static str,
    pub params: &'static [FxParam],
}

pub const EFFECTS: &[FxEffect] = &[FxEffect {
    name: "crt",
    display: "CRT Effect",
    wgsl: include_str!("gpu/fx/crt.wgsl"),
    params: &[
        FxParam {
            name: "curvature",
            display: "CRT Curvature",
            default: 0.4,
            min: 0.0,
            max: 1.0,
        },
        FxParam {
            name: "scanlines",
            display: "CRT Scanlines",
            default: 0.6,
            min: 0.0,
            max: 1.0,
        },
        FxParam { name: "mask", display: "CRT Mask", default: 0.35, min: 0.0, max: 1.0 },
    ],
}];

/// Wraps any visualizer, appending the post-effect settings to its config
/// schema.  Effect state lives here (not on the GPU side) so it persists in
/// the visualizer's JSON config like every other setting.
pub struct FxViz {
    inner: Box<dyn Visualizer>,
    /// Parallel to EFFECTS.
    enabled: Vec<bool>,
    params: Vec<[f32; 8]>,
}

impl FxViz {
    pub fn new(inner: Box<dyn Visualizer>) -> Self {
        let params = EFFECTS
            .iter()
            .map(|e| {
                let mut p = [0.0f32; 8];
                for (i, spec) in e.params.iter().take(8).enumerate() {
                    p[i] = spec.default as f32;
                }
                p
            })
            .collect();
        Self { inner, enabled: vec![false; EFFECTS.len()], params }
    }

    /// The GPU post-effect chain for the current settings, in EFFECTS order.
    pub fn chain(&self) -> Vec<FxInstance> {
        EFFECTS
            .iter()
            .enumerate()
            .filter(|(i, _)| self.enabled[*i])
            .map(|(i, e)| FxInstance { name: e.name, wgsl: e.wgsl, params: self.params[i] })
            .collect()
    }
}

impl Visualizer for FxViz {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn mode(&self) -> RenderMode {
        self.inner.mode()
    }
    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: PixelSize) {
        self.inner.tick(audio, dt, size);
    }
    fn render_software(&mut self, size: PixelSize) -> Option<&Framebuffer> {
        self.inner.render_software(size)
    }
    fn shader_params(&self) -> [f32; 16] {
        self.inner.shader_params()
    }
    fn on_resize(&mut self, size: PixelSize) {
        self.inner.on_resize(size);
    }

    fn get_default_config(&self) -> String {
        // The wrapped visualizer's schema plus one toggle + sliders per effect.
        let mut val: serde_json::Value = serde_json::from_str(&self.inner.get_default_config())
            .unwrap_or_else(|_| serde_json::json!({ "config": [] }));
        if let Some(entries) = val["config"].as_array_mut() {
            for e in EFFECTS {
                entries.push(serde_json::json!({
                    "name": format!("fx_{}", e.name),
                    "display_name": e.display,
                    "type": "bool",
                    "value": false,
                }));
                for p in e.params {
                    entries.push(serde_json::json!({
                        "name": format!("fx_{}_{}", e.name, p.name),
                        "display_name": p.display,
                        "type": "float",
                        "value": p.default,
                        "min": p.min,
                        "max": p.max,
                    }));
                }
            }
        }
        val.to_string()
    }

    fn set_config(&mut self, json: &str) -> Result<String, String> {
        let merged = crate::config::merge_config(&self.get_default_config(), json);
        let val: serde_json::Value =
            serde_json::from_str(&merged).map_err(|e| format!("JSON parse error: {e}"))?;

        if let Some(config) = val["config"].as_array() {
            for entry in config {
                let name = entry["name"].as_str().unwrap_or("");
                for (i, e) in EFFECTS.iter().enumerate() {
                    if name == format!("fx_{}", e.name) {
                        self.enabled[i] = entry["value"].as_bool().unwrap_or(false);
                    } else {
                        for (j, p) in e.params.iter().take(8).enumerate() {
                            if name == format!("fx_{}_{}", e.name, p.name) {
                                self.params[i][j] =
                                    entry["value"].as_f64().unwrap_or(p.default) as f32;
                            }
                        }
                    }
                }
            }
        }

        // The wrapped visualizer merges against its own schema, silently
        // dropping the fx_* entries; the combined `merged` is what we
        // persist and report back.
        self.inner.set_config(&merged)?;
        Ok(merged)
    }
}
