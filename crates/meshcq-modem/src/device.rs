use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use regex::Regex;
use ringbuf::HeapRb;
use std::collections::VecDeque;
use std::sync::{mpsc::Receiver, mpsc::Sender};

const ENERGY_BLOCK: usize = 1024;
const CONCAT_BLOCKS: usize = 3;
const ENERGY_THRESHOLD: f32 = 1.0e-4;
const OUTPUT_RING_CAP: usize = 48_000 * 4;

pub struct TimedChunk {
    pub samples: Vec<f32>,
    pub end_sample: u64,
}

pub fn start_default_input(
    left_tx: Sender<TimedChunk>,
    device_regex: Option<&str>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = select_input_device(&host, device_regex)?;
    let default_config = device.default_input_config()?;
    let mut sample_format = default_config.sample_format();
    let mut config = default_config;
    if sample_format == cpal::SampleFormat::F32 {
        if let Ok(mut supported) = device.supported_input_configs() {
            if let Some(best) = supported.find(|cfg| {
                cfg.sample_format() == cpal::SampleFormat::F32
                    && cfg.min_sample_rate().0 <= 48_000
                    && cfg.max_sample_rate().0 >= 48_000
            }) {
                config = best.with_sample_rate(cpal::SampleRate(48_000));
                sample_format = config.sample_format();
            }
        }
    }
    let config: cpal::StreamConfig = config.into();
    assert!(
        config.sample_rate.0 == 48_000,
        "expected 48 kHz sample rate, got {} Hz",
        config.sample_rate.0
    );
    let channels = config.channels as usize;
    let mut block: Vec<f32> = Vec::with_capacity(ENERGY_BLOCK);
    let mut block_queue: VecDeque<Vec<f32>> = VecDeque::with_capacity(CONCAT_BLOCKS);
    let mut capture: Vec<f32> = Vec::with_capacity(ENERGY_BLOCK * CONCAT_BLOCKS);
    let mut message: Vec<f32> = Vec::new();

    let err_fn = |err| eprintln!("audio stream error: {}", err);
    let mut sample_cursor: u64 = 0;

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _info| {
                let frames = data.len() / channels;
                sample_cursor = sample_cursor.saturating_add(frames as u64);
                let buffer_end = sample_cursor;

                let mut process_block = |block: &mut Vec<f32>| {
                    let energy = block.iter().map(|x| x * x).sum::<f32>()
                        / ENERGY_BLOCK as f32;

                    block_queue.push_back(block.clone());
                    if block_queue.len() > CONCAT_BLOCKS {
                        block_queue.pop_front();
                    }

                    if energy > ENERGY_THRESHOLD && block_queue.len() == CONCAT_BLOCKS {
                        capture.clear();
                        for queued in block_queue.iter() {
                            capture.extend_from_slice(queued);
                        }
                        // TODO: process capture (len = ENERGY_BLOCK * CONCAT_BLOCKS).
                    }

                    if energy > ENERGY_THRESHOLD {
                        message.extend_from_slice(block);
                    } else if !message.is_empty() {
                        let to_send = std::mem::take(&mut message);
                        eprintln!("audio input: message captured ({} samples)", to_send.len());
                        let _ = left_tx.send(TimedChunk {
                            samples: to_send,
                            end_sample: buffer_end,
                        });
                    }

                    block.clear();
                };

                if channels == 1 {
                    let mut offset = 0;
                    while offset < data.len() {
                        let need = ENERGY_BLOCK - block.len();
                        let take = need.min(data.len() - offset);
                        block.extend_from_slice(&data[offset..offset + take]);
                        offset += take;
                        if block.len() >= ENERGY_BLOCK {
                            process_block(&mut block);
                        }
                    }
                } else {
                    for frame in data.chunks(channels) {
                        block.push(frame[0]);
                        if block.len() >= ENERGY_BLOCK {
                            process_block(&mut block);
                        }
                    }
                }
            },
            err_fn,
            None,
        )?,
        _ => return Err("unsupported sample format (expected f32)".into()),
    };

    stream.play()?;
    Ok(stream)
}

pub fn start_default_output(
    left_rx: Receiver<Vec<f32>>,
    output_level: f32,
    device_regex: Option<&str>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = select_output_device(&host, device_regex)?;
    let config = device.default_output_config()?;
    let sample_format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let channels = config.channels as usize;
    if channels < 2 {
        return Err("output device must support at least 2 channels".into());
    }

    let ring = HeapRb::<f32>::new(OUTPUT_RING_CAP);
    let (mut producer, mut consumer) = ring.split();
    let mut phase: f32 = 0.0;
    let phase_inc: f32 = std::f32::consts::TAU * 1000.0 / 48_000.0;

    let err_fn = |err| eprintln!("audio stream error: {}", err);

    let mut pending: VecDeque<f32> = VecDeque::new();

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _| {
                while let Some(sample) = pending.front().copied() {
                    if producer.push(sample).is_ok() {
                        pending.pop_front();
                    } else {
                        break;
                    }
                }

                while let Ok(chunk) = left_rx.try_recv() {
                    eprintln!("audio output: received {} samples", chunk.len());
                    for sample in chunk {
                        pending.push_back(sample);
                    }
                }

                for frame in data.chunks_mut(channels) {
                    let left_opt = consumer.pop();
                    let left = left_opt.unwrap_or(0.0) * output_level;
                    let right = if left_opt.is_some() {
                        let tone = phase.sin();
                        phase += phase_inc;
                        if phase >= std::f32::consts::TAU {
                            phase -= std::f32::consts::TAU;
                        }
                        tone
                    } else {
                        0.0
                    };

                    frame[0] = left;
                    frame[1] = right;
                    for chan in frame.iter_mut().skip(2) {
                        *chan = 0.0;
                    }
                }
            },
            err_fn,
            None,
        )?,
        _ => return Err("unsupported sample format (expected f32)".into()),
    };

    stream.play()?;
    Ok(stream)
}

fn select_input_device(
    host: &cpal::Host,
    device_regex: Option<&str>,
) -> Result<cpal::Device, Box<dyn std::error::Error>> {
    if let Some(pattern) = device_regex {
        let re = Regex::new(pattern)?;
        for dev in host.input_devices()? {
            let name = dev.name().unwrap_or_else(|_| "<unknown>".to_string());
            if re.is_match(&name) {
                return Ok(dev);
            }
        }
        return Err("no input device matched regex".into());
    }

    host.default_input_device()
        .ok_or_else(|| "no default input device available".into())
}

fn select_output_device(
    host: &cpal::Host,
    device_regex: Option<&str>,
) -> Result<cpal::Device, Box<dyn std::error::Error>> {
    if let Some(pattern) = device_regex {
        let re = Regex::new(pattern)?;
        for dev in host.output_devices()? {
            let name = dev.name().unwrap_or_else(|_| "<unknown>".to_string());
            if re.is_match(&name) {
                return Ok(dev);
            }
        }
        return Err("no output device matched regex".into());
    }

    host.default_output_device()
        .ok_or_else(|| "no default output device available".into())
}
