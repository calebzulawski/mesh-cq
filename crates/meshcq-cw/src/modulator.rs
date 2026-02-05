use crate::sine_oscillator::SineOscillator;

/// Modulates Morse units into audio samples.
pub struct CwModulator {
    unit_samples: usize,
    osc: SineOscillator,
    level: f32,
}

impl CwModulator {
    /// Create a CW modulator for the given sample rate, tone frequency, WPM, and level.
    pub fn new(sample_rate_hz: f32, tone_freq_hz: f32, wpm: f32, level: f32) -> Self {
        // PARIS standard: 50 units per word.
        // One word duration (seconds) = 60 / WPM, so one unit = (60 / WPM) / 50.
        let unit_seconds = 60.0 / (wpm * 50.0);
        let unit_samples = (sample_rate_hz * unit_seconds).round() as usize;

        Self {
            unit_samples: unit_samples.max(1),
            osc: SineOscillator::new(sample_rate_hz, tone_freq_hz),
            level,
        }
    }

    /// Fill a buffer with audio samples from the provided Morse units.
    /// Returns the number of samples written (always a multiple of unit samples).
    pub fn modulate<I>(&mut self, units: &mut I, out: &mut [f32]) -> usize
    where
        I: Iterator<Item = bool>,
    {
        let mut offset = 0;
        while offset + self.unit_samples <= out.len() {
            let gate = match units.next() {
                Some(value) => value,
                None => break,
            };

            for sample in &mut out[offset..offset + self.unit_samples] {
                if gate {
                    *sample = self.osc.next() * self.level;
                } else {
                    self.osc.advance(1);
                    *sample = 0.0;
                }
            }

            offset += self.unit_samples;
        }

        offset
    }

    /// Reset the oscillator phase.
    pub fn reset_phase(&mut self) {
        self.osc.reset();
    }

    /// Return the number of samples per Morse unit.
    pub fn unit_samples(&self) -> usize {
        self.unit_samples
    }
}
