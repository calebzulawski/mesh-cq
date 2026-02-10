pub fn estimate_floor(samples: &[f32], ranges: &[(usize, usize)], window_len: usize) -> f32 {
    if window_len == 0 {
        return 0.0;
    }
    let mut min_rms = None;
    let mut pos = 0usize;
    for &(start, end) in ranges {
        if pos < start {
            min_rms = min_rms_segment(samples, pos, start, window_len, min_rms);
        }
        pos = end;
    }
    if pos < samples.len() {
        min_rms = min_rms_segment(samples, pos, samples.len(), window_len, min_rms);
    }
    min_rms.unwrap_or(0.0)
}

pub fn fill_band_limited_gaussian_noise(
    samples: &mut [f32],
    level: f32,
    sample_rate_hz: f32,
    cutoff_hz: f32,
) {
    let mut rng = XorShift32::new(0x1234_5678);
    let mut filt = OnePoleLowpass::new(sample_rate_hz, cutoff_hz);
    let mut i = 0;
    while i < samples.len() {
        let u1 = rng.next_f32().max(1e-12);
        let u2 = rng.next_f32();
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = 2.0 * std::f32::consts::PI * u2;
        let z0 = r * theta.cos();
        let z1 = r * theta.sin();
        let y0 = filt.process(z0 * level);
        samples[i] = y0;
        i += 1;
        if i < samples.len() {
            let y1 = filt.process(z1 * level);
            samples[i] = y1;
            i += 1;
        }
    }
}

struct XorShift32 {
    state: u32,
}

impl XorShift32 {
    fn new(seed: u32) -> Self {
        let state = if seed == 0 { 0xA5A5_1234 } else { seed };
        Self { state }
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    fn next_f32(&mut self) -> f32 {
        let v = self.next_u32();
        (v as f32) / (u32::MAX as f32)
    }
}

struct OnePoleLowpass {
    alpha: f32,
    z: f32,
}

impl OnePoleLowpass {
    fn new(sample_rate_hz: f32, cutoff_hz: f32) -> Self {
        let dt = 1.0 / sample_rate_hz;
        let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz.max(1.0));
        let alpha = dt / (rc + dt);
        Self { alpha, z: 0.0 }
    }

    fn process(&mut self, x: f32) -> f32 {
        self.z += self.alpha * (x - self.z);
        self.z
    }
}
fn min_rms_segment(
    samples: &[f32],
    start: usize,
    end: usize,
    window_len: usize,
    mut current_min: Option<f32>,
) -> Option<f32> {
    if end <= start || end - start < window_len {
        return current_min;
    }
    let mut idx = start;
    while idx + window_len <= end {
        let mut sum_sq = 0.0f64;
        for &s in &samples[idx..idx + window_len] {
            sum_sq += (s as f64) * (s as f64);
        }
        let rms = (sum_sq / window_len as f64).sqrt() as f32;
        current_min = Some(match current_min {
            Some(min) => min.min(rms),
            None => rms,
        });
        idx += window_len;
    }
    current_min
}
