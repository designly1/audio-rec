//! Estimates Bluetooth audio quality by loopback-capturing the default output and
//! counting short silence gaps in the middle of playing audio (stream underruns).
//! Windows exposes no packet-loss or RSSI data for A2DP sinks, so this is a proxy.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
            IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
            WAVEFORMATEX, eConsole, eRender,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
            CoUninitialize,
        },
    },
    core::Result,
};

const SILENCE: f32 = 1e-4; // about -80 dBFS
const MIN_PLAY: f32 = 0.3;
const MIN_GAP: f32 = 0.02;
const MAX_GAP: f32 = 1.0;
const FLOOR_DB: f32 = -90.0;
const DECAY_DB_PER_SEC: f32 = 20.0;
const POLL: Duration = Duration::from_millis(10);
const STALL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
pub struct Stats {
    pub level_db: f32,
    pub last_audio: Option<Instant>,
    pub dropouts: Vec<Instant>,
    pub error: Option<String>,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            level_db: FLOOR_DB,
            last_audio: None,
            dropouts: Vec::new(),
            error: None,
        }
    }
}

impl Stats {
    pub fn dropouts_since(&self, now: Instant, window: Duration) -> usize {
        self.dropouts
            .iter()
            .filter(|t| now.saturating_duration_since(**t) <= window)
            .count()
    }
}

/// Detects mid-playback gaps of 20 ms to 1 s. Longer silences are pauses.
#[derive(Debug)]
pub struct DropoutDetector {
    audio_run: f32,
    gap: f32,
    armed: bool,
    pub level_db: f32,
}

impl Default for DropoutDetector {
    fn default() -> Self {
        Self {
            audio_run: 0.0,
            gap: 0.0,
            armed: false,
            level_db: FLOOR_DB,
        }
    }
}

impl DropoutDetector {
    /// Feeds one block's peak (linear, 0..1) lasting `secs`. Returns true on a dropout.
    pub fn feed(&mut self, peak: f32, secs: f32) -> bool {
        let peak_db = if peak > 0.0 {
            (20.0 * peak.log10()).max(FLOOR_DB)
        } else {
            FLOOR_DB
        };
        self.level_db = peak_db.max(self.level_db - DECAY_DB_PER_SEC * secs);
        if peak < SILENCE {
            if self.audio_run >= MIN_PLAY {
                self.armed = true;
            }
            self.audio_run = 0.0;
            if self.armed {
                self.gap += secs;
                if self.gap > MAX_GAP {
                    self.armed = false;
                    self.gap = 0.0;
                }
            }
            false
        } else {
            let dropout = self.armed && self.gap >= MIN_GAP;
            self.armed = false;
            self.gap = 0.0;
            // Stay "playing" after a dropout so rapid stutters are all counted.
            self.audio_run = if dropout { MIN_PLAY } else { self.audio_run } + secs;
            dropout
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Offline,
    Measuring,
    Good,
    Fair,
    Poor,
}

impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Quality::Offline => "Not connected",
            Quality::Measuring => "Waiting for audio",
            Quality::Good => "Good",
            Quality::Fair => "Fair",
            Quality::Poor => "Poor",
        }
    }
}

pub fn rating(connected: bool, per_min: usize, playing_recently: bool) -> Quality {
    match (connected, per_min) {
        (false, _) => Quality::Offline,
        (_, 3..) => Quality::Poor,
        (_, 1..) => Quality::Fair,
        _ if !playing_recently => Quality::Measuring,
        _ => Quality::Good,
    }
}

pub struct Monitor {
    stop: Arc<AtomicBool>,
    stats: Arc<Mutex<Stats>>,
    thread: Option<JoinHandle<()>>,
}

impl Monitor {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Mutex::new(Stats::default()));
        let thread = {
            let (stop, stats) = (stop.clone(), stats.clone());
            std::thread::spawn(move || worker(&stop, &stats))
        };
        Self {
            stop,
            stats,
            thread: Some(thread),
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Com;
impl Com {
    fn new() -> Result<Self> {
        // SAFETY: called once on this fresh worker thread; balanced in Drop.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: balances the successful initialization on this same thread.
        unsafe { CoUninitialize() };
    }
}

fn worker(stop: &AtomicBool, stats: &Mutex<Stats>) {
    let _com = match Com::new() {
        Ok(com) => com,
        Err(error) => {
            set_error(stats, format!("Audio monitor unavailable: {error}"));
            return;
        }
    };
    let mut detector = DropoutDetector::default();
    while !stop.load(Ordering::Relaxed) {
        if let Err(error) = capture(stop, stats, &mut detector) {
            set_error(stats, format!("Audio monitor paused: {error}"));
            // Retry (e.g. after a default-device change) while staying responsive to stop.
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until && !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn set_error(stats: &Mutex<Stats>, error: String) {
    if let Ok(mut stats) = stats.lock() {
        stats.error = Some(error);
    }
}

#[derive(Clone, Copy)]
enum SampleKind {
    F32,
    I16,
}

/// Reads the mix format, returning (sample kind, channels, sample rate).
///
/// # Safety
/// `format` must point to a valid WAVEFORMATEX (or WAVEFORMATEXTENSIBLE).
unsafe fn read_format(format: *const WAVEFORMATEX) -> Option<(SampleKind, usize, u32)> {
    // SAFETY: guaranteed by the caller; the struct is packed, so read unaligned.
    let f = unsafe { format.read_unaligned() };
    let (tag, bits, size) = (f.wFormatTag, f.wBitsPerSample, f.cbSize);
    let sub = if tag == 0xFFFE && size >= 22 {
        // SubFormat GUID starts at byte 24; its first u32 is the base format tag.
        // SAFETY: cbSize >= 22 guarantees the extensible tail is present.
        unsafe { (format as *const u8).add(24).cast::<u32>().read_unaligned() }
    } else {
        u32::from(tag)
    };
    let kind = match (sub, bits) {
        (3, 32) => SampleKind::F32,
        (1, 16) => SampleKind::I16,
        _ => return None,
    };
    Some((kind, usize::from(f.nChannels), f.nSamplesPerSec))
}

fn capture(stop: &AtomicBool, stats: &Mutex<Stats>, detector: &mut DropoutDetector) -> Result<()> {
    // SAFETY: COM is initialized on this thread; all pointers come from the API
    // and are used only within their documented lifetimes.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let format = client.GetMixFormat()?;
        let parsed = read_format(format);
        let init = client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            2_000_000, // 200 ms in 100 ns units
            0,
            format,
            None,
        );
        CoTaskMemFree(Some(format as *const _));
        init?;
        let Some((kind, channels, rate)) = parsed else {
            set_error(stats, "Audio monitor: unsupported output format".into());
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
            }
            return Ok(());
        };
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        if let Ok(mut stats) = stats.lock() {
            stats.error = None;
        }

        let mut last_data = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(POLL);
            let mut got = false;
            while capture.GetNextPacketSize()? > 0 {
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                let samples = frames as usize * channels;
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                let peak = if silent || data.is_null() || samples == 0 {
                    0.0
                } else {
                    match kind {
                        SampleKind::F32 => std::slice::from_raw_parts(data.cast::<f32>(), samples)
                            .iter()
                            .fold(0.0f32, |m, s| m.max(s.abs())),
                        SampleKind::I16 => std::slice::from_raw_parts(data.cast::<i16>(), samples)
                            .iter()
                            .fold(0.0f32, |m, s| m.max(f32::from(*s).abs() / 32768.0)),
                    }
                };
                capture.ReleaseBuffer(frames)?;
                got = true;
                last_data = Instant::now();
                record(stats, detector, peak, frames as f32 / rate as f32);
            }
            // Loopback delivers nothing while the render stream is idle; treat a
            // stall as silence so a stream that stops mid-song still counts as a gap.
            if !got && last_data.elapsed() >= STALL {
                let idle = last_data.elapsed().as_secs_f32();
                last_data = Instant::now();
                record(stats, detector, 0.0, idle);
            }
        }
        let _ = client.Stop();
    }
    Ok(())
}

fn record(stats: &Mutex<Stats>, detector: &mut DropoutDetector, peak: f32, secs: f32) {
    let dropout = detector.feed(peak, secs);
    if let Ok(mut stats) = stats.lock() {
        let now = Instant::now();
        stats.level_db = detector.level_db;
        if peak >= SILENCE {
            stats.last_audio = Some(now);
        }
        if dropout {
            stats.dropouts.push(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(d: &mut DropoutDetector, peak: f32, secs: f32) -> usize {
        let steps = (secs / 0.01).round() as usize;
        (0..steps).filter(|_| d.feed(peak, 0.01)).count()
    }

    #[test]
    fn short_gap_mid_playback_is_a_dropout() {
        let mut d = DropoutDetector::default();
        assert_eq!(run(&mut d, 0.5, 1.0), 0);
        assert_eq!(run(&mut d, 0.0, 0.1), 0);
        assert_eq!(run(&mut d, 0.5, 0.5), 1);
    }

    #[test]
    fn rapid_stutters_are_all_counted() {
        let mut d = DropoutDetector::default();
        run(&mut d, 0.5, 1.0);
        let mut total = 0;
        for _ in 0..3 {
            total += run(&mut d, 0.0, 0.05);
            total += run(&mut d, 0.5, 0.05);
        }
        assert_eq!(total, 3);
    }

    #[test]
    fn long_pause_is_not_a_dropout() {
        let mut d = DropoutDetector::default();
        run(&mut d, 0.5, 1.0);
        run(&mut d, 0.0, 2.0);
        assert_eq!(run(&mut d, 0.5, 0.5), 0);
    }

    #[test]
    fn gap_without_prior_playback_is_ignored() {
        let mut d = DropoutDetector::default();
        run(&mut d, 0.5, 0.1);
        run(&mut d, 0.0, 0.1);
        assert_eq!(run(&mut d, 0.5, 0.5), 0);
    }

    #[test]
    fn tiny_gap_is_ignored_and_steady_audio_is_clean() {
        let mut d = DropoutDetector::default();
        assert_eq!(run(&mut d, 0.5, 2.0), 0);
        run(&mut d, 0.0, 0.01);
        assert_eq!(run(&mut d, 0.5, 0.5), 0);
        assert!(d.level_db > -7.0 && d.level_db < -5.0);
    }

    #[test]
    fn rating_thresholds() {
        assert_eq!(rating(false, 0, true), Quality::Offline);
        assert_eq!(rating(true, 0, false), Quality::Measuring);
        assert_eq!(rating(true, 0, true), Quality::Good);
        assert_eq!(rating(true, 1, true), Quality::Fair);
        assert_eq!(rating(true, 2, false), Quality::Fair);
        assert_eq!(rating(true, 3, true), Quality::Poor);
    }

    /// Opens a real loopback stream on the default output. Run with `--ignored`.
    #[test]
    #[ignore]
    fn loopback_opens_on_this_machine() {
        let monitor = Monitor::start();
        std::thread::sleep(Duration::from_millis(1500));
        let stats = monitor.stats();
        assert!(stats.error.is_none(), "{:?}", stats.error);
    }

    #[test]
    fn dropouts_since_window() {
        let now = Instant::now();
        let stats = Stats {
            dropouts: vec![
                now - Duration::from_secs(90),
                now - Duration::from_secs(10),
                now,
            ],
            ..Stats::default()
        };
        assert_eq!(stats.dropouts_since(now, Duration::from_secs(60)), 2);
    }
}
