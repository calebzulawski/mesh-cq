use clap::Parser;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepeaterState {
    Idle,
    MidConversation,
}

struct TransmitResult {
    sent_callsign: bool,
    transmission_end: Option<cpal::StreamInstant>,
}

#[derive(Parser, Debug)]
#[command(name = "meshcq-simplex-repeater", about = "Simplex repeater with CW ID")]
struct Args {
    /// Callsign to transmit after each message.
    callsign: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let (input_tx, input_rx) = std::sync::mpsc::channel();
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    let _output = meshcq_modem::device::start_default_output(output_rx)?;
    let _input = meshcq_modem::device::start_default_input(input_tx)?;

    let level = 10.0_f32.powf(-CW_LEVEL_DB_DOWN / 20.0);
    let callsign_samples = callsign::pre_modulate_callsign(
        &args.callsign,
        SAMPLE_RATE_HZ,
        TONE_FREQ_HZ,
        WPM,
        level,
    )?;

    let mut last_id: Option<cpal::StreamInstant> = None;
    let mut last_message_end: Option<cpal::StreamInstant> = None;
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
                    let idle = std::time::Duration::from_secs(ID_IDLE_SECS);
                    if let Some(now) = end.add(idle) {
                        let len = transmit_callsign(&callsign_samples, &output_tx);
                        last_id = now.add(transmit_duration(len));
                    }
                }
                state = RepeaterState::Idle;
                continue;
            }
        };

        last_message_end = Some(message.end);
        let result = transmit_message(
            message,
            &callsign_samples,
            &output_tx,
            last_id,
        );
        if result.sent_callsign {
            last_id = result.transmission_end;
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
    let mut last_end = first.end;

    loop {
        match input_rx.recv_timeout(std::time::Duration::from_secs_f32(CONTINUITY_GAP_SECS)) {
            Ok(next) => {
                let next_duration = std::time::Duration::from_secs_f64(
                    next.samples.len() as f64 / SAMPLE_RATE_HZ as f64,
                );
                let next_start = next.end.sub(next_duration).unwrap_or(next.end);
                let gap = next_start
                    .duration_since(&last_end)
                    .unwrap_or_else(|| std::time::Duration::from_secs(0));
                let gap_samples = (gap.as_secs_f64() * SAMPLE_RATE_HZ as f64).round() as usize;
                combined.extend(std::iter::repeat_n(0.0, gap_samples));
                combined.extend(next.samples);
                last_end = next.end;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(Some(TimedChunk {
        samples: combined,
        end: last_end,
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
    last_id: Option<cpal::StreamInstant>,
) -> TransmitResult {
    let message_end = message.end;
    let id_due = last_id.and_then(|last| last.add(std::time::Duration::from_secs(ID_INTERVAL_SECS)));

    let base_len = transmit_len(message.samples.len(), callsign_samples.len(), false);
    let base_duration = transmit_duration(base_len);
    let will_expire = match (id_due, message_end.add(base_duration)) {
        (Some(due), Some(end)) => end.duration_since(&due).is_some(),
        (Some(_), None) => true,
        (None, _) => true,
    };

    let out = build_transmit_message(&message.samples, callsign_samples, will_expire);
    let out_len = out.len();
    let _ = output_tx.send(out);

    TransmitResult {
        sent_callsign: will_expire,
        transmission_end: message_end.add(transmit_duration(out_len)),
    }
}

fn transmit_duration(samples: usize) -> std::time::Duration {
    std::time::Duration::from_secs_f32(samples as f32 / SAMPLE_RATE_HZ)
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
