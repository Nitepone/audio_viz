/// main.rs — CLI entry point.
///
/// Launch options are unchanged from the terminal app:
///
///   audio-viz [VISUALIZER] [--device NAME] [--fps N]
///   audio-viz --list            list visualizers
///   audio-viz --list-devices    list audio input devices

mod app;
mod audio;
#[allow(dead_code)] // beat-detection queries are a library surface for future visualizers
mod beat;
mod config;
mod dsp;
mod gpu;
mod palette;
mod ui;
mod visualizer;
mod visualizers;

use clap::Parser;
use winit::event_loop::EventLoop;

use crate::visualizer::Visualizer;

#[derive(Parser, Debug)]
#[command(name = "audio-viz", about = "2D/3D windowed audio visualizer", long_about = None)]
struct Cli {
    /// Visualizer to run (see --list)
    #[arg(default_value = "spectrogram")]
    visualizer: String,

    /// Audio input device name substring or index (see --list-devices)
    #[arg(short, long)]
    device: Option<String>,

    /// List available visualizers and exit
    #[arg(short, long)]
    list: bool,

    /// List available audio input devices and exit
    #[arg(long)]
    list_devices: bool,

    /// Frame-rate cap (vsync also applies)
    #[arg(long, default_value_t = 120.0)]
    fps: f32,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.list {
        println!("Available visualizers:");
        for v in visualizers::all_visualizers() {
            let mode = match v.mode() {
                visualizer::RenderMode::Software => "software",
                visualizer::RenderMode::Shader { .. } => "shader",
            };
            println!("  {:18} [{:8}] {}", v.name(), mode, v.description());
        }
        return Ok(());
    }

    let host = audio::select_host();

    if cli.list_devices {
        println!("Available input devices (host: {}):", host.id().name());
        for (i, name) in audio::list_devices(&host)?.iter().enumerate() {
            println!("  [{}] {}", i, name);
        }
        return Ok(());
    }

    // ── Select visualizer ─────────────────────────────────────────────────────
    let viz_name = cli.visualizer.to_lowercase();
    let mut viz: Box<dyn Visualizer> = {
        let all = visualizers::all_visualizers();
        let names: Vec<String> = all.iter().map(|v| v.name().to_string()).collect();
        match all.into_iter().find(|v| v.name() == viz_name) {
            Some(v) => v,
            None => {
                eprintln!("Unknown visualizer '{viz_name}'.");
                eprintln!("Available: {}", names.join(", "));
                std::process::exit(1);
            }
        }
    };
    config::load_and_apply_config(&mut viz);

    // ── Audio capture ─────────────────────────────────────────────────────────
    let capture = audio::start_capture(&host, cli.device.as_deref())?;

    // ── Run the windowed app ──────────────────────────────────────────────────
    let event_loop = EventLoop::new()?;
    let mut app = app::App::new(viz, capture, cli.fps);
    event_loop.run_app(&mut app)?;
    Ok(())
}
