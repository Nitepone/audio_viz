// ── Index: Nebula@10 · new@30 · random_particle@51 · impl@65 · tick@69 · render@120 · on_resize@160 · get_default_config@170 · set_config@184 · register@202
use std::f32::consts::TAU;

use crate::beat::{BeatDetector, BeatDetectorConfig};
use crate::visualizer::{merge_config, AudioFrame, TermSize, Visualizer};
use crate::visualizer_utils::{ansi_dim_fg, ansi_fg, palette_lookup, PALETTE_NEON};
use rand::Rng;

/// Nebula: A fluid, curl-field based particle visualizer.
/// Particles flow through a smoothly varying vector field driven by audio energy.
pub struct Nebula {
    particles: Vec<Particle>,
    field_seed: f32,
    beat: BeatDetector,
    surge: f32,
    // Configurable parameters
    gain: f32,
    speed: f32,
    turbulence: f32,
}

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    age: f32,
    max_age: f32,
    color: u8,
}

impl Nebula {
    pub fn new(size: TermSize) -> Self {
        let count = (size.rows as usize * size.cols as usize) / 4;
        let mut particles = Vec::with_capacity(count);
        for _ in 0..count {
            particles.push(Self::random_particle(size));
        }
        Self {
            particles,
            field_seed: 0.0,
            beat: BeatDetector::new(BeatDetectorConfig::standard()),
            surge: 0.0,
            gain: 1.0,
            speed: 1.0,
            turbulence: 1.0,
        }
    }

    fn random_particle(size: TermSize) -> Particle {
        let mut rng = rand::thread_rng();
        let max_age = rng.gen_range(20.0..60.0);
        Particle {
            x: rng.gen_range(0.0..size.cols as f32),
            y: rng.gen_range(0.0..size.rows as f32),
            vx: 0.0,
            vy: 0.0,
            // Stagger initial ages so particles don't all expire at the same time
            age: rng.gen_range(0.0..max_age * 0.8),
            max_age,
            color: palette_lookup(rng.gen_range(0.0..1.0), PALETTE_NEON),
        }
    }
}

impl Visualizer for Nebula {
    fn name(&self) -> &str { "nebula" }

    fn description(&self) -> &str { "A fluid vector-field simulation driven by audio energy." }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: TermSize) {
        self.beat.update(&audio.fft, dt);
        self.field_seed += dt * 0.15;

        // Beat surge: accumulate on each beat, decay smoothly
        if self.beat.is_beat() {
            self.surge = (self.surge + self.beat.beat_intensity() * 0.5).min(2.0);
        }
        self.surge *= 1.0 - (dt * 3.0).min(1.0);

        let bass_energy = audio.fft.iter().take(10).sum::<f32>() / 10.0;
        let effective_speed = self.speed * (1.0 + self.surge * 1.5);

        let mut rng = rand::thread_rng();
        for p in self.particles.iter_mut() {
            p.age += dt;
            if p.age >= p.max_age {
                // Respawn with a fresh position, lifetime, and color
                p.x = rng.gen_range(0.0..size.cols as f32);
                p.y = rng.gen_range(0.0..size.rows as f32);
                p.vx = 0.0;
                p.vy = 0.0;
                p.age = 0.0;
                p.max_age = rng.gen_range(20.0..60.0);
                p.color = palette_lookup(rng.gen_range(0.0..1.0), PALETTE_NEON);
            }

            // Curl-style flow field: compute a smooth angle from the 2D position,
            // then use cos/sin to produce a tangential (swirling) force.
            // Using separate x/y frequencies avoids the diagonal-band artifact.
            let freq_mod = (bass_energy * 4.0 * self.gain).clamp(0.5, 6.0);
            let theta = (p.x * 0.05).sin() * (p.y * 0.05).cos() * TAU
                      * freq_mod + self.field_seed * TAU;
            let noise_x = theta.cos() * self.turbulence;
            let noise_y = theta.sin() * self.turbulence;

            p.vx += noise_x * dt * effective_speed;
            p.vy += noise_y * dt * effective_speed;

            // Slightly more drag than before to prevent runaway speeds
            p.vx *= 0.92;
            p.vy *= 0.92;

            p.x += p.vx * dt;
            p.y += p.vy * dt;

            // Boundary wrap
            if p.x < 0.0 { p.x += size.cols as f32; }
            if p.x >= size.cols as f32 { p.x -= size.cols as f32; }
            if p.y < 0.0 { p.y += size.rows as f32; }
            if p.y >= size.rows as f32 { p.y -= size.rows as f32; }
        }
    }

    fn render(&self, size: TermSize, _fps: f32) -> Vec<String> {
        let cols = size.cols as usize;
        let rows = size.rows as usize;
        // Each cell stores (char, color, life_frac); life_frac=0 means empty.
        let mut grid: Vec<(char, u8, f32)> = vec![(' ', 0, 0.0); cols * rows];

        for p in &self.particles {
            let ix = p.x as usize;
            let iy = p.y as usize;
            if ix < cols && iy < rows {
                let life_frac = 1.0 - (p.age / p.max_age);
                let cell = &mut grid[iy * cols + ix];
                // When particles overlap, the fresher (higher life_frac) one wins
                if life_frac > cell.2 {
                    let ch = if life_frac > 0.7 { '*' }
                             else if life_frac > 0.4 { 'o' }
                             else { '.' };
                    *cell = (ch, p.color, life_frac);
                }
            }
        }

        let mut lines = Vec::with_capacity(rows);
        for y in 0..rows {
            let mut line = String::with_capacity(cols * 12);
            for x in 0..cols {
                let (ch, color, life_frac) = grid[y * cols + x];
                if life_frac > 0.0 {
                    // Dim particles near end of life for a fade-out effect
                    if life_frac < 0.25 {
                        line.push_str(&ansi_dim_fg(&ch.to_string(), color));
                    } else {
                        line.push_str(&ansi_fg(ch, color));
                    }
                } else {
                    line.push(' ');
                }
            }
            lines.push(line);
        }
        lines
    }

    fn on_resize(&mut self, size: TermSize) {
        let count = (size.rows as usize * size.cols as usize) / 4;
        self.particles.truncate(count);
        while self.particles.len() < count {
            self.particles.push(Self::random_particle(size));
        }
    }

    fn get_default_config(&self) -> String {
        r#"{
  "visualizer_name": "nebula",
  "version": 1,
  "config": [
    { "name": "gain",        "display_name": "Gain",        "type": "float", "value": 1.0, "min": 0.0, "max": 5.0 },
    { "name": "speed",       "display_name": "Speed",       "type": "float", "value": 1.0, "min": 0.1, "max": 5.0 },
    { "name": "turbulence",  "display_name": "Turbulence",  "type": "float", "value": 1.0, "min": 0.1, "max": 5.0 }
  ]
}"#.to_string()
    }

    fn set_config(&mut self, json: &str) -> Result<String, String> {
        let merged = merge_config(&self.get_default_config(), json);
        let val: serde_json::Value = serde_json::from_str(&merged)
            .map_err(|e| format!("JSON parse error: {e}"))?;
        if let Some(config) = val["config"].as_array() {
            for entry in config {
                match entry["name"].as_str().unwrap_or("") {
                    "gain"       => { self.gain       = entry["value"].as_f64().unwrap_or(1.0) as f32; }
                    "speed"      => { self.speed      = entry["value"].as_f64().unwrap_or(1.0) as f32; }
                    "turbulence" => { self.turbulence = entry["value"].as_f64().unwrap_or(1.0) as f32; }
                    _ => {}
                }
            }
        }
        Ok(merged)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(Nebula::new(TermSize { rows: 24, cols: 80 }))]
}
