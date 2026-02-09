use clap::Parser;
use meshcq_dtmf::DtmfDebouncer;

mod callsign;
use meshcq_modem::device::TimedChunk;

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const TONE_FREQ_HZ: f32 = 700.0;
const WPM: f32 = 20.0;
const PRE_CALLSIGN_GAP_SECS: f32 = 1.0;
const CW_LEVEL_DB_DOWN: f32 = 20.0;
const ID_INTERVAL_SECS: u64 = 9 * 60;
const ID_IDLE_SECS: u64 = 30;
const CONTINUITY_GAP_SECS: f32 = 1.0;
const TX_LEAD_TIME_SECS: f32 = 0.2;
const TX_HANG_TIME_SECS: f32 = 1.0;
const DEFAULT_OUTPUT_LEVEL: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepeaterState {
    Idle,
    MidConversation,
}

struct TransmitResult {
    sent_callsign: bool,
    transmission_end_sample: u64,
}

#[derive(Parser, Debug)]
#[command(name = "meshcq-simplex-repeater", about = "Simplex repeater with CW ID")]
struct Args {
    /// Callsign to transmit after each message.
    callsign: String,
    /// Output level multiplier (0.0 - 1.0).
    #[arg(long, default_value_t = DEFAULT_OUTPUT_LEVEL)]
    output_level: f32,
    /// Regex to select input/output device by name.
    #[arg(long)]
    sound_device: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let (input_tx, input_rx) = std::sync::mpsc::channel();
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    let device_regex = args.sound_device.as_deref();
    let _output =
        meshcq_modem::device::start_default_output(output_rx, args.output_level, device_regex)?;
    let _input = meshcq_modem::device::start_default_input(input_tx, device_regex)?;

    let level = 10.0_f32.powf(-CW_LEVEL_DB_DOWN / 20.0);
    let callsign_samples = callsign::pre_modulate_callsign(
        &args.callsign,
        SAMPLE_RATE_HZ,
        TONE_FREQ_HZ,
        WPM,
        level,
    )?;

    let mut dtmf = DtmfDebouncer::builder(SAMPLE_RATE_HZ).build();

    let mut last_id: Option<u64> = None;
    let mut last_message_end: Option<u64> = None;
    let mut state = RepeaterState::Idle;

    loop {
        let timeout = match state {
            RepeaterState::Idle => None,
            RepeaterState::MidConversation => Some(std::time::Duration::from_secs(ID_IDLE_SECS)),
        };
        let message = read_message(&input_rx, timeout)?;

        let message = match message {
            Some(message) => message,
            None => {
                if let Some(end) = last_message_end {
                    let now = end.saturating_add(samples_from_secs(ID_IDLE_SECS as f32));
                    let len = transmit_callsign(&callsign_samples, &output_tx);
                    last_id = Some(now.saturating_add(len as u64));
                }
                state = RepeaterState::Idle;
                continue;
            }
        };

        for (ch, _, _) in dtmf.push(&message.samples) {
            eprintln!("dtmf: {}", ch);
        }

        last_message_end = Some(message.end_sample);
        let result = transmit_message(
            message,
            &callsign_samples,
            &output_tx,
            last_id,
        );
        if result.sent_callsign {
            last_id = Some(result.transmission_end_sample);
            state = RepeaterState::Idle;
        } else {
            state = RepeaterState::MidConversation;
        }
    }
}

fn read_message(
    input_rx: &std::sync::mpsc::Receiver<TimedChunk>,
    first_timeout: Option<std::time::Duration>,
) -> Result<Option<TimedChunk>, Box<dyn std::error::Error>> {
    let first = match first_timeout {
        Some(timeout) => match input_rx.recv_timeout(timeout) {
            Ok(message) => message,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Ok(None),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("input channel disconnected".into());
            }
        },
        None => input_rx.recv()?,
    };

    let mut combined = first.samples;
    let mut last_end = first.end_sample;

    loop {
        match input_rx.recv_timeout(std::time::Duration::from_secs_f32(CONTINUITY_GAP_SECS)) {
            Ok(next) => {
                let next_start = next.end_sample.saturating_sub(next.samples.len() as u64);
                let gap_samples = next_start.saturating_sub(last_end) as usize;
                combined.extend(std::iter::repeat_n(0.0, gap_samples));
                combined.extend(next.samples);
                last_end = next.end_sample;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(Some(TimedChunk {
        samples: combined,
        end_sample: last_end,
    }))
}

fn transmit_callsign(
    callsign_samples: &[f32],
    output_tx: &std::sync::mpsc::Sender<Vec<f32>>,
) -> usize {
    let out = build_transmit_message(&[], callsign_samples, true);
    let out_len = out.len();
    let _ = output_tx.send(out);
    out_len
}

fn transmit_message(
    message: TimedChunk,
    callsign_samples: &[f32],
    output_tx: &std::sync::mpsc::Sender<Vec<f32>>,
    last_id: Option<u64>,
) -> TransmitResult {
    let message_end = message.end_sample;
    let id_due = last_id.map(|last| last.saturating_add(samples_from_secs(ID_INTERVAL_SECS as f32)));

    let base_len = transmit_len(message.samples.len(), callsign_samples.len(), false);
    let will_expire = match id_due {
        Some(due) => message_end.saturating_add(base_len as u64) >= due,
        None => true,
    };

    let out = build_transmit_message(&message.samples, callsign_samples, will_expire);
    let out_len = out.len();
    let _ = output_tx.send(out);

    TransmitResult {
        sent_callsign: will_expire,
        transmission_end_sample: message_end.saturating_add(out_len as u64),
    }
}

fn samples_from_secs(secs: f32) -> u64 {
    (SAMPLE_RATE_HZ * secs).round() as u64
}

fn transmit_len(message_len: usize, callsign_len: usize, include_callsign: bool) -> usize {
    let lead_samples = (SAMPLE_RATE_HZ * TX_LEAD_TIME_SECS).round() as usize;
    let hang_samples = (SAMPLE_RATE_HZ * TX_HANG_TIME_SECS).round() as usize;
    let gap_samples = if include_callsign {
        (SAMPLE_RATE_HZ * PRE_CALLSIGN_GAP_SECS).round() as usize
    } else {
        0
    };
    lead_samples
        + message_len
        + gap_samples
        + if include_callsign { callsign_len } else { 0 }
        + hang_samples
}

fn build_transmit_message(
    message: &[f32],
    callsign_samples: &[f32],
    include_callsign: bool,
) -> Vec<f32> {
    let lead_samples = (SAMPLE_RATE_HZ * TX_LEAD_TIME_SECS).round() as usize;
    let hang_samples = (SAMPLE_RATE_HZ * TX_HANG_TIME_SECS).round() as usize;
    let gap_samples = if include_callsign {
        (SAMPLE_RATE_HZ * PRE_CALLSIGN_GAP_SECS).round() as usize
    } else {
        0
    };

    let mut out = Vec::with_capacity(transmit_len(
        message.len(),
        callsign_samples.len(),
        include_callsign,
    ));
    out.extend(std::iter::repeat_n(0.0, lead_samples));
    out.extend_from_slice(message);
    if include_callsign {
        out.extend(std::iter::repeat_n(0.0, gap_samples));
        out.extend_from_slice(callsign_samples);
    }
    out.extend(std::iter::repeat_n(0.0, hang_samples));
    out
}
