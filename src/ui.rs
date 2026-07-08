/// ui.rs — egui overlay: pop-out side panels around the render pane.
///
/// Layout: a left panel for visualizer selection and a right panel for the
/// active visualizer's settings.  Panels do not resize the window — they
/// claim width from the surface, and `central_px()` reports the remaining
/// area, which the app hands to `GpuContext::set_viz_rect()` so the render
/// pane shrinks to fit.  Small floating buttons open the panels when closed.
///
/// The settings panel is generated from the visualizer's config JSON schema
/// (the same schema the terminal app used): float/int → slider,
/// enum → dropdown, bool → checkbox, plus "Reset to defaults".
///
/// Flow per frame (driven by app.rs):
///   1. `frame()`   — build the UI, collect `UiAction`s, tessellate
///   2. app renders the visualizer into the central rect
///   3. `paint()`   — draw the UI into the same encoder, on top

use egui_wgpu::ScreenDescriptor;
use winit::window::Window;

// ── Actions the UI requests from the app ─────────────────────────────────────

pub enum UiAction {
    /// Switch to the named visualizer.
    SwitchViz(String),
    /// Apply this partial config JSON to the active visualizer and persist.
    ApplyConfig(String),
    /// Restore the active visualizer's default config and persist.
    ResetConfig,
}

// ── Generated settings model ─────────────────────────────────────────────────

enum EntryKind {
    Float { v: f64, min: f64, max: f64 },
    Int { v: i64, min: i64, max: i64 },
    Enum { v: String, variants: Vec<String> },
    Bool { v: bool },
}

struct ConfigEntry {
    name: String,
    display: String,
    kind: EntryKind,
}

fn parse_entries(config_json: &str) -> Vec<ConfigEntry> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(config_json) else {
        return Vec::new();
    };
    let Some(arr) = val["config"].as_array() else { return Vec::new() };

    arr.iter()
        .filter_map(|e| {
            let name = e["name"].as_str()?.to_string();
            let display = e["display_name"].as_str().unwrap_or(&name).to_string();
            let kind = match e["type"].as_str()? {
                "float" => EntryKind::Float {
                    v: e["value"].as_f64().unwrap_or(0.0),
                    min: e["min"].as_f64().unwrap_or(0.0),
                    max: e["max"].as_f64().unwrap_or(1.0),
                },
                "int" => EntryKind::Int {
                    v: e["value"].as_i64().unwrap_or(0),
                    min: e["min"].as_i64().unwrap_or(0),
                    max: e["max"].as_i64().unwrap_or(10),
                },
                "enum" => EntryKind::Enum {
                    v: e["value"].as_str().unwrap_or("").to_string(),
                    variants: e["variants"]
                        .as_array()
                        .map(|v| {
                            v.iter().filter_map(|x| x.as_str().map(String::from)).collect()
                        })
                        .unwrap_or_default(),
                },
                "bool" => EntryKind::Bool { v: e["value"].as_bool().unwrap_or(false) },
                _ => return None,
            };
            Some(ConfigEntry { name, display, kind })
        })
        .collect()
}

// ── The UI state ─────────────────────────────────────────────────────────────

pub struct PanelUi {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,

    pub show_viz_panel: bool,
    pub show_settings: bool,

    /// (category, [(name, description)]) — built once at startup.
    categories: Vec<(String, Vec<(String, String)>)>,
    /// Settings model for the active visualizer.
    entries: Vec<ConfigEntry>,
    viz_name: String,

    /// Central (render pane) rect in egui points, captured during build.
    central_points: egui::Rect,
}

/// Output of `frame()`: actions for the app plus tessellated paint data
/// consumed by `paint()`.
pub struct UiFrameData {
    pub actions: Vec<UiAction>,
    clipped: Vec<egui::ClippedPrimitive>,
    textures_delta: egui::TexturesDelta,
    screen: ScreenDescriptor,
    pixels_per_point: f32,
}

impl PanelUi {
    pub fn new(
        window: &Window,
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        categories: Vec<(String, Vec<(String, String)>)>,
    ) -> Self {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let renderer = egui_wgpu::Renderer::new(device, surface_format, None, 1, false);

        Self {
            ctx,
            state,
            renderer,
            show_viz_panel: false,
            show_settings: false,
            categories,
            entries: Vec::new(),
            viz_name: String::new(),
            central_points: egui::Rect::NOTHING,
        }
    }

    /// Load the settings model for a (newly activated) visualizer from its
    /// current merged config JSON.
    pub fn set_active_viz(&mut self, name: &str, config_json: &str) {
        self.viz_name = name.to_string();
        self.entries = parse_entries(config_json);
    }

    /// Forward a winit event to egui.  Returns egui's response; the app skips
    /// its own key handling when `consumed` is set.
    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &winit::event::WindowEvent,
    ) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }

    /// Build the UI for this frame and tessellate it.
    pub fn frame(&mut self, window: &Window, surface_size: (u32, u32)) -> UiFrameData {
        let raw_input = self.state.take_egui_input(window);
        let mut actions = Vec::new();

        let ctx = self.ctx.clone();
        let output = ctx.run(raw_input, |ctx| self.build(ctx, &mut actions));

        self.state.handle_platform_output(window, output.platform_output);
        let pixels_per_point = output.pixels_per_point;
        let clipped = ctx.tessellate(output.shapes, pixels_per_point);

        UiFrameData {
            actions,
            clipped,
            textures_delta: output.textures_delta,
            screen: ScreenDescriptor {
                size_in_pixels: [surface_size.0, surface_size.1],
                pixels_per_point,
            },
            pixels_per_point,
        }
    }

    /// The render-pane area in physical pixels, from the most recent frame.
    pub fn central_px(&self, data: &UiFrameData, surface_size: (u32, u32)) -> (u32, u32, u32, u32) {
        let ppp = data.pixels_per_point;
        let r = self.central_points;
        if !r.is_positive() {
            return (0, 0, surface_size.0, surface_size.1);
        }
        let (sw, sh) = (surface_size.0 as f32, surface_size.1 as f32);
        let x = (r.min.x * ppp).round().clamp(0.0, sw);
        let y = (r.min.y * ppp).round().clamp(0.0, sh);
        let w = (r.width() * ppp).round().clamp(1.0, sw - x);
        let h = (r.height() * ppp).round().clamp(1.0, sh - y);
        (x as u32, y as u32, w as u32, h as u32)
    }

    // ── UI construction ──────────────────────────────────────────────────────

    fn build(&mut self, ctx: &egui::Context, actions: &mut Vec<UiAction>) {
        // Left: visualizer picker panel
        if self.show_viz_panel {
            egui::SidePanel::left("viz-panel")
                .resizable(false)
                .exact_width(210.0)
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.heading("Visualizers");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("✕").clicked() {
                                self.show_viz_panel = false;
                            }
                        });
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (cat, vizs) in &self.categories {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(capitalize(cat)).strong().small(),
                            );
                            for (name, desc) in vizs {
                                let selected = *name == self.viz_name;
                                let resp = ui
                                    .selectable_label(selected, name)
                                    .on_hover_text(desc);
                                if resp.clicked() && !selected {
                                    actions.push(UiAction::SwitchViz(name.clone()));
                                }
                            }
                        }
                    });
                });
        }

        // Right: settings panel
        if self.show_settings {
            egui::SidePanel::right("settings-panel")
                .resizable(false)
                .exact_width(260.0)
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.heading("Settings");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("✕").clicked() {
                                self.show_settings = false;
                            }
                        });
                    });
                    ui.label(egui::RichText::new(&self.viz_name).weak());
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let mut changed = false;
                        for entry in &mut self.entries {
                            match &mut entry.kind {
                                EntryKind::Float { v, min, max } => {
                                    ui.label(&entry.display);
                                    changed |= ui
                                        .add(
                                            egui::Slider::new(v, *min..=*max)
                                                .fixed_decimals(2),
                                        )
                                        .changed();
                                }
                                EntryKind::Int { v, min, max } => {
                                    ui.label(&entry.display);
                                    changed |=
                                        ui.add(egui::Slider::new(v, *min..=*max)).changed();
                                }
                                EntryKind::Enum { v, variants } => {
                                    ui.label(&entry.display);
                                    egui::ComboBox::from_id_salt(&entry.name)
                                        .selected_text(v.clone())
                                        .width(160.0)
                                        .show_ui(ui, |ui| {
                                            for var in variants.iter() {
                                                if ui
                                                    .selectable_value(v, var.clone(), var)
                                                    .changed()
                                                {
                                                    changed = true;
                                                }
                                            }
                                        });
                                }
                                EntryKind::Bool { v } => {
                                    changed |= ui.checkbox(v, &entry.display).changed();
                                }
                            }
                            ui.add_space(8.0);
                        }
                        if changed {
                            actions.push(UiAction::ApplyConfig(self.partial_json()));
                        }
                        ui.separator();
                        if ui.button("Reset to defaults").clicked() {
                            actions.push(UiAction::ResetConfig);
                        }
                    });
                });
        }

        // Floating open buttons (only when the respective panel is closed).
        if !self.show_viz_panel {
            egui::Area::new(egui::Id::new("open-viz-panel"))
                .anchor(egui::Align2::LEFT_TOP, egui::vec2(8.0, 8.0))
                .show(ctx, |ui| {
                    if ui.button("☰ Visualizers").clicked() {
                        self.show_viz_panel = true;
                    }
                });
        }
        if !self.show_settings {
            egui::Area::new(egui::Id::new("open-settings"))
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0))
                .show(ctx, |ui| {
                    if ui.button("⚙ Settings").clicked() {
                        self.show_settings = true;
                    }
                });
        }

        // Whatever the panels left over is the render pane.
        self.central_points = ctx.available_rect();
    }

    /// Partial config JSON accepted by `Visualizer::set_config()`.
    fn partial_json(&self) -> String {
        let arr: Vec<serde_json::Value> = self
            .entries
            .iter()
            .map(|e| {
                let value = match &e.kind {
                    EntryKind::Float { v, .. } => serde_json::json!(v),
                    EntryKind::Int { v, .. } => serde_json::json!(v),
                    EntryKind::Enum { v, .. } => serde_json::json!(v),
                    EntryKind::Bool { v } => serde_json::json!(v),
                };
                serde_json::json!({ "name": e.name, "value": value })
            })
            .collect();
        serde_json::json!({ "config": arr }).to_string()
    }

    // ── Painting ─────────────────────────────────────────────────────────────

    /// Draw the UI on top of the rendered frame.  Returns command buffers
    /// (from egui texture uploads) to submit before the frame encoder.
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        data: UiFrameData,
    ) -> Vec<wgpu::CommandBuffer> {
        for (id, delta) in &data.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }
        let cmds =
            self.renderer.update_buffers(device, queue, encoder, &data.clipped, &data.screen);

        {
            let pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            let mut pass = pass;
            self.renderer.render(&mut pass, &data.clipped, &data.screen);
        }

        for id in &data.textures_delta.free {
            self.renderer.free_texture(id);
        }
        cmds
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}
