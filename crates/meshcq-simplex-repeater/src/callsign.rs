use meshcq_cw::{encode_units, CwModulator, EncodeError};

pub fn pre_modulate_callsign(
    callsign: &str,
    sample_rate_hz: f32,
    tone_freq_hz: f32,
    wpm: f32,
    level: f32,
) -> Result<Vec<f32>, EncodeError> {
    let units = encode_units(callsign)?;
    let mut modulator = CwModulator::new(sample_rate_hz, tone_freq_hz, wpm, level);
    let unit_samples = modulator.unit_samples();
    let mut out = vec![0.0f32; units.len() * unit_samples];
    let mut iter = units.iter().by_vals();
    let written = modulator.modulate(&mut iter, &mut out);
    out.truncate(written);
    Ok(out)
}
