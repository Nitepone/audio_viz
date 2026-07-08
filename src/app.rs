/// app.rs — Windowed application: winit event loop and per-frame pipeline.
///
/// Each redraw:
///   1. Drain the audio ring buffer and compute the FFT (audio.rs).
///   2. Update the shared beat detector.
///   3. Build the UI (src/ui.rs) — side panels claim width; the remaining
///      central rect becomes the render pane (GpuContext::set_viz_rect).
///   4. Apply UI actions (switch visualizer / config changes).
///   5. viz.tick() with the fresh AudioFrame at the render-pane size.
///   6. Render the visualizer into the pane, then the UI on top, same frame.
///
/// Keys (when not captured by the UI): q / Esc quit · f fullscreen ·
/// v visualizer panel · s settings panel.

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::audio::{self, AudioCapture, FftEngine};
use crate::beat::{BeatDetector, BeatDetectorConfig};
use crate::config;
use crate::dsp::rms;
use crate::fx::FxViz;
use crate::gpu::{AudioTexData, GpuContext, Uniforms};
use crate::ui::{PanelUi, UiAction};
use crate::visualizer::{PixelSize, RenderMode, Visualizer, SAMPLE_RATE};
use crate::visualizers;

/// Construct a visualizer by name from the auto-discovered registry.
pub fn make_viz(name: &str) -> Option<Box<dyn Visualizer>> {
    visualizers::all_visualizers().into_iter().find(|v| v.name() == name)
}

/// (category, [(name, description)]) for the picker panel.
fn build_categories() -> Vec<(String, Vec<(String, String)>)> {
    let all = visualizers::all_visualizers();
    visualizers::visualizer_categories()
        .into_iter()
        .map(|(cat, names)| {
            let entries = names
                .into_iter()
                .map(|n| {
                    let desc = all
                        .iter()
                        .find(|v| v.name() == n)
                        .map(|v| v.description().to_string())
                        .unwrap_or_default();
                    (n.to_string(), desc)
                })
                .collect();
            (cat.to_string(), entries)
        })
        .collect()
}

pub struct App {
    /// The active visualizer, wrapped in the post-effect config layer.
    viz: FxViz,
    capture: AudioCapture,
    host: cpal::Host,
    fft: FftEngine,
    beat: BeatDetector,

    window: Option<Arc<Window>>,
    gpu: Option<GpuContext>,
    ui: Option<PanelUi>,

    start: Instant,
    t_prev: Instant,
    /// Minimum frame duration from --fps (vsync still applies on top).
    frame_cap: Duration,
    fps_ema: f32,
    last_title: Instant,
}

impl App {
    pub fn new(
        viz: Box<dyn Visualizer>,
        capture: AudioCapture,
        fps_cap: f32,
        host: cpal::Host,
    ) -> Self {
        // Wrap before loading the saved config so persisted fx_* entries
        // reach the wrapper instead of being dropped by the inner schema.
        let mut viz = FxViz::new(viz);
        config::load_and_apply_config(&mut viz);
        let now = Instant::now();
        Self {
            viz,
            capture,
            host,
            fft: FftEngine::new(),
            beat: BeatDetector::new(BeatDetectorConfig::standard()),
            window: None,
            gpu: None,
            ui: None,
            start: now,
            t_prev: now,
            frame_cap: Duration::from_secs_f32(1.0 / fps_cap.max(1.0)),
            fps_ema: fps_cap,
            last_title: now,
        }
    }

    fn title(&self) -> String {
        format!(
            "audio-viz — {} · {} · {:.0} fps",
            self.viz.name(),
            self.capture.device_name,
            self.fps_ema
        )
    }

    fn redraw(&mut self) {
        let (Some(gpu), Some(ui), Some(window)) =
            (self.gpu.as_mut(), self.ui.as_mut(), self.window.as_ref())
        else {
            return;
        };

        let t0 = Instant::now();
        let dt = (t0 - self.t_prev).as_secs_f32().clamp(1e-4, 0.15);
        self.t_prev = t0;

        let audio = self.fft.process(&self.capture.ring);
        self.beat.update(&audio.fft, dt);

        // ── UI frame: panels claim width, central rect is the render pane ────
        let surface_size = gpu.surface_size();
        let ui_frame = ui.frame(window, surface_size);

        for action in &ui_frame.actions {
            match action {
                UiAction::SwitchViz(name) => {
                    if let Some(inner) = make_viz(name) {
                        let mut new_viz = FxViz::new(inner);
                        config::load_and_apply_config(&mut new_viz);
                        if let RenderMode::Shader { fragment_wgsl } = new_viz.mode() {
                            gpu.set_shader(fragment_wgsl);
                        }
                        ui.set_active_viz(new_viz.name(), &config::live_config(&new_viz));
                        self.viz = new_viz;
                    }
                }
                UiAction::ApplyConfig(partial) => {
                    if let Ok(cleaned) = self.viz.set_config(partial) {
                        let _ = config::write_config(self.viz.name(), &cleaned);
                    }
                }
                UiAction::ResetConfig => {
                    let default = self.viz.get_default_config();
                    if let Ok(cleaned) = self.viz.set_config(&default) {
                        let _ = config::write_config(self.viz.name(), &cleaned);
                        ui.set_active_viz(self.viz.name(), &cleaned);
                    }
                }
                UiAction::SwitchDevice(name) => match audio::start_capture(&self.host, Some(name))
                {
                    Ok(new_capture) => {
                        ui.set_current_device(&new_capture.device_name);
                        self.capture = new_capture;
                    }
                    Err(e) => eprintln!("[audio error] failed to switch device: {e}"),
                },
            }
        }

        let (rx, ry, rw, rh) = ui.central_px(&ui_frame, surface_size);
        gpu.set_viz_rect(rx, ry, rw, rh);
        gpu.set_post_chain(&self.viz.chain(), (t0 - self.start).as_secs_f32());

        let size = match self.viz.mode() {
            RenderMode::Shader { .. } => {
                let (w, h) = gpu.shader_resolution();
                PixelSize { width: w, height: h }
            }
            RenderMode::Software => PixelSize { width: rw.max(1), height: rh.max(1) },
        };

        self.viz.tick(&audio, dt, size);

        // ── Render: visualizer pane, then UI on top, in one frame ────────────
        let mut fctx = match gpu.begin_frame() {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[render error] {e}");
                return;
            }
        };

        match self.viz.mode() {
            RenderMode::Software => {
                if let Some(fb) = self.viz.render_software(size) {
                    gpu.render_software(&mut fctx, fb);
                }
            }
            RenderMode::Shader { .. } => {
                let mut params = [[0.0f32; 4]; 4];
                let flat = self.viz.shader_params();
                for (i, v) in flat.iter().enumerate() {
                    params[i / 4][i % 4] = *v;
                }
                let uniforms = Uniforms {
                    resolution: [size.width as f32, size.height as f32],
                    time: (t0 - self.start).as_secs_f32(),
                    dt,
                    rms: [rms(&audio.left), rms(&audio.right)],
                    beat: self.beat.beat_intensity(),
                    sample_rate: SAMPLE_RATE as f32,
                    params,
                };
                let tex = AudioTexData::pack(&audio.left, &audio.right, &audio.mono, &audio.fft);
                gpu.render_shader(&mut fctx, &uniforms, &tex);
            }
        }

        let pre_cmds = ui.paint(gpu.device(), gpu.queue(), &mut fctx.encoder, &fctx.view, ui_frame);
        gpu.end_frame(fctx, pre_cmds);

        // FPS bookkeeping + occasional title refresh.
        let inst_fps = 1.0 / dt.max(1e-6);
        self.fps_ema = 0.08 * inst_fps + 0.92 * self.fps_ema;
        if (t0 - self.last_title).as_secs_f32() > 0.5 {
            self.last_title = t0;
            window.set_title(&self.title());
        }

        // Honour --fps as an upper bound (vsync already paces us; this only
        // matters when the cap is below the display refresh rate).
        if let Some(sleep) = self.frame_cap.checked_sub(t0.elapsed()) {
            if sleep > Duration::from_millis(1) {
                std::thread::sleep(sleep);
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(self.title())
                        .with_inner_size(LogicalSize::new(1100.0, 750.0)),
                )
                .expect("failed to create window"),
        );

        let mut gpu = GpuContext::new(window.clone()).expect("failed to initialise GPU");
        if let RenderMode::Shader { fragment_wgsl } = self.viz.mode() {
            gpu.set_shader(fragment_wgsl);
        }

        let devices = audio::list_devices(&self.host).unwrap_or_default();
        let mut ui = PanelUi::new(
            &window,
            gpu.device(),
            gpu.surface_format(),
            build_categories(),
            devices,
            self.capture.device_name.clone(),
        );
        ui.set_active_viz(self.viz.name(), &config::live_config(&self.viz));

        let (w, h) = gpu.surface_size();
        self.viz.on_resize(PixelSize { width: w, height: h });

        self.window = Some(window);
        self.gpu = Some(gpu);
        self.ui = Some(ui);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Give egui first claim on the event.
        let mut ui_consumed = false;
        if let (Some(ui), Some(window)) = (self.ui.as_mut(), self.window.as_ref()) {
            let resp = ui.on_window_event(window, &event);
            ui_consumed = resp.consumed;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(new_size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(new_size.width, new_size.height);
                }
                self.viz.on_resize(PixelSize {
                    width: new_size.width.max(1),
                    height: new_size.height.max(1),
                });
            }

            WindowEvent::KeyboardInput {
                event: KeyEvent { logical_key, state: ElementState::Pressed, .. },
                ..
            } if !ui_consumed => match logical_key {
                Key::Named(NamedKey::Escape) => event_loop.exit(),
                Key::Character(c) if c == "q" => event_loop.exit(),
                Key::Character(c) if c == "f" => {
                    if let Some(w) = &self.window {
                        let fs = if w.fullscreen().is_some() {
                            None
                        } else {
                            Some(Fullscreen::Borderless(None))
                        };
                        w.set_fullscreen(fs);
                    }
                }
                Key::Character(c) if c == "v" => {
                    if let Some(ui) = self.ui.as_mut() {
                        ui.show_viz_panel = !ui.show_viz_panel;
                    }
                }
                Key::Character(c) if c == "s" => {
                    if let Some(ui) = self.ui.as_mut() {
                        ui.show_settings = !ui.show_settings;
                    }
                }
                _ => {}
            },

            WindowEvent::RedrawRequested => {
                self.redraw();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Tear down GPU and window resources while the event loop (and with it
    /// the Wayland/X11 display connection) still exists. `run_app` consumes
    /// the `EventLoop`, so anything left alive in `App` after it returns
    /// would be destroyed against a dead display connection — a segfault on
    /// Wayland.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.ui = None;
        self.gpu = None;
        self.window = None;
    }
}
