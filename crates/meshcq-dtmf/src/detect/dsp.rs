const DTMF_FREQS: [f32; 8] = [697.0, 770.0, 852.0, 941.0, 1209.0, 1336.0, 1477.0, 1633.0];
const TOTAL_BINS: usize = 8;

const DTMF_KEYS: [[char; 4]; 4] = [
    ['1', '2', '3', 'A'],
    ['4', '5', '6', 'B'],
    ['7', '8', '9', 'C'],
    ['*', '0', '#', 'D'],
];

const DEFAULT_PEAK_RATIO: f32 = 6.0;
const DEFAULT_TWIST_DB: f32 = 12.0;

/// DTMF detector using the Goertzel algorithm.
pub struct DtmfDetector {
    n: usize,
    coeffs: [f32; TOTAL_BINS],
    peak_ratio: f32,
    twist_db: f32,
    s1: [f32; TOTAL_BINS],
    s2: [f32; TOTAL_BINS],
    samples_seen: usize,
}

impl DtmfDetector {
    /// Create a detector with default thresholds.
    pub fn new(sample_rate_hz: f32, n: usize) -> Self {
        Self::with_thresholds(sample_rate_hz, n, DEFAULT_PEAK_RATIO, DEFAULT_TWIST_DB)
    }

    /// Create a detector with custom peak ratio and twist thresholds.
    pub fn with_thresholds(
        sample_rate_hz: f32,
        n: usize,
        peak_ratio: f32,
        twist_db: f32,
    ) -> Self {
        let coeffs = goertzel_coeffs(sample_rate_hz, DTMF_FREQS);
        Self {
            n: n.max(1),
            coeffs,
            peak_ratio,
            twist_db,
            s1: [0.0; TOTAL_BINS],
            s2: [0.0; TOTAL_BINS],
            samples_seen: 0,
        }
    }

    /// Reset internal state for a new accumulation window.
    pub fn reset(&mut self) {
        self.s1 = [0.0; TOTAL_BINS];
        self.s2 = [0.0; TOTAL_BINS];
        self.samples_seen = 0;
    }

    /// Feed samples into the detector accumulators.
    pub fn feed(&mut self, samples: &[f32]) {
        if self.samples_seen >= self.n {
            return;
        }
        let remaining = self.n - self.samples_seen;
        let samples = &samples[..samples.len().min(remaining)];
        for &x in samples {
            for i in 0..TOTAL_BINS {
                let s0 = x + self.coeffs[i] * self.s1[i] - self.s2[i];
                self.s2[i] = self.s1[i];
                self.s1[i] = s0;
            }
            self.samples_seen += 1;
        }
    }

    /// Finalize the current accumulator and return the detected tone, if any.
    pub fn finish(&self) -> Option<char> {
        let mags = goertzel_finish(self.s1, self.s2, self.coeffs);
        let (low_i, low_peak, low_next) = top_two(&mags[..4])?;
        let (high_i, high_peak, high_next) = top_two(&mags[4..])?;

        if low_peak < low_next * self.peak_ratio || high_peak < high_next * self.peak_ratio {
            return None;
        }

        if !twist_ok(low_peak, high_peak, self.twist_db) {
            return None;
        }

        Some(DTMF_KEYS[low_i][high_i])
    }

    /// Convenience helper for one-shot detection over a single slice.
    pub fn detect_frame(&mut self, samples: &[f32]) -> Option<char> {
        if samples.len() != self.n {
            return None;
        }
        self.reset();
        self.feed(samples);
        self.finish()
    }
}

fn goertzel_coeffs(sample_rate_hz: f32, freqs: [f32; 8]) -> [f32; TOTAL_BINS] {
    std::array::from_fn(|i| {
        let freq_hz = freqs[i];
        let omega = 2.0 * std::f32::consts::PI * freq_hz / sample_rate_hz;
        2.0 * omega.cos()
    })
}

fn goertzel_finish<const N: usize>(s1: [f32; N], s2: [f32; N], coeffs: [f32; N]) -> [f32; N] {
    std::array::from_fn(|i| s1[i] * s1[i] + s2[i] * s2[i] - coeffs[i] * s1[i] * s2[i])
}

fn top_two(values: &[f32]) -> Option<(usize, f32, f32)> {
    let mut max_i = 0;
    let mut max_v = values.get(0).copied()?;
    let mut next_v = f32::MIN;

    for (i, &v) in values.iter().enumerate().skip(1) {
        if v > max_v {
            next_v = max_v;
            max_v = v;
            max_i = i;
        } else if v > next_v {
            next_v = v;
        }
    }

    if next_v == f32::MIN {
        return None;
    }
    Some((max_i, max_v, next_v))
}

fn twist_ok(low_peak: f32, high_peak: f32, twist_db: f32) -> bool {
    if low_peak <= 0.0 || high_peak <= 0.0 {
        return false;
    }
    let ratio = (high_peak / low_peak).log10().abs();
    let db = 10.0 * ratio;
    db <= twist_db
}
