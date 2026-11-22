//! Local recording with a documented fallback.
//!
//! When `ffmpeg` is available on `PATH`, the recorder pipes PNG frames into it
//! and writes an MKV or MP4 file. Otherwise it writes a numbered PNG sequence
//! to a directory, which is a real, offline recording that can be assembled
//! later. The UI reports which mode is active; it never claims video encoding
//! when ffmpeg is missing.

use std::path::{Path, PathBuf};

/// Container requested by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// Matroska, generally the most forgiving container.
    Mkv,
    /// MPEG-4 Part 14.
    Mp4,
}

impl Container {
    /// File extension for this container.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mkv => "mkv",
            Self::Mp4 => "mp4",
        }
    }

    /// ffmpeg muxer name.
    #[must_use]
    pub const fn muxer(self) -> &'static str {
        match self {
            Self::Mkv => "matroska",
            Self::Mp4 => "mp4",
        }
    }
}

/// Where recorded frames will end up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingPlan {
    /// Pipe PNG frames into ffmpeg, producing a video file.
    Ffmpeg {
        /// ffmpeg executable found on `PATH`.
        program: PathBuf,
        /// Target video file.
        output: PathBuf,
        /// Container to mux.
        container: Container,
    },
    /// Write numbered PNG files into a directory.
    PngSequence {
        /// Destination directory.
        directory: PathBuf,
    },
}

impl RecordingPlan {
    /// Human-readable description of the active recording mode.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::Ffmpeg {
                output, container, ..
            } => format!(
                "Recording video ({}) to {} via ffmpeg",
                container.extension(),
                output.display()
            ),
            Self::PngSequence { directory } => format!(
                "ffmpeg not found - recording PNG sequence to {}",
                directory.display()
            ),
        }
    }

    /// True when real video encoding is active. Used in tests.
    #[must_use]
    #[cfg(test)]
    pub const fn is_video(&self) -> bool {
        matches!(self, Self::Ffmpeg { .. })
    }
}

/// Search `PATH` for an executable named `ffmpeg`.
///
/// Accepts an injectable PATH string so the decision logic is testable without
/// touching the host environment.
#[must_use]
pub fn find_program(path_var: &str, program: &str) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

/// Choose a recording plan from the environment and the user's target path.
///
/// If ffmpeg is missing, the output path's parent is used as the PNG sequence
/// directory (falling back to the current directory).
#[must_use]
pub fn choose_plan(
    ffmpeg: Option<PathBuf>,
    output: PathBuf,
    container: Container,
    fps: u32,
) -> RecordingPlan {
    if let Some(program) = ffmpeg {
        let _ = fps; // fps is passed to ffmpeg when the session is started.
        RecordingPlan::Ffmpeg {
            program,
            output,
            container,
        }
    } else {
        let directory = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        RecordingPlan::PngSequence { directory }
    }
}

/// Name of frame `index` in a PNG sequence (one-based, zero padded).
#[must_use]
pub fn frame_name(index: u64) -> String {
    format!("frame_{index:06}.png")
}

/// A running recording session.
#[derive(Debug)]
pub struct RecordingSession {
    plan: RecordingPlan,
    frames: u64,
    process: Option<std::process::Child>,
}

impl RecordingSession {
    /// Create a session from a plan. Creates the PNG directory if needed.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the PNG destination directory cannot be
    /// created or ffmpeg cannot be spawned.
    pub fn start(plan: RecordingPlan, fps: u32) -> Result<Self, String> {
        let mut process = None;
        if let RecordingPlan::Ffmpeg {
            program,
            output,
            container,
        } = &plan
        {
            let child = std::process::Command::new(program)
                .args([
                    "-y",
                    "-f",
                    "image2pipe",
                    "-framerate",
                    &fps.to_string(),
                    "-vcodec",
                    "png",
                    "-i",
                    "pipe:0",
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-f",
                    container.muxer(),
                ])
                .arg(output)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|error| format!("could not start ffmpeg: {error}"))?;
            process = Some(child);
        } else if let RecordingPlan::PngSequence { directory } = &plan {
            std::fs::create_dir_all(directory)
                .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
        }
        Ok(Self {
            plan,
            frames: 0,
            process,
        })
    }

    /// Append one PNG-encoded frame.
    ///
    /// # Errors
    ///
    /// Returns an error when a PNG file cannot be written or the ffmpeg pipe
    /// is broken.
    pub fn push_frame(&mut self, png: &[u8]) -> Result<(), String> {
        self.frames = self.frames.saturating_add(1);
        match &self.plan {
            RecordingPlan::PngSequence { directory } => {
                let path = directory.join(frame_name(self.frames));
                std::fs::write(&path, png)
                    .map_err(|error| format!("could not write {}: {error}", path.display()))
            }
            RecordingPlan::Ffmpeg { .. } => {
                use std::io::Write;
                let child = self
                    .process
                    .as_mut()
                    .ok_or_else(|| "ffmpeg process is not running".to_owned())?;
                let stdin = child
                    .stdin
                    .as_mut()
                    .ok_or_else(|| "ffmpeg stdin is not available".to_owned())?;
                stdin
                    .write_all(png)
                    .map_err(|error| format!("could not feed ffmpeg: {error}"))
            }
        }
    }

    /// Finish the recording, closing ffmpeg and collecting its status.
    ///
    /// # Errors
    ///
    /// Returns an error when ffmpeg exits unsuccessfully.
    pub fn finish(mut self) -> Result<u64, String> {
        if let Some(mut child) = self.process.take() {
            drop(child.stdin.take());
            let output = child
                .wait_with_output()
                .map_err(|error| format!("could not finish ffmpeg: {error}"))?;
            if !output.status.success() {
                return Err(format!(
                    "ffmpeg failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }
        Ok(self.frames)
    }

    /// Number of frames accepted so far.
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    /// The active plan.
    #[must_use]
    pub const fn plan(&self) -> &RecordingPlan {
        &self.plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_names_are_zero_padded() {
        assert_eq!(frame_name(1), "frame_000001.png");
        assert_eq!(frame_name(42), "frame_000042.png");
    }

    #[test]
    fn falls_back_to_png_sequence_without_ffmpeg() {
        let plan = choose_plan(
            None,
            PathBuf::from("/tmp/out/movie.mp4"),
            Container::Mp4,
            30,
        );
        assert_eq!(
            plan,
            RecordingPlan::PngSequence {
                directory: PathBuf::from("/tmp/out")
            }
        );
        assert!(!plan.is_video());
        assert!(plan.description().contains("PNG sequence"));
    }

    #[test]
    fn uses_ffmpeg_when_available() {
        let plan = choose_plan(
            Some(PathBuf::from("/usr/bin/ffmpeg")),
            PathBuf::from("out.mkv"),
            Container::Mkv,
            30,
        );
        assert!(plan.is_video());
        assert!(plan.description().contains("mkv"));
    }

    #[test]
    fn find_program_respects_the_given_path() {
        let directory =
            std::env::temp_dir().join(format!("streamcraft-path-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&directory);
        let fake = directory.join("ffmpeg");
        let _ = std::fs::write(&fake, b"#!/bin/sh\n");
        let found = find_program(directory.to_string_lossy().as_ref(), "ffmpeg");
        let _ = std::fs::remove_file(&fake);
        let _ = std::fs::remove_dir(&directory);
        assert_eq!(found, Some(fake));
        assert_eq!(find_program("/nonexistent", "ffmpeg"), None);
    }

    #[test]
    fn png_sequence_session_writes_frames() {
        let directory =
            std::env::temp_dir().join(format!("streamcraft-rec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let plan = RecordingPlan::PngSequence {
            directory: directory.clone(),
        };
        let mut session = RecordingSession::start(plan, 10).expect("start");
        let png = crate::render::encode_png(&[255, 0, 0, 255], 1, 1);
        session.push_frame(&png).expect("frame 1");
        session.push_frame(&png).expect("frame 2");
        assert_eq!(session.frames(), 2);
        let count = session.finish().expect("finish");
        assert_eq!(count, 2);
        let first = std::fs::read(directory.join(frame_name(1))).expect("first frame");
        assert_eq!(first, png);
        assert!(directory.join(frame_name(2)).exists());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
