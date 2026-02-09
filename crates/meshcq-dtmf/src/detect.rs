pub mod dsp;

use dsp::DtmfDetector;

const DEFAULT_FRAME_MS: f32 = 30.0;
const DEFAULT_MIN_PRESS_FRAMES: usize = 2;
const DEFAULT_MIN_GAP_FRAMES: usize = 3;

/// Stateful DTMF debouncer for sequential frames.
pub struct DtmfDebouncer {
    frame_len: usize,
    min_press_frames: usize,
    min_gap_frames: usize,
    samples_in_frame: usize,
    current: Option<char>,
    gap_frames: usize,
    current_start: u64,
    current_last: u64,
    current_history: Vec<char>,
    detector: DtmfDetector,
}

impl DtmfDebouncer {
    /// Create a builder with default settings.
    pub fn builder(sample_rate_hz: f32) -> DtmfDebouncerBuilder {
        DtmfDebouncerBuilder::new(sample_rate_hz)
    }

    /// Feed samples and return detected key events.
    /// Each event is (char, start_sample, end_sample).
    pub fn push(&mut self, samples: &[f32]) -> Vec<(char, usize, usize)> {
        let mut events = Vec::new();

        let mut pos = 0usize;
        while pos < samples.len() {
            let to_frame_end = self.frame_len - self.samples_in_frame;
            let take = to_frame_end.min(samples.len() - pos);
            let chunk = &samples[pos..pos + take];
            self.detector.feed(chunk);
            self.samples_in_frame += take;

            if self.samples_in_frame == self.frame_len {
                let detected = self.detector.finish();
                self.detector.reset();
                let frame_end = (pos + take) as u64 - 1;
                let frame_start = frame_end + 1 - self.frame_len as u64;
                self.consume_frame(detected, frame_start, frame_end, &mut events);
                self.samples_in_frame = 0;
            }

            pos += take;
        }

        events
    }

    /// Reset internal state and clear any pending detections.
    pub fn reset(&mut self) {
        self.samples_in_frame = 0;
        self.current = None;
        self.gap_frames = 0;
        self.current_start = 0;
        self.current_last = 0;
        self.current_history.clear();
        self.detector.reset();
    }

    fn consume_frame(
        &mut self,
        detected: Option<char>,
        frame_start: u64,
        frame_end: u64,
        events: &mut Vec<(char, usize, usize)>,
    ) {
        if let Some(ch) = detected {
            if self.current.is_none() {
                // Start tracking a new key.
                self.current = Some(ch);
                self.current_start = frame_start;
            }
            // Accumulate detected frames until release.
            self.current_history.push(ch);
            self.current_last = frame_end;
            // Reset release debounce.
            self.gap_frames = 0;
            return;
        }

        if self.current.is_some() {
            self.gap_frames += 1;
            if self.gap_frames >= self.min_gap_frames {
                if self.current_history.len() >= self.min_press_frames {
                    // Commit after a sufficient release gap.
                    if let Some(resolved) = most_common_key(&self.current_history) {
                        events.push((
                            resolved,
                            self.current_start as usize,
                            self.current_last as usize,
                        ));
                    }
                }
                // Clear current key after release.
                self.current = None;
                self.gap_frames = 0;
                self.current_history.clear();
            }
        }
    }
}

fn ms_to_samples(ms: f32, sample_rate_hz: f32) -> usize {
    let len = (sample_rate_hz * (ms / 1000.0)).round() as usize;
    len.max(1)
}

/// Builder for configuring a DtmfDebouncer.
pub struct DtmfDebouncerBuilder {
    sample_rate_hz: f32,
    frame_samples: usize,
    min_press_frames: usize,
    min_gap_frames: usize,
    detector: Option<DtmfDetector>,
}

impl DtmfDebouncerBuilder {
    /// Create a builder with defaults for the given sample rate.
    pub fn new(sample_rate_hz: f32) -> Self {
        Self {
            sample_rate_hz,
            frame_samples: ms_to_samples(DEFAULT_FRAME_MS, sample_rate_hz),
            min_press_frames: DEFAULT_MIN_PRESS_FRAMES,
            min_gap_frames: DEFAULT_MIN_GAP_FRAMES,
            detector: None,
        }
    }

    /// Set the frame length in milliseconds.
    pub fn frame_ms(mut self, frame_ms: f32) -> Self {
        self.frame_samples = ms_to_samples(frame_ms, self.sample_rate_hz);
        self
    }

    /// Set the frame length in samples.
    pub fn frame_samples(mut self, frame_samples: usize) -> Self {
        self.frame_samples = frame_samples.max(1);
        self
    }

    /// Set the minimum number of frames required to accept a key.
    pub fn min_press_frames(mut self, frames: usize) -> Self {
        self.min_press_frames = frames.max(1);
        self
    }

    /// Set the minimum number of empty frames required to release a key.
    pub fn min_gap_frames(mut self, frames: usize) -> Self {
        self.min_gap_frames = frames.max(1);
        self
    }

    /// Provide a custom detector instance.
    pub fn detector(mut self, detector: DtmfDetector) -> Self {
        self.detector = Some(detector);
        self
    }

    /// Build the debouncer.
    pub fn build(self) -> DtmfDebouncer {
        let detector = self.detector.unwrap_or_else(|| {
            DtmfDetector::new(self.sample_rate_hz, self.frame_samples.max(1))
        });

        let frame_len = self.frame_samples.max(1);

        DtmfDebouncer {
            frame_len,
            min_press_frames: self.min_press_frames,
            min_gap_frames: self.min_gap_frames,
            samples_in_frame: 0,
            current: None,
            gap_frames: 0,
            current_start: 0,
            current_last: 0,
            current_history: Vec::new(),
            detector,
        }
    }
}

fn most_common_key(history: &[char]) -> Option<char> {
    let mut counts = [0usize; 16];
    for &ch in history {
        let idx = match ch {
            '1' => 0,
            '2' => 1,
            '3' => 2,
            'A' => 3,
            '4' => 4,
            '5' => 5,
            '6' => 6,
            'B' => 7,
            '7' => 8,
            '8' => 9,
            '9' => 10,
            'C' => 11,
            '*' => 12,
            '0' => 13,
            '#' => 14,
            'D' => 15,
            _ => unreachable!("unexpected dtmf key"),
        };
        counts[idx] += 1;
    }
    let mut max_i: Option<usize> = None;
    let mut max_v = 0usize;
    for (i, &v) in counts.iter().enumerate() {
        if v > max_v {
            max_v = v;
            max_i = Some(i);
        }
    }
    let idx = max_i?;
    Some(match idx {
        0 => '1',
        1 => '2',
        2 => '3',
        3 => 'A',
        4 => '4',
        5 => '5',
        6 => '6',
        7 => 'B',
        8 => '7',
        9 => '8',
        10 => '9',
        11 => 'C',
        12 => '*',
        13 => '0',
        14 => '#',
        15 => 'D',
        _ => unreachable!("invalid dtmf index"),
    })
}
