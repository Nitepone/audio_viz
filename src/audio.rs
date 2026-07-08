/// audio.rs — Audio capture and analysis pipeline.
///
/// Ported from the terminal app's main.rs:
///   1. Enumerate audio devices and select an input source (loopback devices
///      like BlackHole are preferred on macOS, the "pulse" device on Linux).
///   2. Build a cpal input stream that fills a mutex-guarded ring buffer with
///      interleaved stereo samples.
///   3. Each frame, `FftEngine::process()` drains the ring buffer into
///      sliding FFT_SIZE sample windows and computes the Hann-windowed
///      magnitude spectrum of the mono mix.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustfft::{num_complex::Complex, FftPlanner};

use crate::visualizer::{AudioFrame, CHANNELS, FFT_SIZE, SAMPLE_RATE};

// ── Ring buffer ───────────────────────────────────────────────────────────────

pub type RingBuf = Arc<Mutex<Vec<f32>>>;

fn make_ring() -> RingBuf {
    Arc::new(Mutex::new(Vec::with_capacity(FFT_SIZE * CHANNELS * 4)))
}

// ── Device selection ──────────────────────────────────────────────────────────

pub fn select_host() -> cpal::Host {
    cpal::default_host()
}

pub fn list_devices(host: &cpal::Host) -> anyhow::Result<Vec<String>> {
    Ok(host
        .input_devices()?
        .map(|d| d.name().unwrap_or_else(|_| "?".into()))
        .collect())
}

fn find_best_device(host: &cpal::Host) -> Option<cpal::Device> {
    #[cfg(target_os = "linux")]
    if let Ok(mut devs) = host.input_devices() {
        if let Some(d) = devs.find(|d| d.name().map(|n| n == "pulse").unwrap_or(false)) {
            return Some(d);
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        if let Ok(mut devs) = host.input_devices() {
            if let Some(d) = devs.find(|d| {
                d.name()
                    .map(|n| {
                        let lc = n.to_lowercase();
                        lc.contains("blackhole") || lc.contains("loopback")
                    })
                    .unwrap_or(false)
            }) {
                return Some(d);
            }
        }
        eprintln!("audio: no loopback device found; falling back to the default input.");
        eprintln!("       For system audio on macOS install BlackHole: https://existential.audio/blackhole/");
    }

    host.default_input_device()
}

fn find_device_by_name(host: &cpal::Host, name: &str) -> Option<cpal::Device> {
    let name_lc = name.to_lowercase();
    if let Ok(mut devs) = host.input_devices() {
        if let Some(d) =
            devs.find(|d| d.name().map(|n| n.to_lowercase().contains(&name_lc)).unwrap_or(false))
        {
            return Some(d);
        }
    }
    if let Ok(idx) = name.parse::<usize>() {
        if let Ok(devs) = host.input_devices() {
            return devs.into_iter().nth(idx);
        }
    }
    None
}

// ── Capture ───────────────────────────────────────────────────────────────────

/// A running audio capture: keep the stream alive for as long as you want
/// samples to flow into `ring`.
pub struct AudioCapture {
    pub ring: RingBuf,
    pub device_name: String,
    // Held only to keep the stream alive; dropped with the struct.
    _stream: cpal::Stream,
}

/// Open the requested (or best available) input device and start capturing.
pub fn start_capture(host: &cpal::Host, device_arg: Option<&str>) -> anyhow::Result<AudioCapture> {
    let device = match device_arg {
        Some(name) => find_device_by_name(host, name).ok_or_else(|| {
            anyhow::anyhow!(
                "Device not found: {name}\nRun --list-devices to see available devices."
            )
        })?,
        None => find_best_device(host).ok_or_else(|| {
            anyhow::anyhow!(
                "No suitable input device found.\n\
                 On macOS install BlackHole: https://existential.audio/blackhole/\n\
                 Use --list-devices to see what is available."
            )
        })?,
    };

    let device_name = device.name().unwrap_or_else(|_| "unknown".into());

    let config = {
        let preferred = cpal::StreamConfig {
            channels: CHANNELS as u16,
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };
        let supported = device
            .supported_input_configs()
            .map(|mut it| {
                it.any(|c| {
                    c.sample_format() == cpal::SampleFormat::F32
                        && (c.channels() as usize == CHANNELS || c.channels() >= 1)
                })
            })
            .unwrap_or(false);
        if supported { preferred } else { device.default_input_config()?.into() }
    };

    let actual_channels = config.channels as usize;

    let ring = make_ring();
    let ring2 = Arc::clone(&ring);

    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _| {
            let mut buf = ring2.lock().unwrap();
            for frame in data.chunks(actual_channels) {
                if actual_channels >= 2 {
                    buf.push(frame[0]);
                    buf.push(frame[1]);
                } else {
                    buf.push(frame[0]);
                    buf.push(frame[0]);
                }
            }
            const MAX_RING: usize = FFT_SIZE * CHANNELS * 8;
            if buf.len() > MAX_RING {
                let drain = buf.len() - MAX_RING;
                buf.drain(0..drain);
            }
        },
        |err| eprintln!("[audio error] {err}"),
        None,
    )?;
    stream.play()?;

    Ok(AudioCapture { ring, device_name, _stream: stream })
}

// ── FFT pipeline ──────────────────────────────────────────────────────────────

fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos()))
        .collect()
}

/// Maintains the sliding sample windows and computes the per-frame spectrum.
pub struct FftEngine {
    window: Vec<f32>,
    planner: FftPlanner<f32>,
    left: Vec<f32>,
    right: Vec<f32>,
    mono: Vec<f32>,
}

impl FftEngine {
    pub fn new() -> Self {
        Self {
            window: hann_window(FFT_SIZE),
            planner: FftPlanner::new(),
            left: vec![0.0; FFT_SIZE],
            right: vec![0.0; FFT_SIZE],
            mono: vec![0.0; FFT_SIZE],
        }
    }

    /// Drain the ring buffer into the sliding windows and produce a fresh
    /// `AudioFrame` (clones of the windows plus the FFT of the mono mix).
    pub fn process(&mut self, ring: &RingBuf) -> AudioFrame {
        {
            let mut buf = ring.lock().unwrap();
            if !buf.is_empty() {
                let n_pairs = buf.len() / 2;
                let take = n_pairs.min(FFT_SIZE);
                let keep = FFT_SIZE - take;

                self.left.copy_within(take.., 0);
                self.right.copy_within(take.., 0);
                self.mono.copy_within(take.., 0);

                let start_pair = n_pairs.saturating_sub(take);
                for i in 0..take {
                    let pair_idx = (start_pair + i) * 2;
                    if pair_idx + 1 < buf.len() {
                        let l = buf[pair_idx];
                        let r = buf[pair_idx + 1];
                        self.left[keep + i] = l;
                        self.right[keep + i] = r;
                        self.mono[keep + i] = (l + r) * 0.5;
                    }
                }
                buf.clear();
            }
        }

        let fft = self.compute_fft();
        AudioFrame {
            left: self.left.clone(),
            right: self.right.clone(),
            mono: self.mono.clone(),
            fft,
            sample_rate: SAMPLE_RATE,
        }
    }

    fn compute_fft(&mut self) -> Vec<f32> {
        let n = FFT_SIZE;
        let mut input: Vec<Complex<f32>> = (0..n)
            .map(|i| Complex::new(self.mono[i] * self.window[i], 0.0))
            .collect();
        let fft = self.planner.plan_fft_forward(n);
        fft.process(&mut input);
        let scale = 1.0 / n as f32;
        input[..n / 2 + 1].iter().map(|c| c.norm() * scale).collect()
    }
}
