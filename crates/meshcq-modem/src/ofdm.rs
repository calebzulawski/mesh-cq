//! OFDM modulation utilities.

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// OFDM modulator using a 2048-point IFFT with 104 active subcarriers.
pub struct OfdmModulator {
    nfft: usize,
    active_bins: usize,
    cp_len: usize,
}

impl Default for OfdmModulator {
    fn default() -> Self {
        Self {
            nfft: 2048,
            active_bins: 104,
            cp_len: 256,
        }
    }
}

impl OfdmModulator {
    /// Create a modulator with the default 2048-point FFT and 104 active subcarriers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Modulate one OFDM symbol.
    ///
    /// `data` must contain exactly 104 complex subcarrier symbols. These are mapped to
    /// the lowest positive-frequency bins (1..=104), leaving DC (bin 0) unused.
    /// The remaining bins are zeroed. The output is the 2048-sample time-domain
    /// complex baseband symbol with a 256-sample cyclic prefix prepended.
    pub fn modulate(&self, data: &[Complex<f32>]) -> Result<Vec<Complex<f32>>, String> {
        if data.len() != self.active_bins {
            return Err(format!(
                "expected {} subcarriers, got {}",
                self.active_bins,
                data.len()
            ));
        }

        let mut freq_bins = vec![Complex::new(0.0, 0.0); self.nfft];
        for (i, sym) in data.iter().enumerate() {
            freq_bins[i + 1] = *sym;
        }

        let mut planner = FftPlanner::<f32>::new();
        let ifft = planner.plan_fft_inverse(self.nfft);
        ifft.process(&mut freq_bins);

        // Normalize: rustfft's inverse is unscaled.
        let scale = 1.0 / self.nfft as f32;
        for bin in &mut freq_bins {
            *bin *= scale;
        }

        let mut with_cp = Vec::with_capacity(self.nfft + self.cp_len);
        with_cp.extend_from_slice(&freq_bins[self.nfft - self.cp_len..]);
        with_cp.extend_from_slice(&freq_bins);
        Ok(with_cp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modulator_rejects_wrong_length() {
        let modem = OfdmModulator::new();
        let data = vec![Complex::new(1.0, 0.0); 10];
        assert!(modem.modulate(&data).is_err());
    }

    #[test]
    fn modulator_zero_input_is_zero_output() {
        let modem = OfdmModulator::new();
        let data = vec![Complex::new(0.0, 0.0); 104];
        let out = modem.modulate(&data).expect("modulate");
        assert_eq!(out.len(), 2048 + 256);
        assert!(out.iter().all(|v| v.re == 0.0 && v.im == 0.0));
    }
}
