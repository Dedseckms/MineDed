//! cpal device I/O. Streams live on a dedicated thread (cpal streams are not
//! `Send` on every platform); dropping [`AudioIo`] closes them.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use lc_types::voice::{AudioDevice, AudioDirection};
use lc_types::{AppError, AppResult, ErrorCode};

use crate::dsp::{downmix, Resampler, SAMPLE_RATE};
use crate::mixer::Mixer;

/// Max captured audio kept between ticks (500 ms at 48 kHz).
const CAPTURE_MAX: usize = 24_000;

/// Captured mono samples at `rate`, drained by the engine tick.
#[derive(Debug, Default)]
pub struct CaptureQueue {
    buf: Mutex<VecDeque<f32>>,
    rate: AtomicU32,
}

impl CaptureQueue {
    pub fn new() -> Self {
        Self {
            buf: Mutex::new(VecDeque::new()),
            rate: AtomicU32::new(SAMPLE_RATE),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }

    fn push(&self, mono: &[f32]) {
        let mut b = self.buf.lock().unwrap_or_else(PoisonError::into_inner);
        b.extend(mono.iter().copied());
        let excess = b.len().saturating_sub(CAPTURE_MAX);
        b.drain(..excess);
    }

    pub fn drain_into(&self, out: &mut Vec<f32>) {
        let mut b = self.buf.lock().unwrap_or_else(PoisonError::into_inner);
        out.extend(b.drain(..));
    }

    pub fn clear(&self) {
        self.buf
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

fn device_id(d: &Device) -> Option<String> {
    d.id().ok().map(|id| id.to_string())
}

fn device_name(d: &Device) -> String {
    d.description()
        .map(|desc| desc.name().to_owned())
        .unwrap_or_else(|_| "Unknown device".to_owned())
}

fn audio_err(what: &str, e: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::Io, format!("{what}: {e}"))
}

/// Lists input and output devices of the default host.
pub fn list_devices() -> AppResult<Vec<AudioDevice>> {
    let host = cpal::default_host();
    let def_in = host.default_input_device().and_then(|d| device_id(&d));
    let def_out = host.default_output_device().and_then(|d| device_id(&d));
    let mut out = Vec::new();
    let mut add = |devices: Vec<Device>, direction: AudioDirection, def: &Option<String>| {
        for d in devices {
            let Some(id) = device_id(&d) else { continue };
            out.push(AudioDevice {
                is_default: def.as_deref() == Some(id.as_str()),
                name: device_name(&d),
                id,
                direction,
            });
        }
    };
    let inputs = host
        .input_devices()
        .map_err(|e| audio_err("input devices", e))?
        .collect();
    add(inputs, AudioDirection::Input, &def_in);
    let outputs = host
        .output_devices()
        .map_err(|e| audio_err("output devices", e))?
        .collect();
    add(outputs, AudioDirection::Output, &def_out);
    Ok(out)
}

fn find_device(input: bool, id: Option<&str>) -> AppResult<Device> {
    let host = cpal::default_host();
    if let Some(id) = id {
        let devices = if input {
            host.input_devices().map(|d| d.collect::<Vec<_>>())
        } else {
            host.output_devices().map(|d| d.collect::<Vec<_>>())
        }
        .map_err(|e| audio_err("devices", e))?;
        if let Some(d) = devices
            .into_iter()
            .find(|d| device_id(d).as_deref() == Some(id))
        {
            return Ok(d);
        }
        tracing::warn!(device = id, "audio device not found, using default");
    }
    let d = if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    };
    d.ok_or_else(|| {
        AppError::new(ErrorCode::NotFound, "no audio device")
            .detail("direction", if input { "input" } else { "output" })
    })
}

/// What to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoConfig {
    pub input: Option<Option<String>>,
    pub output: Option<String>,
}

/// Open streams. Drop to close.
pub struct AudioIo {
    pub config: IoConfig,
    /// Set when the input could not be opened (listen-only).
    pub input_error: Option<AppError>,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioIo {
    /// Opens the output (required) and the input (optional, failure is kept
    /// in `input_error`).
    pub fn open(
        config: IoConfig,
        capture: Arc<CaptureQueue>,
        mixer: Arc<Mutex<Mixer>>,
    ) -> AppResult<Self> {
        let (ready_tx, ready_rx) = mpsc::sync_channel::<AppResult<Option<AppError>>>(1);
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let cfg = config.clone();
        let thread = std::thread::Builder::new()
            .name("voice-audio".into())
            .spawn(move || {
                let output = match build_output(cfg.output.as_deref(), mixer) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let mut input_error = None;
                let input = match &cfg.input {
                    Some(dev) => match build_input(dev.as_deref(), capture) {
                        Ok(s) => Some(s),
                        Err(e) => {
                            input_error = Some(e);
                            None
                        }
                    },
                    None => None,
                };
                let _ = ready_tx.send(Ok(input_error));
                // Park until stopped (sender dropped).
                let _ = stop_rx.recv();
                drop(input);
                drop(output);
            })
            .map_err(|e| audio_err("audio thread", e))?;
        let input_error = ready_rx
            .recv()
            .map_err(|_| AppError::internal("audio thread exited"))??;
        Ok(Self {
            config,
            input_error,
            stop: Some(stop_tx),
            thread: Some(thread),
        })
    }
}

impl Drop for AudioIo {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn build_input(id: Option<&str>, capture: Arc<CaptureQueue>) -> AppResult<Stream> {
    let device = find_device(true, id)?;
    let supported = device
        .default_input_config()
        .map_err(|e| audio_err("input config", e))?;
    let config: StreamConfig = supported.config();
    capture.rate.store(config.sample_rate, Ordering::Relaxed);
    capture.clear();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => input_stream::<f32>(&device, &config, capture),
        SampleFormat::I16 => input_stream::<i16>(&device, &config, capture),
        SampleFormat::I32 => input_stream::<i32>(&device, &config, capture),
        SampleFormat::U16 => input_stream::<u16>(&device, &config, capture),
        other => Err(
            AppError::new(ErrorCode::Io, "unsupported input sample format")
                .detail("format", format!("{other:?}")),
        ),
    }?;
    stream.play().map_err(|e| audio_err("input play", e))?;
    Ok(stream)
}

fn input_stream<T>(
    device: &Device,
    config: &StreamConfig,
    capture: Arc<CaptureQueue>,
) -> AppResult<Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let mut f = Vec::new();
    let mut mono = Vec::new();
    device
        .build_input_stream(
            *config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                f.clear();
                f.extend(data.iter().map(|s| s.to_sample::<f32>()));
                mono.clear();
                downmix(&f, channels, &mut mono);
                capture.push(&mono);
            },
            |e| tracing::warn!(error = %e, "voice input stream error"),
            None,
        )
        .map_err(|e| audio_err("input stream", e))
}

fn build_output(id: Option<&str>, mixer: Arc<Mutex<Mixer>>) -> AppResult<Stream> {
    let device = find_device(false, id)?;
    let supported = device
        .default_output_config()
        .map_err(|e| audio_err("output config", e))?;
    let config: StreamConfig = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => output_stream::<f32>(&device, &config, mixer),
        SampleFormat::I16 => output_stream::<i16>(&device, &config, mixer),
        SampleFormat::I32 => output_stream::<i32>(&device, &config, mixer),
        SampleFormat::U16 => output_stream::<u16>(&device, &config, mixer),
        other => Err(
            AppError::new(ErrorCode::Io, "unsupported output sample format")
                .detail("format", format!("{other:?}")),
        ),
    }?;
    stream.play().map_err(|e| audio_err("output play", e))?;
    Ok(stream)
}

fn output_stream<T>(
    device: &Device,
    config: &StreamConfig,
    mixer: Arc<Mutex<Mixer>>,
) -> AppResult<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels.max(1));
    let mut resampler = Resampler::new(SAMPLE_RATE, config.sample_rate);
    let mut pending: VecDeque<f32> = VecDeque::new();
    let mut pulled = Vec::new();
    let mut resampled = Vec::new();
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let frames = data.len() / channels;
                while pending.len() < frames {
                    pulled.clear();
                    mixer
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .pull(crate::dsp::FRAME, &mut pulled);
                    resampled.clear();
                    resampler.process(&pulled, &mut resampled);
                    pending.extend(resampled.iter().copied());
                }
                for frame in data.chunks_exact_mut(channels) {
                    let s = pending.pop_front().unwrap_or(0.0);
                    for out in frame {
                        *out = T::from_sample(s);
                    }
                }
            },
            |e| tracing::warn!(error = %e, "voice output stream error"),
            None,
        )
        .map_err(|e| audio_err("output stream", e))
}
