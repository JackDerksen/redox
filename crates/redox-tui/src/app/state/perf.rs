use std::time::Duration;

use super::{EditorMode, EditorState};

const PERF_EMA_ALPHA: f32 = 0.25;

#[derive(Debug, Clone, Copy, Default)]
pub struct FramePerfSample {
    pub frame: Duration,
    pub flush: Duration,
    pub load: Duration,
    pub snapshot: Duration,
    pub syntax: Duration,
    pub overlays: Duration,
    pub lines: Duration,
    pub status: Duration,
    pub input: Duration,
    pub event_count: usize,
}

impl FramePerfSample {
    pub fn measured_total(self) -> Duration {
        self.flush
            .saturating_add(self.load)
            .saturating_add(self.snapshot)
            .saturating_add(self.syntax)
            .saturating_add(self.overlays)
            .saturating_add(self.lines)
            .saturating_add(self.status)
            .saturating_add(self.input)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FramePerfStats {
    pub frame_ms: f32,
    pub flush_ms: f32,
    pub load_ms: f32,
    pub snapshot_ms: f32,
    pub syntax_ms: f32,
    pub overlays_ms: f32,
    pub lines_ms: f32,
    pub status_ms: f32,
    pub input_ms: f32,
    pub other_ms: f32,
    pub event_count: usize,
}

impl FramePerfStats {
    fn from_sample(sample: FramePerfSample) -> Self {
        let other = sample.frame.saturating_sub(sample.measured_total());
        Self {
            frame_ms: duration_ms(sample.frame),
            flush_ms: duration_ms(sample.flush),
            load_ms: duration_ms(sample.load),
            snapshot_ms: duration_ms(sample.snapshot),
            syntax_ms: duration_ms(sample.syntax),
            overlays_ms: duration_ms(sample.overlays),
            lines_ms: duration_ms(sample.lines),
            status_ms: duration_ms(sample.status),
            input_ms: duration_ms(sample.input),
            other_ms: duration_ms(other),
            event_count: sample.event_count,
        }
    }

    fn blend(self, sample: FramePerfSample) -> Self {
        let next = Self::from_sample(sample);
        Self {
            frame_ms: ema(self.frame_ms, next.frame_ms),
            flush_ms: ema(self.flush_ms, next.flush_ms),
            load_ms: ema(self.load_ms, next.load_ms),
            snapshot_ms: ema(self.snapshot_ms, next.snapshot_ms),
            syntax_ms: ema(self.syntax_ms, next.syntax_ms),
            overlays_ms: ema(self.overlays_ms, next.overlays_ms),
            lines_ms: ema(self.lines_ms, next.lines_ms),
            status_ms: ema(self.status_ms, next.status_ms),
            input_ms: ema(self.input_ms, next.input_ms),
            other_ms: ema(self.other_ms, next.other_ms),
            event_count: next.event_count,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PerfPopup {
    pub stats: Option<FramePerfStats>,
}

impl EditorState {
    pub fn perf_popup(&self) -> Option<PerfPopup> {
        self.perf_visible.then_some(PerfPopup {
            stats: self.perf_stats,
        })
    }

    pub fn record_perf_sample(&mut self, sample: FramePerfSample) {
        self.perf_stats = Some(match self.perf_stats {
            Some(stats) => stats.blend(sample),
            None => FramePerfStats::from_sample(sample),
        });
    }

    pub fn dismiss_perf_popup(&mut self) -> bool {
        if self.mode != EditorMode::Normal || !self.perf_visible {
            return false;
        }

        self.perf_visible = false;
        self.clear_status();
        true
    }

    pub(super) fn command_toggle_perf(&mut self) {
        self.perf_visible = !self.perf_visible;
        self.mode = EditorMode::Normal;
        self.clear_status();
    }
}

fn duration_ms(duration: Duration) -> f32 {
    duration.as_secs_f32() * 1_000.0
}

fn ema(previous: f32, next: f32) -> f32 {
    previous.mul_add(1.0 - PERF_EMA_ALPHA, next * PERF_EMA_ALPHA)
}
