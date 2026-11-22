//! Microphone level monitoring with `cpal`.
//!
//! The audio thread computes a peak level into an atomic slot; the UI only
//! reads that slot. Device discovery is best-effort: on a machine without an
//! input device the status is reported as unavailable rather than faked.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// An RMS level in `0.0..=1.0`, stored as `f32` bits in an atomic.
#[derive(Debug, Clone)]
pub struct AudioLevel(Arc<AtomicU32>);

impl AudioLevel {
    /// Create a level at zero.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(AtomicU32::new(0)))
    }

    /// Publish a new level, clamped to `0.0..=1.0`.
    #[cfg(test)]
    pub fn set(&self, level: f32) {
        self.0
            .store(level.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Current level.
    #[must_use]
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed)).clamp(0.0, 1.0)
    }
}

impl Default for AudioLevel {
    fn default() -> Self {
        Self::new()
    }
}

/// Why microphone monitoring is unavailable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioError {
    /// No default input device is configured.
    #[error("no microphone found on this system")]
    NoDevice,
    /// The device could not provide a compatible input configuration.
    #[error("microphone configuration error: {0}")]
    Configuration(String),
    /// The operating system refused to start the input stream.
    #[error("could not start microphone stream: {0}")]
    Stream(String),
}

/// Result of trying to start microphone monitoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioStatus {
    /// Monitoring is running on the named device.
    Running(String),
    /// Monitoring is not running, with a user-facing reason.
    Unavailable(String),
}

impl AudioStatus {
    /// Short text for the status bar.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Running(device) => format!("Microphone: {device}"),
            Self::Unavailable(reason) => format!("Microphone unavailable: {reason}"),
        }
    }
}

/// A live microphone monitor. Keeps the `cpal` stream alive while in scope.
pub struct AudioMonitor {
    _stream: Option<cpal::Stream>,
    level: AudioLevel,
    status: AudioStatus,
}

impl AudioMonitor {
    /// Start monitoring the default input device, reporting failures instead
    /// of panicking. Safe to call on a headless machine.
    #[must_use]
    pub fn start() -> Self {
        let level = AudioLevel::new();
        let (stream, status) = match Self::try_start(&level) {
            Ok((stream, device)) => (Some(stream), AudioStatus::Running(device)),
            Err(error) => (None, AudioStatus::Unavailable(error.to_string())),
        };
        Self {
            _stream: stream,
            level,
            status,
        }
    }

    fn try_start(level: &AudioLevel) -> Result<(cpal::Stream, String), AudioError> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(AudioError::NoDevice)?;
        let name = device.to_string();
        let config = device
            .default_input_config()
            .map_err(|error| AudioError::Configuration(error.to_string()))?;
        let sample_format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();
        let callback_level = Arc::clone(&level.0);

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                stream_config,
                move |data: &[f32], _| {
                    callback_level.store(rms_f32(data).to_bits(), Ordering::Relaxed);
                },
                |error| eprintln!("microphone input error: {error}"),
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                stream_config,
                move |data: &[i16], _| {
                    let peak = data
                        .iter()
                        .copied()
                        .map(|sample| f32::from(sample) / f32::from(i16::MAX))
                        .fold(0.0_f32, f32::max);
                    callback_level.store(peak.abs().to_bits(), Ordering::Relaxed);
                },
                |error| eprintln!("microphone input error: {error}"),
                None,
            ),
            cpal::SampleFormat::U8 => device.build_input_stream(
                stream_config,
                move |data: &[u8], _| {
                    let peak = data
                        .iter()
                        .copied()
                        .map(|sample| (f32::from(sample) - 128.0) / 128.0)
                        .fold(0.0_f32, f32::max);
                    callback_level.store(peak.abs().to_bits(), Ordering::Relaxed);
                },
                |error| eprintln!("microphone input error: {error}"),
                None,
            ),
            _ => {
                return Err(AudioError::Configuration(format!(
                    "unsupported sample format {sample_format:?}"
                )));
            }
        }
        .map_err(|error| AudioError::Stream(error.to_string()))?;

        stream
            .play()
            .map_err(|error| AudioError::Stream(error.to_string()))?;
        Ok((stream, name))
    }

    /// Current level, or zero when unavailable.
    #[must_use]
    pub fn level(&self) -> f32 {
        self.level.get()
    }

    /// Current monitoring status.
    #[must_use]
    pub fn status(&self) -> &AudioStatus {
        &self.status
    }
}

/// Root-mean-square of a floating-point sample block.
#[must_use]
pub fn rms_f32(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|sample| sample * sample).sum();
    (sum / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_is_correct() {
        assert_eq!(rms_f32(&[]), 0.0);
        let value = rms_f32(&[3.0, 4.0, 0.0]);
        assert!((value - (25.0_f32 / 3.0).sqrt()).abs() < 0.0001);
    }

    #[test]
    fn level_is_clamped_and_shared() {
        let level = AudioLevel::new();
        let handle = level.clone();
        handle.set(2.0);
        assert_eq!(level.get(), 1.0);
        handle.set(-1.0);
        assert_eq!(level.get(), 0.0);
        handle.set(0.25);
        assert_eq!(level.get(), 0.25);
    }

    #[test]
    fn status_labels_are_human_readable() {
        assert_eq!(
            AudioStatus::Running("USB mic".into()).label(),
            "Microphone: USB mic"
        );
        assert_eq!(
            AudioStatus::Unavailable("no microphone found on this system".into()).label(),
            "Microphone unavailable: no microphone found on this system"
        );
    }
}
