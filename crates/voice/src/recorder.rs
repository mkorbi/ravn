//! Microphone capture via `cpal`.
//!
//! cpal's `Stream` is `!Send`, so capture runs on a dedicated OS thread that
//! owns the stream for its whole lifetime. The [`Recorder`] handle only holds
//! a stop flag + the thread's `JoinHandle`, so it is `Send + Sync` and safe to
//! park in the async UI state.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

use crate::error::Error;

/// Raw captured audio plus the format it came in as. Samples are interleaved
/// and normalized to `[-1.0, 1.0]`.
pub struct RecordResult {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

struct Active {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Result<RecordResult, Error>>,
}

#[derive(Default)]
pub struct Recorder {
    active: Mutex<Option<Active>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_recording(&self) -> bool {
        self.active.lock().is_some()
    }

    /// Begin capturing from the default input device. No-op if already running.
    pub fn start(&self) -> Result<(), Error> {
        let mut guard = self.active.lock();
        if guard.is_some() {
            return Ok(());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let handle = std::thread::Builder::new()
            .name("ravn-mic".into())
            .spawn(move || capture_loop(stop_thread))
            .map_err(|e| Error::Audio(format!("spawn capture thread: {e}")))?;
        *guard = Some(Active { stop, handle });
        Ok(())
    }

    /// Stop capturing and return the audio. Returns `None` if not recording or
    /// if the capture thread errored / panicked (logged).
    pub fn stop(&self) -> Option<RecordResult> {
        let active = self.active.lock().take()?;
        active.stop.store(true, Ordering::Relaxed);
        match active.handle.join() {
            Ok(Ok(result)) => Some(result),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "mic capture failed");
                None
            }
            Err(_) => {
                tracing::warn!("mic capture thread panicked");
                None
            }
        }
    }
}

fn cpal_err(e: cpal::StreamError) {
    tracing::warn!(error = %e, "cpal input stream error");
}

fn capture_loop(stop: Arc<AtomicBool>) -> Result<RecordResult, Error> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or(Error::NoDevice)?;
    let supported = device
        .default_input_config()
        .map_err(|e| Error::Audio(format!("default input config: {e}")))?;
    let sample_rate = supported.sample_rate();
    let channels = supported.channels();
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let stream = build_stream(&device, &config, sample_format, buf.clone())?;
    stream
        .play()
        .map_err(|e| Error::Audio(format!("play stream: {e}")))?;

    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(stream); // stops the data callback

    let samples = std::mem::take(&mut *buf.lock());
    Ok(RecordResult {
        samples,
        sample_rate,
        channels,
    })
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    buf: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream, Error> {
    use cpal::InputCallbackInfo as Info;
    use cpal::SampleFormat as SF;
    let res = match format {
        SF::F32 => {
            let b = buf.clone();
            device.build_input_stream(
                config,
                move |data: &[f32], _: &Info| b.lock().extend_from_slice(data),
                cpal_err,
                None,
            )
        }
        SF::I16 => {
            let b = buf.clone();
            device.build_input_stream(
                config,
                move |data: &[i16], _: &Info| {
                    b.lock().extend(data.iter().map(|&s| s as f32 / 32768.0))
                },
                cpal_err,
                None,
            )
        }
        SF::U16 => {
            let b = buf.clone();
            device.build_input_stream(
                config,
                move |data: &[u16], _: &Info| {
                    b.lock().extend(data.iter().map(|&s| s as f32 / 32768.0 - 1.0))
                },
                cpal_err,
                None,
            )
        }
        SF::I32 => {
            let b = buf.clone();
            device.build_input_stream(
                config,
                move |data: &[i32], _: &Info| {
                    b.lock()
                        .extend(data.iter().map(|&s| s as f32 / 2_147_483_648.0))
                },
                cpal_err,
                None,
            )
        }
        other => return Err(Error::UnsupportedFormat(format!("{other:?}"))),
    };
    res.map_err(|e| Error::Audio(format!("build input stream: {e}")))
}
