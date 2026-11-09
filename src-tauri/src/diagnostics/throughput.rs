//! Bounded observations of admitted byte counters, independent of engine speed estimates.
//! Missing progress is an observation gap, never proof that the network disconnected.
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, time::Duration};

const STALL_MS: f64 = 5_000.0;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadProtocol {
    Unknown,
    Http,
    Https,
    HttpDashSegments,
    M3u8,
    M3u8Native,
}
impl DownloadProtocol {
    pub fn from_engine(value: &str) -> Self {
        match value {
            "http" => Self::Http,
            "https" => Self::Https,
            "http_dash_segments" => Self::HttpDashSegments,
            "m3u8" => Self::M3u8,
            "m3u8_native" => Self::M3u8Native,
            _ => Self::Unknown,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloaderMode {
    YtdlpDefault,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    Inherited,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThroughputSnapshot {
    #[serde(default)]
    pub protocol: Option<DownloadProtocol>,
    #[serde(default)]
    pub downloader: Option<DownloaderMode>,
    #[serde(default)]
    pub network_mode: Option<NetworkMode>,
    pub observed_bytes: u64,
    pub duration_ms: f64,
    pub samples: u32,
    pub stream_count: u32,
    pub mean_bps: Option<f64>,
    pub window_5s_bps: Option<f64>,
    pub window_10s_bps: Option<f64>,
    pub max_5s_bps: Option<f64>,
    pub stall_count: u32,
    pub stall_duration_ms: f64,
    pub progress_gap_ms: f64,
    pub final_sample: bool,
}
impl ThroughputSnapshot {
    pub(super) fn valid(&self) -> bool {
        let bounded = |v: f64| v.is_finite() && (0.0..=604_800_000.0).contains(&v);
        bounded(self.duration_ms)
            && bounded(self.stall_duration_ms)
            && bounded(self.progress_gap_ms)
            && self.stall_duration_ms <= self.duration_ms
            && self.progress_gap_ms <= self.duration_ms
            && self.stream_count <= 33
            && [
                self.mean_bps,
                self.window_5s_bps,
                self.window_10s_bps,
                self.max_5s_bps,
            ]
            .into_iter()
            .flatten()
            .all(|v| v.is_finite() && (0.0..=1.0e15).contains(&v))
    }
}

#[derive(Default)]
pub(super) struct Throughput {
    protocol: Option<DownloadProtocol>,
    start: Option<Duration>,
    elapsed_ms: f64,
    last_bytes: u64,
    bytes: u64,
    samples: u32,
    streams: u32,
    points: VecDeque<(f64, u64)>,
    growth_ms: f64,
    stalled: bool,
    stall_count: u32,
    completed_stalls_ms: f64,
    max_5s: Option<f64>,
    emitted: Option<Duration>,
    stopped: bool,
    finalized: bool,
}
impl Throughput {
    pub(super) fn protocol(&mut self, protocol: Option<DownloadProtocol>, new_stream: bool) {
        if new_stream {
            self.protocol = None;
        }
        if protocol.is_some() {
            self.protocol = protocol;
        }
    }
    fn elapsed(&self, now: Duration) -> f64 {
        self.elapsed_ms
            + self.start.map_or(0.0, |start| {
                now.saturating_sub(start).as_secs_f64() * 1000.0
            })
    }
    fn gap(&self, elapsed: f64) -> f64 {
        (elapsed - self.growth_ms).max(0.0)
    }
    fn mark_stall(&mut self, elapsed: f64) {
        if self.gap(elapsed) >= STALL_MS && !self.stalled {
            self.stalled = true;
            self.stall_count = self.stall_count.saturating_add(1);
        }
    }
    fn point(&mut self, elapsed: f64) {
        if self.points.back().is_some_and(|p| p.0 == elapsed) {
            self.points.pop_back();
        }
        self.points.push_back((elapsed, self.bytes));
        while self.points.len() > 2
            && self
                .points
                .get(1)
                .is_some_and(|p| p.0 <= elapsed - 10_000.0)
        {
            self.points.pop_front();
        }
        // Malformed/extra-high-frequency producers cannot grow this window indefinitely.
        while self.points.len() > 128 {
            self.points.pop_front();
        }
        if let Some(rate) = self.window(elapsed, 5_000.0) {
            self.max_5s = Some(self.max_5s.map_or(rate, |old| old.max(rate)));
        }
    }
    fn window(&self, elapsed: f64, width_ms: f64) -> Option<f64> {
        // Use a real observation at/before the boundary, without inventing partial bytes.
        let (time, bytes) = self
            .points
            .iter()
            .rev()
            .find(|p| p.0 <= elapsed - width_ms)?;
        let duration = elapsed - time;
        (duration > 0.0).then(|| self.bytes.saturating_sub(*bytes) as f64 * 1000.0 / duration)
    }
    fn snapshot(&self, now: Duration, final_sample: bool) -> ThroughputSnapshot {
        let elapsed = self.elapsed(now);
        let gap = self.gap(elapsed);
        ThroughputSnapshot {
            protocol: self.protocol,
            downloader: Some(DownloaderMode::YtdlpDefault),
            network_mode: Some(NetworkMode::Inherited),
            observed_bytes: self.bytes,
            duration_ms: elapsed,
            samples: self.samples,
            stream_count: self.streams,
            mean_bps: (elapsed > 0.0 && self.samples >= 2)
                .then(|| self.bytes as f64 * 1000.0 / elapsed),
            window_5s_bps: self.window(elapsed, 5_000.0),
            window_10s_bps: self.window(elapsed, 10_000.0),
            max_5s_bps: self.max_5s,
            stall_count: self.stall_count,
            stall_duration_ms: self.completed_stalls_ms + if self.stalled { gap } else { 0.0 },
            progress_gap_ms: gap,
            final_sample,
        }
    }
    fn due(&mut self, now: Duration) -> Option<ThroughputSnapshot> {
        if self.samples == 0
            || self
                .emitted
                .is_some_and(|previous| now.saturating_sub(previous) < SAMPLE_INTERVAL)
        {
            return None;
        }
        self.emitted = Some(now);
        Some(self.snapshot(now, false))
    }
    pub(super) fn observe(
        &mut self,
        now: Duration,
        bytes: u64,
        new_stream: bool,
        finished: bool,
    ) -> Option<ThroughputSnapshot> {
        if self.stopped {
            return None;
        }
        if self.samples == 0 || new_stream {
            if self.streams >= 33 {
                return None;
            }
            self.freeze_stream(now);
            if self.stalled {
                self.completed_stalls_ms += self.gap(self.elapsed_ms);
                self.stalled = false;
            }
            self.streams += 1;
            self.start = Some(now);
            self.last_bytes = bytes;
            self.growth_ms = self.elapsed_ms;
            self.points.clear();
        } else if self.start.is_none() || bytes < self.last_bytes {
            return None;
        } else {
            let elapsed = self.elapsed(now);
            self.mark_stall(elapsed);
            let delta = bytes - self.last_bytes;
            self.bytes = self.bytes.saturating_add(delta);
            self.last_bytes = bytes;
            if delta > 0 {
                if self.stalled {
                    self.completed_stalls_ms += self.gap(elapsed);
                    self.stalled = false;
                }
                self.growth_ms = elapsed;
            }
        }
        self.samples = self.samples.saturating_add(1);
        self.point(self.elapsed(now));
        if finished {
            self.freeze_stream(now);
        }
        self.due(now)
    }
    fn freeze_stream(&mut self, now: Duration) {
        if self.start.is_some() {
            self.elapsed_ms = self.elapsed(now);
            self.mark_stall(self.elapsed_ms);
            self.start = None;
        }
    }
    pub(super) fn tick(&mut self, now: Duration) -> Option<ThroughputSnapshot> {
        if self.stopped || self.start.is_none() {
            return None;
        }
        let elapsed = self.elapsed(now);
        self.mark_stall(elapsed);
        self.point(elapsed);
        self.due(now)
    }
    pub(super) fn stop(&mut self, now: Duration) {
        self.freeze_stream(now);
        self.stopped = true;
    }
    pub(super) fn pause(&mut self, now: Duration) {
        self.freeze_stream(now);
    }
    pub(super) fn finalize(&mut self, now: Duration) -> Option<ThroughputSnapshot> {
        self.stop(now);
        if self.samples == 0 || self.finalized {
            return None;
        }
        self.finalized = true;
        Some(self.snapshot(now, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }
    #[test]
    fn real_deltas_windows_and_units_exclude_initial_resume_bytes() {
        let mut t = Throughput::default();
        let first = t.observe(s(0), 50_000, false, false).unwrap();
        assert_eq!(first.mean_bps, None);
        for n in 1..=10 {
            t.observe(s(n), 50_000 + n * 1024, false, false);
        }
        let v = t.finalize(s(10)).unwrap();
        assert_eq!(v.observed_bytes, 10 * 1024);
        assert_eq!(v.mean_bps, Some(1024.0));
        assert_eq!(v.window_5s_bps, Some(1024.0));
        assert_eq!(v.window_10s_bps, Some(1024.0));
        assert_eq!(v.mean_bps.unwrap() / 1024.0, 1.0);
        assert_eq!(v.mean_bps.unwrap() * 8.0 / 1000.0, 8.192);
        assert!(v.valid());
        assert!(t.finalize(s(20)).is_none());
    }
    #[test]
    fn gaps_are_observations_and_end_postprocessing_freezes_clock() {
        let mut t = Throughput::default();
        t.observe(s(0), 0, false, false);
        t.observe(s(1), 1000, false, false);
        for n in 2..=8 {
            t.tick(s(n));
        }
        let v = t.snapshot(s(8), false);
        assert_eq!(v.stall_count, 1);
        assert_eq!(v.stall_duration_ms, 7000.0);
        t.observe(s(9), 2000, false, false);
        t.observe(s(10), 3000, false, true);
        // Completed stream waits for FFmpeg/next media; time is frozen.
        assert!(t.tick(s(30)).is_none());
        let v = t.finalize(s(40)).unwrap();
        assert_eq!(v.duration_ms, 10_000.0);
        assert_eq!(v.stall_duration_ms, 8000.0);
        assert_eq!(v.mean_bps, Some(300.0));
        assert!(t.observe(s(50), 10_000, false, false).is_none());
    }
    #[test]
    fn streams_do_not_add_counter_jumps_or_bridge_window_gaps() {
        let mut t = Throughput::default();
        t.observe(s(0), 100, false, false);
        t.observe(s(2), 2100, false, true);
        t.observe(s(20), 80_000, true, false);
        t.observe(s(22), 82_000, false, true);
        let v = t.finalize(s(100)).unwrap();
        assert_eq!(v.observed_bytes, 4000);
        assert_eq!(v.duration_ms, 4000.0);
        assert_eq!(v.stream_count, 2);
        assert_eq!(v.window_5s_bps, None);
        assert_eq!(v.window_10s_bps, None);
        assert_eq!(v.mean_bps, Some(1000.0));
    }
    #[test]
    fn no_progress_is_absent_and_rate_output_is_bounded() {
        let mut t = Throughput::default();
        assert!(t.tick(s(100)).is_none());
        assert!(t.finalize(s(100)).is_none());
        let mut t = Throughput::default();
        let mut outputs = 0;
        for n in 0..=1000 {
            outputs += usize::from(
                t.observe(Duration::from_millis(n * 10), n, false, false)
                    .is_some(),
            );
        }
        assert_eq!(outputs, 3);
        assert!(t.points.len() <= 128);
        assert_eq!(t.snapshot(s(10), false).window_10s_bps, None);
    }
}
