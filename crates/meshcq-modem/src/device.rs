use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use ringbuf::HeapRb;
use std::sync::{mpsc::Receiver, mpsc::Sender};
use std::thread;

const ENERGY_BLOCK: usize = 1024;
const CONCAT_BLOCKS: usize = 3;
const ENERGY_THRESHOLD: f32 = 1.0e-4;
const OUTPUT_RING_CAP: usize = 48_000 * 4;

pub fn start_default_input_thread(left_tx: Sender<Vec<f32>>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(err) = run_default_input(left_tx) {
            eprintln!("audio input thread error: {}", err);
        }
    })
}

fn run_default_input(left_tx: Sender<Vec<f32>>) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or("no default input device available")?;
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

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| {
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
                        let _ = left_tx.send(to_send);
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
    std::thread::park();
    Ok(stream)
}

pub fn start_default_output_thread(left_rx: Receiver<Vec<f32>>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(err) = run_default_output(left_rx) {
            eprintln!("audio output thread error: {}", err);
        }
    })
}

fn run_default_output(left_rx: Receiver<Vec<f32>>) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no default output device available")?;
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

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _| {
                for frame in data.chunks_mut(channels) {
                    let left_opt = consumer.pop();
                    let left = left_opt.unwrap_or(0.0);
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
    loop {
        let chunk = left_rx.recv()?;
        for sample in chunk {
            let _ = producer.push(sample);
        }
    }
}
