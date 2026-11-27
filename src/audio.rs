//! Microphone level monitoring with `cpal`.
//!
//! The audio thread computes a peak level into an atomic slot; the UI only
//! reads that slot. Device discovery is best-effort: on a machine without an
//! input device the status is reported as unavailable rather than faked.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

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
        self.publish(level);
    }

    fn publish(&self, level: f32) {
        let level = if level.is_finite() {
            level.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.0.store(level.to_bits(), Ordering::Relaxed);
    }

    /// Current level.
    #[must_use]
    pub fn get(&self) -> f32 {
        let level = f32::from_bits(self.0.load(Ordering::Relaxed));
        if level.is_finite() {
            level.clamp(0.0, 1.0)
        } else {
            0.0
        }
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

#[derive(Debug, Default)]
struct AudioFailureState {
    failed: AtomicBool,
    message: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Default)]
struct AudioFailure(Arc<AudioFailureState>);

impl AudioFailure {
    fn report(&self, error: impl std::fmt::Display) {
        if let Ok(mut slot) = self.0.message.lock() {
            *slot = Some(error.to_string());
        }
        self.0.failed.store(true, Ordering::Release);
    }

    fn clear(&self) {
        // The normal audio callback only touches an atomic. The mutex is used
        // solely on the error/recovery transition, never for every sample block.
        if self.0.failed.load(Ordering::Relaxed)
            && self.0.failed.swap(false, Ordering::AcqRel)
            && let Ok(mut slot) = self.0.message.lock()
            && !self.0.failed.load(Ordering::Acquire)
        {
            *slot = None;
        }
    }

    fn get(&self) -> Option<String> {
        if !self.0.failed.load(Ordering::Acquire) {
            return None;
        }
        self.0.message.lock().ok().and_then(|slot| slot.clone())
    }
}

/// A live microphone monitor. Keeps the `cpal` stream alive while in scope.
pub struct AudioMonitor {
    _stream: Option<cpal::Stream>,
    level: AudioLevel,
    status: AudioStatus,
    failure: AudioFailure,
}

impl AudioMonitor {
    /// Start monitoring the default input device, reporting failures instead
    /// of panicking. Safe to call on a headless machine.
    #[must_use]
    pub fn start() -> Self {
        let level = AudioLevel::new();
        let failure = AudioFailure::default();
        let (stream, status) = match Self::try_start(&level, &failure) {
            Ok((stream, device)) => (Some(stream), AudioStatus::Running(device)),
            Err(error) => (None, AudioStatus::Unavailable(error.to_string())),
        };
        Self {
            _stream: stream,
            level,
            status,
            failure,
        }
    }

    fn try_start(
        level: &AudioLevel,
        failure: &AudioFailure,
    ) -> Result<(cpal::Stream, String), AudioError> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(AudioError::NoDevice)?;
        let name = device.to_string();
        let config = device
            .default_input_config()
            .map_err(|error| AudioError::Configuration(error.to_string()))?;
        let sample_format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();
        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let callback_level = level.clone();
                let error_level = callback_level.clone();
                let callback_failure = failure.clone();
                let error_failure = failure.clone();
                device.build_input_stream(
                    stream_config,
                    move |data: &[f32], _| {
                        callback_failure.clear();
                        callback_level.publish(rms_f32(data));
                    },
                    move |error| {
                        error_level.publish(0.0);
                        error_failure.report(error);
                    },
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let callback_level = level.clone();
                let error_level = callback_level.clone();
                let callback_failure = failure.clone();
                let error_failure = failure.clone();
                device.build_input_stream(
                    stream_config,
                    move |data: &[i16], _| {
                        callback_failure.clear();
                        callback_level.publish(rms_i16(data));
                    },
                    move |error| {
                        error_level.publish(0.0);
                        error_failure.report(error);
                    },
                    None,
                )
            }
            cpal::SampleFormat::U8 => {
                let callback_level = level.clone();
                let error_level = callback_level.clone();
                let callback_failure = failure.clone();
                let error_failure = failure.clone();
                device.build_input_stream(
                    stream_config,
                    move |data: &[u8], _| {
                        callback_failure.clear();
                        callback_level.publish(rms_u8(data));
                    },
                    move |error| {
                        error_level.publish(0.0);
                        error_failure.report(error);
                    },
                    None,
                )
            }
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

    /// Current monitoring status, including asynchronous device errors.
    #[must_use]
    pub fn status(&self) -> AudioStatus {
        self.failure.get().map_or_else(
            || self.status.clone(),
            |error| AudioStatus::Unavailable(format!("input stream error: {error}")),
        )
    }
}

/// Root-mean-square of a floating-point sample block.
///
/// Non-finite samples are ignored so a faulty device cannot put `NaN` into the
/// meter. Arithmetic uses `f64` to avoid overflow while squaring.
#[must_use]
pub fn rms_f32(samples: &[f32]) -> f32 {
    let mut sum = 0.0_f64;
    let mut count = 0_usize;
    for &sample in samples {
        if sample.is_finite() {
            sum += f64::from(sample) * f64::from(sample);
            count += 1;
        }
    }
    rms_from_sum(sum, count)
}

/// Root-mean-square of signed 16-bit samples, normalized to `-1.0..=1.0`.
#[must_use]
pub fn rms_i16(samples: &[i16]) -> f32 {
    let mut sum = 0.0_f64;
    for &sample in samples {
        let sample = f64::from(sample) / 32_768.0;
        sum += sample * sample;
    }
    rms_from_sum(sum, samples.len())
}

/// Root-mean-square of unsigned 8-bit samples, centered at 128.
#[must_use]
pub fn rms_u8(samples: &[u8]) -> f32 {
    let mut sum = 0.0_f64;
    for &sample in samples {
        let sample = (f64::from(sample) - 128.0) / 128.0;
        sum += sample * sample;
    }
    rms_from_sum(sum, samples.len())
}

fn rms_from_sum(sum: f64, count: usize) -> f32 {
    if count == 0 {
        0.0
    } else {
        (sum / count as f64).sqrt() as f32
    }
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
    fn integer_formats_use_rms_instead_of_peak() {
        assert!((rms_i16(&[i16::MIN, 0]) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.0001);
        assert!((rms_u8(&[128, 192]) - std::f32::consts::FRAC_1_SQRT_2 * 0.5).abs() < 0.0001);
        assert_eq!(rms_i16(&[]), 0.0);
        assert_eq!(rms_u8(&[]), 0.0);
    }

    #[test]
    fn rms_ignores_non_finite_samples() {
        assert_eq!(rms_f32(&[f32::NAN, f32::INFINITY]), 0.0);
        assert!((rms_f32(&[3.0, f32::NAN, 4.0]) - 12.5_f32.sqrt()).abs() < 0.0001);
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
        handle.set(f32::NAN);
        assert_eq!(level.get(), 0.0);
    }

    #[test]
    fn asynchronous_errors_are_visible_and_recoverable() {
        let failure = AudioFailure::default();
        let handle = failure.clone();
        handle.report("device disconnected");
        assert_eq!(failure.get().as_deref(), Some("device disconnected"));
        handle.clear();
        assert_eq!(failure.get(), None);
    }

    #[test]
    fn missing_device_is_reported_without_panicking() {
        let monitor = AudioMonitor::start();
        assert!(monitor.level().is_finite());
        assert!(!monitor.status().label().is_empty());
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
