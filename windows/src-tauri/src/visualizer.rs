// Music visualizer — captures Windows system audio (WASAPI loopback)
// and computes 16 frequency bands via FFT for the Dynamic Island and Now Playing card.

use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::speech::audio::{self, fft};
use crate::speech::SAMPLE_RATE;

pub const BANDS_COUNT: usize = 16;
const FFT_SIZE: usize = 512;
const LOW_HZ: f64 = 45.0;
const HIGH_HZ: f64 = 7500.0;

/// Generates bin ranges [from, to) for `count` bands spaced logarithmically.
fn compute_bin_ranges(count: usize) -> Vec<(usize, usize)> {
    let bin_hz = SAMPLE_RATE as f64 / FFT_SIZE as f64;
    let max_bin = FFT_SIZE / 2;
    let mut edges = Vec::with_capacity(count + 1);
    for i in 0..=count {
        edges.push(LOW_HZ * (HIGH_HZ / LOW_HZ).powf(i as f64 / count as f64));
    }
    let mut out = Vec::with_capacity(count);
    let mut last = 1;
    for i in 0..count {
        let from = last.max((edges[i] / bin_hz).floor() as usize);
        let to = max_bin.min((from + 1).max((edges[i + 1] / bin_hz).ceil() as usize));
        out.push((from, to));
        last = to;
    }
    out
}

pub struct MusicBandMeter {
    pending: Vec<f32>,
    ranges: Vec<(usize, usize)>,
    pub levels: [f32; BANDS_COUNT],
}

impl Default for MusicBandMeter {
    fn default() -> Self {
        Self {
            pending: Vec::with_capacity(FFT_SIZE),
            ranges: compute_bin_ranges(BANDS_COUNT),
            levels: [0.0; BANDS_COUNT],
        }
    }
}

impl MusicBandMeter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds 16 kHz samples; returns true if an FFT frame was computed.
    pub fn feed(&mut self, samples: &[f32]) -> bool {
        let mut measured = false;
        for &s in samples {
            self.pending.push(s);
            if self.pending.len() == FFT_SIZE {
                self.measure();
                self.pending.clear();
                measured = true;
            }
        }
        measured
    }

    fn measure(&mut self) {
        let mut re = vec![0.0f64; FFT_SIZE];
        let mut im = vec![0.0f64; FFT_SIZE];
        for (i, &s) in self.pending.iter().enumerate() {
            // Hann window
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (FFT_SIZE - 1) as f64).cos();
            re[i] = s as f64 * w;
        }
        fft(&mut re, &mut im);

        // Noise floor -72 dB, peak ceiling -18 dB
        let floor = -72.0f64;
        let ceiling = -18.0f64;

        for (b, &(from, to)) in self.ranges.iter().enumerate() {
            let mut power = 0.0f64;
            for k in from..to {
                power += re[k] * re[k] + im[k] * im[k];
            }
            power /= ((to - from).max(1) * FFT_SIZE) as f64;
            let db = 10.0 * (power + 1e-12).log10();
            let raw_level = ((db - floor) / (ceiling - floor)).clamp(0.0, 1.0) as f32;

            // Attack fast (0.75), decay smoothly (0.28)
            let k = if raw_level > self.levels[b] { 0.75 } else { 0.28 };
            self.levels[b] += (raw_level - self.levels[b]) * k;
        }
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.pending.clear();
        self.levels = [0.0; BANDS_COUNT];
    }
}

struct VisualizerState {
    active: bool,
    capture: Option<audio::Capture>,
}

static STATE: LazyLock<Mutex<VisualizerState>> =
    LazyLock::new(|| Mutex::new(VisualizerState { active: false, capture: None }));

pub fn set_active(app: &AppHandle, active: bool) {
    let mut state = STATE.lock().unwrap();
    if state.active == active {
        return;
    }
    state.active = active;

    if active {
        let app_handle = app.clone();
        let meter = Arc::new(Mutex::new(MusicBandMeter::new()));
        let meter_capture = Arc::clone(&meter);
        let mut last_emit = Instant::now();

        let capture = audio::Capture::start(audio::Source::System, move |samples| {
            let mut m = meter_capture.lock().unwrap();
            if m.feed(samples) {
                let now = Instant::now();
                // Throttle to at most ~40 FPS (25ms) to be ultra-efficient
                if now.duration_since(last_emit) >= Duration::from_millis(25) {
                    last_emit = now;
                    let levels_vec = m.levels.to_vec();
                    let _ = app_handle.emit("music-bands", levels_vec);
                }
            }
        });

        match capture {
            Ok(c) => {
                state.capture = Some(c);
            }
            Err(e) => {
                crate::log::line(format!("visualizer: failed to start loopback capture: {e}"));
                state.capture = None;
            }
        }
    } else {
        if let Some(mut c) = state.capture.take() {
            c.stop();
        }
        // Emit zeroed levels so UI drops down to resting state
        let zero = vec![0.0f32; BANDS_COUNT];
        let _ = app.emit("music-bands", zero);
    }
}

#[tauri::command]
pub fn music_visualizer_set_active(app: AppHandle, active: bool) {
    set_active(&app, active);
}
