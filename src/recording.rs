//! Local recording with a documented fallback.
//!
//! When `ffmpeg` is available on `PATH`, the recorder pipes PNG frames into it
//! and writes an MKV or MP4 file. Otherwise it writes a numbered PNG sequence
//! to a new directory, which is a real, offline recording that can be assembled
//! later. The UI reports which mode is active; it never claims video encoding
//! when ffmpeg is missing.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::thread::JoinHandle;

use serde::Serialize;

/// Highest recording rate accepted by the recorder.
pub const MAX_RECORD_FPS: u32 = 120;
/// Maximum number of missed frames repeated after a long UI stall.
pub const MAX_CATCH_UP_FRAMES: u32 = 120;
const MAX_STDERR_BYTES: usize = 64 * 1024;

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
        /// Input frame rate.
        fps: u32,
    },
    /// Write numbered PNG files into a directory.
    PngSequence {
        /// Requested destination directory. A unique sibling is created at start.
        directory: PathBuf,
        /// Frame rate used for the input timeline and mux metadata.
        fps: u32,
    },
}

impl RecordingPlan {
    /// Human-readable description of the active recording mode.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::Ffmpeg {
                output,
                container,
                fps,
                ..
            } => format!(
                "Recording video ({}) at {fps} fps to {} via ffmpeg",
                container.extension(),
                output.display()
            ),
            Self::PngSequence { directory, fps } => format!(
                "ffmpeg not found - recording PNG sequence at {fps} fps to {}",
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

/// Search `PATH` for an executable named `program`.
///
/// Accepts an injectable PATH value so the decision logic is testable without
/// touching the host environment.
#[must_use]
pub fn find_program(path_var: impl AsRef<OsStr>, program: &str) -> Option<PathBuf> {
    std::env::split_paths(path_var.as_ref()).find_map(|directory| {
        let candidate = directory.join(program);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let candidate = directory.join(format!("{program}.exe"));
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
        None
    })
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Choose a recording plan from the environment and the user's target path.
///
/// If ffmpeg is missing, a directory derived from the output name is requested.
/// [`RecordingSession::start`] creates a unique sibling so an existing recording
/// is never silently overwritten.
#[must_use]
pub fn choose_plan(
    ffmpeg: Option<PathBuf>,
    output: PathBuf,
    container: Container,
    fps: u32,
) -> RecordingPlan {
    if let Some(program) = ffmpeg {
        RecordingPlan::Ffmpeg {
            program,
            output,
            container,
            fps,
        }
    } else {
        RecordingPlan::PngSequence {
            directory: output.with_extension("frames"),
            fps,
        }
    }
}

/// Name of frame `index` in a PNG sequence (one-based, zero padded).
#[must_use]
pub fn frame_name(index: u64) -> String {
    format!("frame_{index:06}.png")
}

/// Wall-clock recording cadence with exact sub-millisecond frame intervals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramePacer {
    fps: u32,
}

impl FramePacer {
    /// Validate a frame rate and create its pacer.
    ///
    /// # Errors
    ///
    /// Returns an error for zero or unreasonably high frame rates.
    pub fn new(fps: u32) -> Result<Self, String> {
        if fps == 0 || fps > MAX_RECORD_FPS {
            return Err(format!(
                "frame rate must be between 1 and {MAX_RECORD_FPS} fps"
            ));
        }
        Ok(Self { fps })
    }

    /// Exact interval between frames. For example, 60 fps is 16,666,667 ns,
    /// rather than the 16 ms produced by integer millisecond division.
    #[must_use]
    pub const fn interval(self) -> std::time::Duration {
        let fps = self.fps as u64;
        // Round up so the first scheduled repaint is never a fraction of a
        // nanosecond before the first frame's wall-clock deadline.
        std::time::Duration::from_nanos(1_000_000_000_u64.div_ceil(fps))
    }

    /// Number of frames due at `elapsed`, given how many were already written.
    ///
    /// The timeline is anchored to the recording start, so rendering time does
    /// not introduce cumulative drift. Catch-up is bounded to avoid a huge
    /// burst after the window has been suspended.
    #[must_use]
    pub fn frames_due(self, elapsed: std::time::Duration, recorded: u64) -> u32 {
        let fps = u128::from(self.fps);
        let target = u128::from(elapsed.as_secs()) * fps
            + u128::from(elapsed.subsec_nanos()) * fps / 1_000_000_000;
        let due = target.saturating_sub(u128::from(recorded));
        due.min(u128::from(MAX_CATCH_UP_FRAMES)) as u32
    }
}

/// A running recording session.
#[derive(Debug)]
pub struct RecordingSession {
    plan: RecordingPlan,
    frames: u64,
    process: Option<Child>,
    stderr: Option<JoinHandle<String>>,
}

impl RecordingSession {
    /// Create a session from a plan. Creates a unique PNG directory if needed.
    ///
    /// # Errors
    ///
    /// Returns an error when the frame rate is invalid, the PNG destination
    /// cannot be created, or ffmpeg cannot be spawned.
    pub fn start(mut plan: RecordingPlan, fps: u32) -> Result<Self, String> {
        FramePacer::new(fps)?;
        let mut process = None;
        let mut stderr = None;
        match &mut plan {
            RecordingPlan::Ffmpeg {
                program,
                output,
                container,
                fps: planned_fps,
            } => {
                *planned_fps = fps;
                let mut command = std::process::Command::new(program);
                command.args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "image2pipe",
                    "-framerate",
                    &fps.to_string(),
                    "-vcodec",
                    "png",
                    "-i",
                    "pipe:0",
                    "-an",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "veryfast",
                    "-pix_fmt",
                    "yuv420p",
                    "-f",
                    container.muxer(),
                ]);
                if *container == Container::Mp4 {
                    command.arg("-movflags").arg("+faststart");
                }
                let mut child = command
                    .arg(output)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|error| format!("could not start ffmpeg: {error}"))?;
                let Some(reader) = child.stderr.take() else {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("could not capture ffmpeg diagnostics".to_owned());
                };
                // ffmpeg can emit enough errors to fill an undrained pipe. Drain
                // concurrently, retaining only a bounded tail for the UI.
                stderr = Some(std::thread::spawn(move || drain_stderr(reader)));
                process = Some(child);
            }
            RecordingPlan::PngSequence {
                directory,
                fps: planned_fps,
            } => {
                *planned_fps = fps;
                *directory = create_unique_directory(directory)?;
            }
        }
        Ok(Self {
            plan,
            frames: 0,
            process,
            stderr,
        })
    }

    /// Append one PNG-encoded frame.
    ///
    /// # Errors
    ///
    /// Returns an error when a PNG file cannot be written or the ffmpeg pipe
    /// is broken. A failed frame is not included in [`Self::frames`].
    pub fn push_frame(&mut self, png: &[u8]) -> Result<(), String> {
        let next_frame = self.frames.saturating_add(1);
        match &self.plan {
            RecordingPlan::PngSequence { directory, .. } => {
                let path = directory.join(frame_name(next_frame));
                std::fs::write(&path, png)
                    .map_err(|error| format!("could not write {}: {error}", path.display()))?;
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
                    .map_err(|error| format!("could not feed ffmpeg: {error}"))?;
            }
        }
        self.frames = next_frame;
        Ok(())
    }

    /// Finish the recording, closing ffmpeg and collecting its status.
    ///
    /// # Errors
    ///
    /// Returns an error when no frames were written, the PNG metadata cannot be
    /// saved, or ffmpeg exits unsuccessfully.
    pub fn finish(mut self) -> Result<u64, String> {
        let mut status = None;
        if let Some(mut child) = self.process.take() {
            drop(child.stdin.take());
            status = Some(child.wait());
        }
        let diagnostics = self.take_stderr();
        if let Some(status) = status {
            let status = status.map_err(|error| {
                format!(
                    "could not finish ffmpeg: {error}{}",
                    diagnostic_suffix(&diagnostics)
                )
            })?;
            if !status.success() {
                return Err(format!(
                    "ffmpeg failed ({}){}",
                    status_label(status),
                    diagnostic_suffix(&diagnostics)
                ));
            }
        }
        if self.frames == 0 {
            return Err("no frames were recorded".to_owned());
        }
        if let RecordingPlan::PngSequence { directory, fps } = &self.plan {
            let metadata = serde_json::to_vec(&PngMetadata {
                fps: *fps,
                frames: self.frames,
                frame_pattern: "frame_%06d.png",
            })
            .map_err(|error| format!("could not encode recording metadata: {error}"))?;
            let path = directory.join("recording.json");
            std::fs::write(&path, metadata)
                .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        }
        Ok(self.frames)
    }

    fn take_stderr(&mut self) -> String {
        self.stderr
            .take()
            .and_then(|handle| handle.join().ok())
            .unwrap_or_else(|| "ffmpeg diagnostics unavailable".to_owned())
    }

    /// Number of frames accepted so far.
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    /// The active plan, including the unique PNG directory selected at start.
    #[must_use]
    pub const fn plan(&self) -> &RecordingPlan {
        &self.plan
    }
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.take() {
            // Child's default Drop does not terminate or reap it. This runs on
            // write failures, application shutdown and explicit cancellation.
            let _ = child.kill();
            let _ = child.wait();
        }
        // Joining after wait guarantees the pipe reader reaches EOF.
        if let Some(handle) = self.stderr.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Serialize)]
struct PngMetadata {
    fps: u32,
    frames: u64,
    frame_pattern: &'static str,
}

fn drain_stderr(mut reader: impl Read) -> String {
    let mut retained = VecDeque::with_capacity(MAX_STDERR_BYTES);
    let mut buffer = [0_u8; 4096];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return format!("could not read ffmpeg diagnostics: {error}"),
        };
        for &byte in &buffer[..count] {
            if retained.len() == MAX_STDERR_BYTES {
                retained.pop_front();
            }
            retained.push_back(byte);
        }
    }
    String::from_utf8_lossy(&retained.into_iter().collect::<Vec<_>>())
        .trim()
        .to_owned()
}

fn diagnostic_suffix(diagnostics: &str) -> String {
    if diagnostics.is_empty() {
        String::new()
    } else {
        format!(": {diagnostics}")
    }
}

fn status_label(status: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(code) = status.code() {
            return format!("exit {code}");
        }
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }
    status.to_string()
}

fn create_unique_directory(requested: &Path) -> Result<PathBuf, String> {
    if let Some(parent) = requested
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    for suffix in 0..1000 {
        let candidate = if suffix == 0 {
            requested.to_path_buf()
        } else {
            let name = requested.file_name().map_or_else(
                || "recording".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            requested.with_file_name(format!("{name}-{suffix}"))
        };
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("could not create {}: {error}", candidate.display()));
            }
        }
    }
    Err(format!(
        "could not find an unused recording directory near {}",
        requested.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::render::encode_png;

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "streamcraft-{label}-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn frame_names_are_zero_padded() {
        assert_eq!(frame_name(1), "frame_000001.png");
        assert_eq!(frame_name(42), "frame_000042.png");
    }

    #[test]
    fn frame_pacer_uses_exact_intervals_and_wall_clock_targets() {
        let pacer = FramePacer::new(60).expect("60 fps");
        assert_eq!(
            pacer.interval(),
            std::time::Duration::from_nanos(16_666_667)
        );
        assert_eq!(pacer.frames_due(std::time::Duration::from_millis(16), 0), 0);
        assert_eq!(pacer.frames_due(std::time::Duration::from_millis(20), 0), 1);
        assert_eq!(pacer.frames_due(std::time::Duration::from_secs(1), 59), 1);
        assert_eq!(pacer.frames_due(std::time::Duration::from_secs(1), 60), 0);
        assert_eq!(pacer.frames_due(std::time::Duration::from_secs(10), 0), 120);
        for fps in [1, 15, 24, 30, 60, 120] {
            let pacer = FramePacer::new(fps).expect("supported FPS");
            assert_eq!(pacer.frames_due(pacer.interval(), 0), 1, "FPS {fps}");
        }
        assert!(FramePacer::new(0).is_err());
        assert!(FramePacer::new(MAX_RECORD_FPS + 1).is_err());
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
                directory: PathBuf::from("/tmp/out/movie.frames"),
                fps: 30,
            }
        );
        assert!(!plan.is_video());
        assert!(plan.description().contains("PNG sequence"));
        assert!(plan.description().contains("30 fps"));
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
        assert!(plan.description().contains("30 fps"));
    }

    #[test]
    fn find_program_respects_the_given_path_and_executable_bit() {
        let directory = temp_path("path");
        std::fs::create_dir_all(&directory).expect("directory");
        let fake = directory.join("ffmpeg");
        std::fs::write(&fake, b"#!/bin/sh\n").expect("fake program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o644))
                .expect("non-executable permissions");
            assert_eq!(
                find_program(directory.to_string_lossy().as_ref(), "ffmpeg"),
                None
            );
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
                .expect("executable permissions");
        }
        let found = find_program(directory.to_string_lossy().as_ref(), "ffmpeg");
        let _ = std::fs::remove_dir_all(&directory);
        assert_eq!(found, Some(fake));
        assert_eq!(find_program("/nonexistent", "ffmpeg"), None);
    }

    #[test]
    fn png_sequence_session_writes_frames_and_metadata() {
        let directory = temp_path("recording");
        let plan = choose_plan(None, directory.join("movie.mp4"), Container::Mp4, 10);
        let mut session = RecordingSession::start(plan.clone(), 10).expect("start");
        let actual = match session.plan() {
            RecordingPlan::PngSequence { directory, .. } => directory.clone(),
            RecordingPlan::Ffmpeg { .. } => panic!("expected PNG plan"),
        };
        assert_eq!(actual, directory.join("movie.frames"));
        let png = encode_png(&[255, 0, 0, 255], 1, 1);
        session.push_frame(&png).expect("frame 1");
        session.push_frame(&png).expect("frame 2");
        assert_eq!(session.frames(), 2);
        let count = session.finish().expect("finish");
        assert_eq!(count, 2);
        assert_eq!(
            std::fs::read(actual.join(frame_name(1))).expect("first frame"),
            png
        );
        assert!(actual.join(frame_name(2)).exists());
        let metadata: serde_json::Value = serde_json::from_slice(
            &std::fs::read(actual.join("recording.json")).expect("metadata"),
        )
        .expect("metadata JSON");
        assert_eq!(metadata["fps"], 10);
        assert_eq!(metadata["frames"], 2);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn failed_png_writes_are_not_counted() {
        let root = temp_path("png-write-failure");
        let plan = choose_plan(None, root.join("movie.mp4"), Container::Mp4, 30);
        let mut session = RecordingSession::start(plan, 30).expect("start");
        let RecordingPlan::PngSequence { directory, .. } = session.plan() else {
            panic!("expected PNG plan");
        };
        std::fs::remove_dir_all(directory).expect("remove destination");
        let png = encode_png(&[0, 0, 0, 255], 1, 1);
        assert!(session.push_frame(&png).is_err());
        assert_eq!(session.frames(), 0);
        assert!(session.finish().is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn png_sequence_never_overwrites_an_existing_recording() {
        let root = temp_path("collision");
        std::fs::create_dir_all(root.join("movie.frames")).expect("existing directory");
        std::fs::write(root.join("movie.frames/frame_000001.png"), b"keep")
            .expect("existing frame");
        let plan = choose_plan(None, root.join("movie.mp4"), Container::Mp4, 30);
        let session = RecordingSession::start(plan, 30).expect("start");
        let RecordingPlan::PngSequence { directory, .. } = session.plan() else {
            panic!("expected PNG plan");
        };
        assert_eq!(directory, &root.join("movie.frames-1"));
        assert_eq!(
            std::fs::read(root.join("movie.frames/frame_000001.png")).expect("old frame"),
            b"keep"
        );
        drop(session);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stderr_is_drained_and_bounded() {
        let diagnostics = drain_stderr(std::io::repeat(b'x').take(200_000));
        assert_eq!(diagnostics.len(), MAX_STDERR_BYTES);
        assert!(diagnostics.chars().all(|character| character == 'x'));
    }

    #[test]
    #[cfg(unix)]
    fn ffmpeg_failure_is_reported() {
        let directory = temp_path("ffmpeg-failure");
        std::fs::create_dir_all(&directory).expect("directory");
        let program = directory.join("ffmpeg");
        std::fs::write(
            &program,
            b"#!/bin/sh\necho 'test encoder failure' >&2\nexit 3\n",
        )
        .expect("fake ffmpeg");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
                .expect("executable permissions");
        }
        let plan = RecordingPlan::Ffmpeg {
            program,
            output: directory.join("out.mkv"),
            container: Container::Mkv,
            fps: 10,
        };
        let mut session = RecordingSession::start(plan, 10).expect("start");
        let png = encode_png(&[0, 0, 0, 255], 1, 1);
        let _ = session.push_frame(&png);
        let error = session.finish().expect_err("ffmpeg must fail");
        assert!(error.contains("test encoder failure"), "{error}");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn ffmpeg_child_is_terminated_and_reaped_on_drop() {
        #[cfg(target_os = "linux")]
        {
            let directory = temp_path("ffmpeg-drop");
            std::fs::create_dir_all(&directory).expect("directory");
            let pid_file = directory.join("pid");
            let program = directory.join("ffmpeg");
            std::fs::write(
                &program,
                format!(
                    "#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n",
                    pid_file.display()
                ),
            )
            .expect("fake ffmpeg");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
                    .expect("executable permissions");
            }
            let plan = RecordingPlan::Ffmpeg {
                program,
                output: directory.join("out.mkv"),
                container: Container::Mkv,
                fps: 10,
            };
            let session = RecordingSession::start(plan, 10).expect("start");
            let mut pid = None;
            for _ in 0..100 {
                if let Ok(value) = std::fs::read_to_string(&pid_file) {
                    pid = Some(value);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let pid = pid.expect("child pid");
            drop(session);
            assert!(!Path::new(&format!("/proc/{pid}")).exists());
            let _ = std::fs::remove_dir_all(&directory);
        }
    }

    #[test]
    fn ffmpeg_session_encodes_real_frames_when_available() {
        let path_var = std::env::var_os("PATH").unwrap_or_default();
        let Some(program) = find_program(&path_var, "ffmpeg") else {
            return;
        };
        let directory = temp_path("ffmpeg-integration");
        std::fs::create_dir_all(&directory).expect("directory");
        let png = encode_png(&[255, 0, 0, 255].repeat(16 * 16), 16, 16);
        for container in [Container::Mkv, Container::Mp4] {
            let output = directory.join(format!("out.{}", container.extension()));
            let plan = RecordingPlan::Ffmpeg {
                program: program.clone(),
                output: output.clone(),
                container,
                fps: 10,
            };
            let mut session = RecordingSession::start(plan, 10).expect("start ffmpeg");
            for _ in 0..10 {
                session.push_frame(&png).expect("frame");
            }
            assert_eq!(session.finish().expect("finish ffmpeg"), 10);
            assert!(std::fs::metadata(&output).is_ok_and(|metadata| metadata.len() > 100));
            let status = std::process::Command::new(&program)
                .args(["-v", "error", "-i"])
                .arg(&output)
                .args(["-f", "null", "-"])
                .status()
                .expect("verify ffmpeg output");
            assert!(status.success());
            let progress = std::process::Command::new(&program)
                .args(["-v", "error", "-nostats", "-progress", "pipe:1", "-i"])
                .arg(&output)
                .args(["-f", "null", "-"])
                .output()
                .expect("count encoded frames");
            assert!(progress.status.success());
            let stdout = String::from_utf8_lossy(&progress.stdout);
            let frames = stdout
                .lines()
                .rev()
                .find_map(|line| line.strip_prefix("frame="))
                .expect("ffmpeg frame progress");
            // Ten frames at 10 fps verify the encoded one-second timeline and
            // ensure no frame was lost while flushing either container.
            assert_eq!(frames.trim(), "10");
        }
        let _ = std::fs::remove_dir_all(&directory);
    }
}
