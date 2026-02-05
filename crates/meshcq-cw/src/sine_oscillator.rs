use std::sync::OnceLock;

const TABLE_LEN: usize = 4096;

static SINE_TABLE: OnceLock<Vec<f32>> = OnceLock::new();

fn sine_table() -> &'static [f32] {
    SINE_TABLE.get_or_init(|| {
        let mut table = Vec::with_capacity(TABLE_LEN);
        for i in 0..TABLE_LEN {
            let phase = (i as f32) * std::f32::consts::TAU / TABLE_LEN as f32;
            table.push(phase.sin());
        }
        table
    })
}

pub(super) struct SineOscillator {
    phase: f32,
    phase_inc: f32,
}

impl SineOscillator {
    pub(super) fn new(sample_rate_hz: f32, tone_freq_hz: f32) -> Self {
        let phase_inc = tone_freq_hz * TABLE_LEN as f32 / sample_rate_hz;
        Self {
            phase: 0.0,
            phase_inc,
        }
    }

    pub(super) fn next(&mut self) -> f32 {
        let table = sine_table();
        let idx = self.phase.floor() as usize;
        let frac = self.phase - idx as f32;
        let next = (idx + 1) % TABLE_LEN;
        let value = table[idx] * (1.0 - frac) + table[next] * frac;

        self.advance(1);

        value
    }

    pub(super) fn advance(&mut self, samples: usize) {
        if samples == 0 {
            return;
        }
        self.phase += self.phase_inc * samples as f32;
        let table_len = TABLE_LEN as f32;
        if self.phase >= table_len {
            self.phase %= table_len;
        }
    }

    pub(super) fn reset(&mut self) {
        self.phase = 0.0;
    }
}
