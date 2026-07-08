---
name: new-visualizer
description: Scaffold a complete new audio visualizer for the audio_viz project. Use when the user wants to add a new visualizer — creates the file with all boilerplate filled in, correct imports, config schema, and the file index comment.
argument-hint: <name> <category>
---

Create a new visualizer named `$ARGUMENTS[0]` in category `$ARGUMENTS[1]`.

Valid categories: `frequency`, `scopes`, `effects`, `abstract`. If no category is given, ask.

## Steps

1. **Read context** — read `src/visualizer_utils.rs` lines 1–30 (see the index comment for exact offsets of each utility) and one existing visualizer in the target category as a structural reference.

2. **Create the file** at `src/visualizers/$ARGUMENTS[1]/$ARGUMENTS[0].rs` with the structure below.

3. **Run `cargo check`** and fix any compiler errors before finishing.

4. **Report** the sections the user needs to fill in (render logic, tick logic, config fields).

---

## Required file structure (in order)

```
/// <name>.rs — <one-line description>
///
/// <3-5 line description of the visual effect and how audio drives it>
///
/// Config:
///   <field>  — <range>: <description>
///   …

// ── Index: <Struct>@N · new@N · impl@N · config@N · set_config@N · tick@N · render@N · register@N

use …;

const CONFIG_VERSION: u64 = 1;

// ── Helper functions (if needed) ────────────────────────

pub struct <Name>Viz {
    t:      f32,          // time accumulator
    source: String,
    // smoothed audio bands
    bass:   f32,
    mid:    f32,
    high:   f32,
    // config
    gain:         f32,
    color_scheme: String,
    // … other config fields
}

impl <Name>Viz {
    pub fn new(source: &str) -> Self {
        Self {
            t: 0.0, bass: 0.0, mid: 0.0, high: 0.0,
            source: source.to_string(),
            gain: 1.0,
            color_scheme: "spectrum".to_string(),
            // … defaults
        }
    }
}

impl Visualizer for <Name>Viz {
    fn name(&self)        -> &str { "<name>" }
    fn description(&self) -> &str { "<one-line description>" }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "<name>",
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
                    "name": "color_scheme",
                    "display_name": "Color Scheme",
                    "type": "enum",
                    "value": "spectrum",
                    "variants": ["spectrum", "fire", "neon", "ice"]
                }
                // … additional config fields
            ]
        }).to_string()
    }

    fn set_config(&mut self, json: &str) -> Result<String, String> {
        let merged = merge_config(&self.get_default_config(), json);
        let val: serde_json::Value = serde_json::from_str(&merged)
            .map_err(|e| format!("JSON parse error: {e}"))?;
        if let Some(config) = val["config"].as_array() {
            for entry in config {
                match entry["name"].as_str().unwrap_or("") {
                    "gain" => { self.gain = entry["value"].as_f64().unwrap_or(1.0) as f32; }
                    "color_scheme" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.color_scheme = s.to_string();
                        }
                    }
                    // … other fields
                    _ => {}
                }
            }
        }
        Ok(merged)
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, _size: TermSize) {
        self.t += dt;

        let fft = &audio.fft;
        let n   = fft.len();
        // Use band_energy() from visualizer_utils for clean band extraction:
        //   band_energy(fft, 20.0, 250.0)   → bass
        //   band_energy(fft, 250.0, 4_000.0) → mid
        //   band_energy(fft, 4_000.0, 12_000.0) → high
        // Use smooth_asymmetric(current, target, rise, fall) to smooth bands.
        // Typical rise/fall: bass (0.3, 0.88), mid (0.35, 0.90), high (0.25, 0.92)
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = rows.saturating_sub(1).max(1); // leave one row for status bar

        let mut lines = Vec::with_capacity(rows);

        for r in 0..vis {
            let mut line = String::with_capacity(cols * 14);
            for c in 0..cols {
                // render logic here
                line.push(' ');
            }
            lines.push(line);
        }

        lines.push(status_bar(cols, fps, self.name(), &self.source, ""));
        pad_frame(lines, rows, cols)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(<Name>Viz::new(""))]
}
```

---

## Import conventions

Always import from both modules:

```rust
use crate::visualizer::{
    merge_config,
    pad_frame, status_bar,        // required for render()
    specgrad,                      // if using spectrum colour gradient
    AudioFrame, TermSize, Visualizer,
    // FFT_SIZE, SAMPLE_RATE       // only if needed directly
};
use crate::visualizer_utils::{
    band_energy, smooth_asymmetric, // for audio reactivity
    palette_lookup,                  // for palette-based colour
    brightness_char, ansi_fg,        // for character rendering
    PALETTE_FIRE, PALETTE_ICE, PALETTE_NEON,  // as needed
    // rms, freq_to_bin, mag_to_frac, with_gained_fft — as needed
};
```

Do **not** define local copies of any function already in `visualizer_utils`.

## Colour patterns

- **Palette colour**: `palette_lookup(frac, PALETTE_FIRE)` returns an ANSI 256-colour code (u8)
- **Spectrum gradient**: `specgrad(frac)` returns an ANSI 256-colour code (u8)
- **Rendering**: `format!("\x1b[38;5;{code}m{ch}\x1b[0m")` — or use `ansi_fg(ch, code)`
- **Bold**: prepend `"\x1b[1m"` before the colour code

## Audio reactivity patterns

For per-band energy, prefer `band_energy()` over manual bin slicing unless you need many fine-grained bands:

```rust
let bass = smooth_asymmetric(self.bass, (band_energy(fft, 20.0, 250.0) * self.gain).min(1.0), 0.30, 0.88);
```

For SpectrumBars (many per-column bars like spectrum/radial/fire/matrix):

```rust
use crate::visualizer::{SpectrumBars};
use crate::visualizer_utils::with_gained_fft;
// in tick():
with_gained_fft(&audio.fft, self.gain, |fft| self.bars.update(fft, dt));
```

## Index comment

After writing the file, count the actual line numbers of each key section (the index comment line itself is line N, so all sections below it are shifted by 1 from their pre-comment positions). Write the index comment as line 1 after the doc comment block, before the first `use` statement. Update the numbers to match what is actually in the file.
