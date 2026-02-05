use bitvec::vec::BitVec;
use clap::Parser;
use meshcq_cw::{encode_units, CwModulator};

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const TONE_FREQ_HZ: f32 = 700.0;
const WPM: f32 = 20.0;
const PRE_CALLSIGN_GAP_SECS: f32 = 1.0;
const CW_LEVEL_DB_DOWN: f32 = 20.0;

#[derive(Parser, Debug)]
#[command(name = "meshcq-simplex-repeater", about = "Simplex repeater with CW ID")]
struct Args {
    /// Callsign to transmit after each message.
    callsign: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let units = encode_units(&args.callsign)?;

    let (input_tx, input_rx) = std::sync::mpsc::channel();
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    let _output = meshcq_modem::device::start_default_output(output_rx)?;
    let _input = meshcq_modem::device::start_default_input(input_tx)?;

    let level = 10.0_f32.powf(-CW_LEVEL_DB_DOWN / 20.0);
    let mut modulator = CwModulator::new(SAMPLE_RATE_HZ, TONE_FREQ_HZ, WPM, level);

    loop {
        let message = input_rx.recv()?;
        let _ = output_tx.send(message);

        let gap_samples = (SAMPLE_RATE_HZ * PRE_CALLSIGN_GAP_SECS).round() as usize;
        let _ = output_tx.send(vec![0.0; gap_samples]);

        modulator.reset_phase();
        let cw_samples = modulate_callsign(&mut modulator, &units);
        let _ = output_tx.send(cw_samples);
    }
}

fn modulate_callsign(modulator: &mut CwModulator, units: &BitVec) -> Vec<f32> {
    let unit_samples = modulator.unit_samples();
    let mut out = vec![0.0f32; units.len() * unit_samples];
    let mut iter = units.iter().by_vals();
    let written = modulator.modulate(&mut iter, &mut out);
    out.truncate(written);
    out
}
