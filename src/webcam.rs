//! Webcam device discovery.
//!
//! This tranche detects camera devices and reports their status. It does **not**
//! decode video frames: no camera backend is bundled, so the UI never claims a
//! working webcam preview. The detection logic is injectable and unit-tested
//! with synthetic device paths.

use std::path::{Path, PathBuf};

/// A discovered camera device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebcamDevice {
    /// Device node path, for example `/dev/video0`.
    pub path: PathBuf,
    /// Short display label.
    pub label: String,
}

impl WebcamDevice {
    /// Build a device from a path.
    #[must_use]
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("camera")
            .to_owned();
        Self { path, label }
    }
}

/// Status of webcam support on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebcamStatus {
    /// No camera device was found.
    NoDevice,
    /// Cameras were found, but frame decoding is not available in this build.
    DevicesWithoutDecoder(Vec<WebcamDevice>),
}

impl WebcamStatus {
    /// User-facing description. Never claims frames are being captured.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::NoDevice => "Webcam: no device detected".to_owned(),
            Self::DevicesWithoutDecoder(devices) => format!(
                "Webcam: {} device(s) detected ({}); frame capture is not implemented in this build",
                devices.len(),
                devices
                    .iter()
                    .map(|device| device.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// Check whether a name looks like a Linux video device node.
///
/// This is deliberately strict: only `videoN` with a numeric suffix is
/// accepted, so unrelated nodes under `/dev` are not reported as cameras.
#[must_use]
pub fn is_video_device_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix("video") else {
        return false;
    };
    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
}

/// Build a status from an iterator of candidate paths. Pure; no filesystem
/// access, so it is fully unit-testable.
#[must_use]
pub fn status_from_paths(paths: impl IntoIterator<Item = PathBuf>) -> WebcamStatus {
    let mut devices: Vec<WebcamDevice> = paths
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_video_device_name)
        })
        .map(WebcamDevice::from_path)
        .collect();
    devices.sort_by(|left, right| left.path.cmp(&right.path));
    devices.dedup_by(|left, right| left.path == right.path);
    if devices.is_empty() {
        WebcamStatus::NoDevice
    } else {
        WebcamStatus::DevicesWithoutDecoder(devices)
    }
}

/// Scan `/dev` for video device nodes. Missing or unreadable directories are
/// reported as "no device" rather than producing an error.
#[must_use]
pub fn probe_devices() -> WebcamStatus {
    let mut paths = Vec::new();
    if let Ok(entries) = std::fs::read_dir(Path::new("/dev")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() || path.exists() {
                paths.push(path);
            }
        }
    }
    status_from_paths(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_only_numbered_video_nodes() {
        assert!(is_video_device_name("video0"));
        assert!(is_video_device_name("video12"));
        assert!(!is_video_device_name("video"));
        assert!(!is_video_device_name("video0a"));
        assert!(!is_video_device_name("vga"));
    }

    #[test]
    fn status_from_paths_sorts_and_deduplicates() {
        let status = status_from_paths([
            PathBuf::from("/dev/video1"),
            PathBuf::from("/dev/video0"),
            PathBuf::from("/dev/video0"),
            PathBuf::from("/dev/null"),
        ]);
        let WebcamStatus::DevicesWithoutDecoder(devices) = status else {
            panic!("expected devices");
        };
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].label, "video0");
        assert_eq!(devices[1].path, PathBuf::from("/dev/video1"));
    }

    #[test]
    fn status_description_never_claims_capture() {
        let description = WebcamStatus::NoDevice.description();
        assert!(description.contains("no device"));
        let description = status_from_paths([PathBuf::from("/dev/video0")]).description();
        assert!(description.contains("not implemented"));
    }

    #[test]
    fn probe_does_not_panic() {
        let _ = probe_devices();
    }
}
