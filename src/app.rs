//! StreamCraft UI, menus, actions and project lifecycle.
//!
//! Canvas drawing uses egui's `Painter`; exports and recording reuse the pure
//! renderer in [`crate::render`]. Capture is platform-abstracted in
//! [`crate::capture`] and reports its real status to the user.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use craftkit_io::DocumentHost;
use craftkit_ui::cli::CliOptions;
use craftkit_ui::eframe::{self, egui};
use craftkit_ui::egui::{
    Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
    ViewportCommand,
};
use craftkit_ui::{AppIdentity, AppShell, Tone, dialogs, theme, widgets};

use crate::audio::{AudioMonitor, AudioStatus};
use crate::capture::{CaptureBackend, CaptureTarget, default_backend};
use crate::document::{
    CANVAS_H, CANVAS_W, Color, Document, FORMAT, SCHEMA_VERSION, SMPTE_BARS, SMPTE_LOWER, Scene,
    SourceKind, Transition, TransitionKind,
};
use crate::recording::{Container, FramePacer, RecordingSession, choose_plan, find_program};
use crate::render::{render_scene_png, render_transition_png};
use crate::webcam::probe_devices;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const CAPTURE_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    New,
    Open,
    Save,
    SaveAs,
    ExportPng,
    ExportSvg,
    Undo,
    Redo,
    NextScene,
    PreviousScene,
    ToggleFullscreen,
    ToggleCapture,
    StartRecording,
    StopRecording,
    About,
    Settings,
}

#[derive(Debug, Clone)]
enum PendingAction {
    New,
    Open(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingChoice {
    Save,
    Discard,
    Cancel,
}

#[derive(Debug, Clone, Copy)]
struct DragState {
    source_id: u64,
    grab_x: f32,
    grab_y: f32,
}

struct CapturePreview {
    texture: Option<egui::TextureHandle>,
    last_attempt: Instant,
    error: Option<String>,
}

/// The application state.
pub struct StreamCraftApp {
    shell: AppShell,
    host: DocumentHost<Document>,
    selected_source: Option<u64>,
    drag: Option<DragState>,
    property_gesture: bool,
    transition_gesture: bool,
    pending: Option<PendingAction>,
    capture_backend: Box<dyn CaptureBackend>,
    capture_target: CaptureTarget,
    capture_targets: Vec<CaptureTarget>,
    capture_enabled: bool,
    capture_preview: CapturePreview,
    audio: AudioMonitor,
    webcam: crate::webcam::WebcamStatus,
    recording: Option<RecordingSession>,
    record_container: Container,
    record_fps: u32,
    record_started: Instant,
    record_pacer: Option<FramePacer>,
    fullscreen: bool,
    transition_start: Option<Instant>,
    transition_from: Option<Scene>,
    transition: Transition,
}

impl StreamCraftApp {
    /// Build the application, opening the startup project when requested.
    pub fn new(context: &eframe::CreationContext<'_>, options: CliOptions) -> Self {
        let identity = AppIdentity {
            id: "streamcraft",
            name: "StreamCraft",
            version: APP_VERSION,
            tagline: "Scene compositor with local capture - not a streaming encoder",
            repository: Some("https://github.com/emilio-n8n/streamcraft"),
        };
        let shell = AppShell::new(identity);
        let document = if options.sample {
            Document::sample()
        } else {
            Document::default()
        };
        let transition = document
            .selected_scene()
            .map_or(Transition::default(), |scene| scene.transition);
        let capture_backend = default_backend();
        let capture_targets = capture_backend.targets().unwrap_or_default();
        let mut app = Self {
            shell,
            host: DocumentHost::new(document, FORMAT, SCHEMA_VERSION, APP_VERSION),
            selected_source: None,
            drag: None,
            property_gesture: false,
            transition_gesture: false,
            pending: None,
            capture_backend,
            capture_target: CaptureTarget::Screen,
            capture_targets,
            capture_enabled: false,
            capture_preview: CapturePreview {
                texture: None,
                last_attempt: Instant::now(),
                error: None,
            },
            audio: AudioMonitor::start(),
            webcam: probe_devices(),
            recording: None,
            record_container: Container::Mkv,
            record_fps: 30,
            record_started: Instant::now(),
            record_pacer: None,
            fullscreen: false,
            transition_start: None,
            transition_from: None,
            transition,
        };
        app.shell.install_theme(&context.egui_ctx);
        if let Some(path) = options.open {
            app.load_path(path);
        }
        app
    }

    fn load_path(&mut self, path: PathBuf) {
        match DocumentHost::from_file(path.clone(), FORMAT, SCHEMA_VERSION, APP_VERSION) {
            Ok(host) => {
                self.host = host;
                self.selected_source = None;
                self.clear_transition();
                self.transition = self
                    .host
                    .document()
                    .selected_scene()
                    .map_or(Transition::default(), |scene| scene.transition);
                self.shell.set_status(format!("Opened {}", path.display()));
            }
            Err(error) => {
                self.shell
                    .notify(Tone::Error, "Could not open project", error.to_string())
            }
        }
    }

    fn new_document(&mut self) {
        self.host.reset(Document::default());
        self.selected_source = None;
        self.clear_transition();
        self.transition = Transition::default();
        self.shell.set_status("New document");
    }

    fn clear_transition(&mut self) {
        self.transition_start = None;
        self.transition_from = None;
    }

    fn finish_property_gesture(&mut self) {
        if self.property_gesture {
            self.host.history_mut().end_gesture();
            self.property_gesture = false;
        }
    }

    fn finish_transition_gesture(&mut self) {
        if self.transition_gesture {
            self.host.history_mut().end_gesture();
            self.transition_gesture = false;
        }
    }

    fn finish_gesture(&mut self) {
        if self.drag.take().is_some() {
            self.host.history_mut().end_gesture();
        }
        self.finish_property_gesture();
        self.finish_transition_gesture();
    }

    fn begin_property_gesture(&mut self) {
        self.finish_transition_gesture();
        if !self.property_gesture {
            self.host.history_mut().begin_gesture();
            self.property_gesture = true;
        }
    }

    fn begin_transition_gesture(&mut self) {
        self.finish_property_gesture();
        if !self.transition_gesture {
            self.host.history_mut().begin_gesture();
            self.transition_gesture = true;
        }
    }

    fn edit_transform(&mut self, id: u64, x: f32, y: f32, w: f32, h: f32) {
        self.begin_property_gesture();
        self.host
            .history_mut()
            .current_mut()
            .set_transform(id, x, y, w, h);
    }

    fn edit_opacity(&mut self, id: u64, opacity: f32) {
        self.begin_property_gesture();
        self.host
            .history_mut()
            .current_mut()
            .set_opacity(id, opacity);
    }

    fn edit_transition(&mut self, index: usize, transition: Transition) {
        self.finish_gesture();
        self.host.apply(|document| {
            if let Some(scene) = document.scenes.get_mut(index) {
                scene.transition = transition;
            }
        });
        if index == self.host.document().selected_scene {
            self.transition = transition;
        }
    }

    fn edit_text(&mut self, id: u64, text: String) {
        self.begin_property_gesture();
        if let Some(source) = self
            .host
            .history_mut()
            .current_mut()
            .selected_scene_mut()
            .and_then(|scene| scene.source_mut(id))
        {
            source.text = text;
        }
    }

    fn edit_name(&mut self, id: u64, name: String) {
        self.begin_property_gesture();
        if let Some(source) = self
            .host
            .history_mut()
            .current_mut()
            .selected_scene_mut()
            .and_then(|scene| scene.source_mut(id))
        {
            source.name = name;
        }
    }

    fn save(&mut self) -> bool {
        self.finish_gesture();
        if self.host.path().is_none() {
            return self.save_as();
        }
        match self.host.save() {
            Ok(path) => {
                self.shell.set_status(format!("Saved {}", path.display()));
                true
            }
            Err(error) => {
                self.shell
                    .notify(Tone::Error, "Could not save project", error.to_string());
                false
            }
        }
    }

    fn save_as(&mut self) -> bool {
        self.finish_gesture();
        let default_name = format!("{}.json", slugify(&self.host.document().title));
        let Some(path) = dialogs::save_project("Save project", &default_name, "json") else {
            return false;
        };
        match self.host.save_as(path) {
            Ok(path) => {
                self.shell.set_status(format!("Saved {}", path.display()));
                true
            }
            Err(error) => {
                self.shell
                    .notify(Tone::Error, "Could not save project", error.to_string());
                false
            }
        }
    }

    fn export(&mut self, png: bool) {
        self.finish_gesture();
        let (default_name, bytes) = {
            let document = self.host.document();
            let Some(scene) = document.selected_scene() else {
                self.shell
                    .notify(Tone::Warning, "Nothing to export", "Create a scene first.");
                return;
            };
            let extension = if png { "png" } else { "svg" };
            let name = format!("{}.{}", slugify(&scene.name), extension);
            let bytes = if png {
                render_scene_png(scene)
            } else {
                document.export_svg().unwrap_or_default().into_bytes()
            };
            (name, bytes)
        };
        let (title, extension) = if png {
            ("Export scene as PNG", "png")
        } else {
            ("Export scene as SVG", "svg")
        };
        let Some(path) = dialogs::save_project(title, &default_name, extension) else {
            return;
        };
        match std::fs::write(&path, &bytes) {
            Ok(()) => {
                self.shell
                    .set_status(format!("Exported {}", path.display()));
                self.shell.notify(
                    Tone::Success,
                    "Export complete",
                    format!("Wrote {} bytes to {}", bytes.len(), path.display()),
                );
            }
            Err(error) => {
                self.shell
                    .notify(Tone::Error, "Could not export scene", error.to_string())
            }
        }
    }

    fn request_new(&mut self) {
        self.finish_gesture();
        if self.host.is_dirty() {
            self.pending = Some(PendingAction::New);
        } else {
            self.new_document();
        }
    }

    fn request_open(&mut self) {
        self.finish_gesture();
        let Some(path) = dialogs::open_project("Open project", &["json"]) else {
            return;
        };
        if self.host.is_dirty() {
            self.pending = Some(PendingAction::Open(path));
        } else {
            self.load_path(path);
        }
    }

    fn perform(&mut self, action: PendingAction) {
        match action {
            PendingAction::New => self.new_document(),
            PendingAction::Open(path) => self.load_path(path),
        }
    }

    fn resolve_pending(&mut self, choice: PendingChoice) {
        let Some(action) = self.pending.take() else {
            return;
        };
        match choice {
            PendingChoice::Cancel => {}
            PendingChoice::Discard => self.perform(action),
            PendingChoice::Save => {
                if self.save() {
                    self.perform(action);
                } else {
                    self.pending = Some(action);
                }
            }
        }
    }

    fn clamp_selection(&mut self) {
        self.host
            .history_mut()
            .current_mut()
            .clamp_scene_selection();
        if self
            .selected_source
            .is_some_and(|id| self.host.document().source(id).is_none())
        {
            self.selected_source = None;
        }
    }

    fn switch_scene(&mut self, next: bool) {
        self.finish_gesture();
        let document = self.host.document();
        let count = document.scenes.len();
        if count < 2 {
            return;
        }
        let current = document.selected_scene;
        let target = if next {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        if target == current {
            return;
        }
        let from = document.selected_scene().cloned();
        let transition = document.scenes[target].transition;
        if self.host.apply(|document| {
            if next {
                document.next_scene();
            } else {
                document.previous_scene();
            }
        }) {
            self.selected_source = None;
            self.transition = transition;
            self.transition_from = from;
            self.transition_start = Some(Instant::now());
            self.shell.set_status(format!(
                "Scene {} - {} transition",
                target + 1,
                self.transition.kind.label()
            ));
        }
    }

    fn select_scene(&mut self, index: usize) {
        self.finish_gesture();
        let document = self.host.document();
        if index == document.selected_scene || index >= document.scenes.len() {
            return;
        }
        let from = document.selected_scene().cloned();
        let transition = document.scenes[index].transition;
        if self.host.apply(|document| {
            document.select_scene(index);
        }) {
            self.selected_source = None;
            self.transition = transition;
            self.transition_from = from;
            self.transition_start = Some(Instant::now());
        }
    }

    fn refresh_capture_targets(&mut self) {
        match self.capture_backend.targets() {
            Ok(targets) => {
                self.capture_targets = targets;
                if !self.capture_targets.contains(&self.capture_target) {
                    self.capture_target = self
                        .capture_targets
                        .first()
                        .cloned()
                        .unwrap_or(CaptureTarget::Screen);
                }
                self.capture_preview.error = None;
            }
            Err(error) => self.capture_preview.error = Some(error.to_string()),
        }
        self.capture_preview.texture = None;
    }

    fn toggle_capture(&mut self) {
        self.capture_enabled = !self.capture_enabled;
        if self.capture_enabled {
            self.refresh_capture_targets();
            self.capture_preview.last_attempt = Instant::now() - CAPTURE_INTERVAL;
            let available = self.capture_backend.available();
            self.shell.set_status(format!(
                "Capture preview via {} ({}) - verify on this display",
                self.capture_backend.name(),
                if available { "available" } else { "no display" }
            ));
        } else {
            self.capture_preview.texture = None;
            self.capture_preview.error = None;
            self.shell.set_status("Capture preview stopped");
        }
    }

    fn poll_capture(&mut self, ctx: &egui::Context) {
        if !self.capture_enabled || self.capture_preview.last_attempt.elapsed() < CAPTURE_INTERVAL {
            return;
        }
        self.capture_preview.last_attempt = Instant::now();
        match self.capture_backend.capture(&self.capture_target) {
            Ok(frame) => {
                let image = frame.to_color_image();
                match self.capture_preview.texture.as_mut() {
                    Some(texture) => texture.set(image, egui::TextureOptions::NEAREST),
                    None => {
                        self.capture_preview.texture = Some(ctx.load_texture(
                            "streamcraft-capture",
                            image,
                            egui::TextureOptions::NEAREST,
                        ));
                    }
                }
                self.capture_preview.error = None;
            }
            Err(error) => {
                // Never leave a stale frame on screen after a target or display
                // server disappears.
                self.capture_preview.texture = None;
                self.capture_preview.error = Some(error.to_string());
            }
        }
    }

    fn start_recording(&mut self) {
        self.finish_gesture();
        if self.recording.is_some() {
            self.shell.notify(
                Tone::Warning,
                "Already recording",
                "Stop the current recording first.",
            );
            return;
        }
        let Ok(pacer) = FramePacer::new(self.record_fps) else {
            self.shell.notify(
                Tone::Error,
                "Could not start recording",
                "Choose a frame rate between 1 and 120 fps.",
            );
            return;
        };
        let extension = self.record_container.extension();
        let default_name = format!("recording.{extension}");
        let Some(output) = dialogs::save_project("Start recording", &default_name, extension)
        else {
            return;
        };
        let path_var = std::env::var_os("PATH").unwrap_or_default();
        let program = find_program(&path_var, "ffmpeg");
        let plan = choose_plan(program, output, self.record_container, self.record_fps);
        match RecordingSession::start(plan, self.record_fps) {
            Ok(session) => {
                self.shell
                    .set_status(format!("Recording: {}", session.plan().description()));
                self.recording = Some(session);
                self.record_started = Instant::now();
                self.record_pacer = Some(pacer);
            }
            Err(error) => self
                .shell
                .notify(Tone::Error, "Could not start recording", error),
        }
    }

    fn stop_recording(&mut self) {
        let Some(session) = self.recording.take() else {
            return;
        };
        self.record_pacer = None;
        match session.finish() {
            Ok(frames) => {
                self.shell
                    .set_status(format!("Recording stopped ({frames} frames)"));
                self.shell.notify(
                    Tone::Success,
                    "Recording saved",
                    format!("{frames} frames written"),
                );
            }
            Err(error) => self.shell.notify(Tone::Error, "Recording failed", error),
        }
    }

    fn poll_recording(&mut self) {
        let Some(pacer) = self.record_pacer else {
            return;
        };
        let Some(session) = self.recording.as_mut() else {
            return;
        };
        let due = pacer.frames_due(self.record_started.elapsed(), session.frames());
        if due == 0 {
            return;
        }
        let Some(scene) = self.host.document().selected_scene().cloned() else {
            return;
        };
        let png = self.transition_from.as_ref().map_or_else(
            || render_scene_png(&scene),
            |from| {
                let progress = self.transition_start.map_or(1.0, |start| {
                    self.transition
                        .progress(u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX))
                });
                render_transition_png(from, &scene, progress)
            },
        );
        for _ in 0..due {
            if let Err(error) = session.push_frame(&png) {
                self.shell.notify(Tone::Error, "Recording stopped", error);
                // Dropping the session terminates and reaps ffmpeg, or leaves
                // the already-written PNG frames available for manual muxing.
                self.recording = None;
                self.record_pacer = None;
                break;
            }
        }
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::New => self.request_new(),
            Action::Open => self.request_open(),
            Action::Save => {
                self.save();
            }
            Action::SaveAs => {
                self.save_as();
            }
            Action::ExportPng => self.export(true),
            Action::ExportSvg => self.export(false),
            Action::Undo => {
                self.finish_gesture();
                if self.host.undo() {
                    self.clear_transition();
                    self.clamp_selection();
                    self.shell.set_status("Undo");
                }
            }
            Action::Redo => {
                self.finish_gesture();
                if self.host.redo() {
                    self.clear_transition();
                    self.clamp_selection();
                    self.shell.set_status("Redo");
                }
            }
            Action::NextScene => self.switch_scene(true),
            Action::PreviousScene => self.switch_scene(false),
            Action::ToggleFullscreen => self.fullscreen = !self.fullscreen,
            Action::ToggleCapture => self.toggle_capture(),
            Action::StartRecording => self.start_recording(),
            Action::StopRecording => self.stop_recording(),
            Action::About => self.shell.open_about(),
            Action::Settings => self.shell.open_settings(),
        }
    }

    fn handle_shortcuts(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let action = ui.input_mut(|input| {
            if input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::S,
            ) {
                Some(Action::SaveAs)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::S) {
                Some(Action::Save)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::O) {
                Some(Action::Open)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::N) {
                Some(Action::New)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z) {
                Some(Action::Undo)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Y) {
                Some(Action::Redo)
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::PageDown) {
                Some(Action::NextScene)
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::PageUp) {
                Some(Action::PreviousScene)
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::F11) {
                Some(Action::ToggleFullscreen)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::R) {
                Some(Action::StartRecording)
            } else {
                None
            }
        });
        if let Some(action) = action {
            self.handle_action(action);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let title = self.host.display_name();
        let dirty = self.host.is_dirty();
        let can_undo = self.host.can_undo();
        let can_redo = self.host.can_redo();
        let recording = self.recording.is_some();
        let capturing = self.capture_enabled;
        let mut action = None;
        self.shell.top_bar(ui, &title, dirty, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    action = Some(Action::New);
                    ui.close();
                }
                if ui.button("Open...").clicked() {
                    action = Some(Action::Open);
                    ui.close();
                }
                if ui.button("Save").clicked() {
                    action = Some(Action::Save);
                    ui.close();
                }
                if ui.button("Save As...").clicked() {
                    action = Some(Action::SaveAs);
                    ui.close();
                }
                ui.separator();
                if ui.button("Export scene as PNG...").clicked() {
                    action = Some(Action::ExportPng);
                    ui.close();
                }
                if ui.button("Export scene as SVG...").clicked() {
                    action = Some(Action::ExportSvg);
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui
                    .add_enabled(can_undo, egui::Button::new("Undo"))
                    .clicked()
                {
                    action = Some(Action::Undo);
                    ui.close();
                }
                if ui
                    .add_enabled(can_redo, egui::Button::new("Redo"))
                    .clicked()
                {
                    action = Some(Action::Redo);
                    ui.close();
                }
            });
            ui.menu_button("Scene", |ui| {
                if ui.button("Previous (PageUp)").clicked() {
                    action = Some(Action::PreviousScene);
                    ui.close();
                }
                if ui.button("Next (PageDown)").clicked() {
                    action = Some(Action::NextScene);
                    ui.close();
                }
                if ui.button("Toggle fullscreen (F11)").clicked() {
                    action = Some(Action::ToggleFullscreen);
                    ui.close();
                }
            });
            ui.menu_button("Capture", |ui| {
                let label = if capturing {
                    "Stop capture preview"
                } else {
                    "Start capture preview"
                };
                if ui.button(label).clicked() {
                    action = Some(Action::ToggleCapture);
                    ui.close();
                }
                let label = if recording {
                    "Stop recording"
                } else {
                    "Start recording (Ctrl+R)"
                };
                if ui.button(label).clicked() {
                    action = Some(if recording {
                        Action::StopRecording
                    } else {
                        Action::StartRecording
                    });
                    ui.close();
                }
            });
            if ui.button("About").clicked() {
                action = Some(Action::About);
            }
            if ui.button("Settings").clicked() {
                action = Some(Action::Settings);
            }
        });
        if let Some(action) = action {
            self.handle_action(action);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let mic_status = self.audio.status().label();
        let level = self.audio.level();
        let webcam = self.webcam.description();
        let recording = self
            .recording
            .as_ref()
            .map_or("idle".to_owned(), |session| {
                format!("{} frames", session.frames())
            });
        self.shell.status_bar_with(ui, |ui| {
            ui.label(RichText::new(mic_status).size(10.0).color(theme::MUTED));
            ui.add(
                egui::ProgressBar::new(level)
                    .desired_width(60.0)
                    .show_percentage(),
            );
            ui.label(RichText::new(webcam).size(10.0).color(theme::FAINT));
            ui.label(
                RichText::new(format!("Recording: {recording}"))
                    .size(10.0)
                    .color(theme::MUTED),
            );
        });
    }

    fn scenes_panel(&mut self, ui: &mut egui::Ui) {
        let scenes: Vec<(usize, String, Transition)> = self
            .host
            .document()
            .scenes
            .iter()
            .enumerate()
            .map(|(index, scene)| (index, scene.name.clone(), scene.transition))
            .collect();
        let selected_scene = self.host.document().selected_scene;
        let mut end_transition_gesture = false;

        egui::Panel::left("streamcraft-scenes")
            .resizable(true)
            .default_size(210.0)
            .min_size(150.0)
            .show(ui, |ui| {
                widgets::section_label(ui, "Scenes");
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for (index, name, _) in &scenes {
                            if ui
                                .selectable_label(*index == selected_scene, name)
                                .clicked()
                            {
                                self.select_scene(*index);
                            }
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Add scene").clicked() {
                        self.finish_gesture();
                        let from = self.host.document().selected_scene().cloned();
                        self.host.apply(|document| {
                            document.add_scene();
                        });
                        self.selected_source = None;
                        self.transition = self
                            .host
                            .document()
                            .selected_scene()
                            .map_or(Transition::default(), |scene| scene.transition);
                        self.transition_from = from;
                        self.transition_start = Some(Instant::now());
                        self.shell.set_status("Scene added");
                    }
                    if ui.button("Remove").clicked() {
                        self.finish_gesture();
                        let index = self.host.document().selected_scene;
                        let from = self.host.document().selected_scene().cloned();
                        self.host.apply(|document| {
                            document.remove_scene(index);
                        });
                        self.clamp_selection();
                        self.transition = self
                            .host
                            .document()
                            .selected_scene()
                            .map_or(Transition::default(), |scene| scene.transition);
                        if self.host.document().selected_scene != index {
                            self.transition_from = from;
                            self.transition_start = Some(Instant::now());
                        } else {
                            self.clear_transition();
                        }
                    }
                });
                ui.separator();
                widgets::section_label(ui, "Scene transition");
                let current = scenes
                    .iter()
                    .find(|(index, _, _)| *index == selected_scene)
                    .map(|(_, _, transition)| *transition)
                    .unwrap_or_default();
                let mut kind = current.kind;
                egui::ComboBox::from_label("On enter")
                    .selected_text(kind.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut kind, TransitionKind::Cut, "Cut");
                        ui.selectable_value(&mut kind, TransitionKind::Fade, "Fade");
                    });
                if kind != current.kind {
                    self.edit_transition(
                        selected_scene,
                        Transition {
                            kind,
                            duration_ms: current.duration_ms,
                        },
                    );
                }
                let mut duration = current.duration_ms;
                let response = ui.add(
                    egui::DragValue::new(&mut duration)
                        .prefix("ms ")
                        .speed(10.0)
                        .range(0..=10_000),
                );
                if response.changed() {
                    self.begin_transition_gesture();
                    if let Some(scene) = self
                        .host
                        .history_mut()
                        .current_mut()
                        .scenes
                        .get_mut(selected_scene)
                    {
                        scene.transition.duration_ms = duration;
                    }
                    self.transition.duration_ms = duration;
                }
                end_transition_gesture |= response.drag_stopped() || response.lost_focus();
            });
        if end_transition_gesture {
            self.finish_transition_gesture();
        }
    }

    fn sources_panel(&mut self, ui: &mut egui::Ui) {
        let sources: Vec<(u64, String, SourceKind, bool)> = self
            .host
            .document()
            .selected_scene()
            .map(|scene| {
                scene
                    .sources
                    .iter()
                    .map(|source| (source.id, source.name.clone(), source.kind, source.visible))
                    .collect()
            })
            .unwrap_or_default();
        let selected_source = self.selected_source;

        egui::Panel::left("streamcraft-sources")
            .resizable(true)
            .default_size(210.0)
            .min_size(150.0)
            .show(ui, |ui| {
                widgets::section_label(ui, "Sources (z order, top first)");
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (id, name, kind, visible) in sources.iter().rev() {
                            ui.horizontal(|ui| {
                                let label = format!("{}  {}", name, kind.label());
                                if ui
                                    .selectable_label(selected_source == Some(*id), label)
                                    .clicked()
                                {
                                    self.selected_source = Some(*id);
                                }
                                let id = *id;
                                if ui.small_button("^").on_hover_text("Raise").clicked() {
                                    self.finish_gesture();
                                    self.host.apply(|document| {
                                        document.move_source_z(id, 1);
                                    });
                                }
                                if ui.small_button("v").on_hover_text("Lower").clicked() {
                                    self.finish_gesture();
                                    self.host.apply(|document| {
                                        document.move_source_z(id, -1);
                                    });
                                }
                                let mut visible = *visible;
                                if ui.checkbox(&mut visible, "").changed() {
                                    self.finish_gesture();
                                    self.host.apply(|document| {
                                        if let Some(source) = document
                                            .selected_scene_mut()
                                            .and_then(|scene| scene.source_mut(id))
                                        {
                                            source.visible = visible;
                                        }
                                    });
                                }
                            });
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Colour").clicked() {
                        self.add_source(SourceKind::Color);
                    }
                    if ui.button("Pattern").clicked() {
                        self.add_source(SourceKind::TestPattern);
                    }
                    if ui.button("Text").clicked() {
                        self.add_source(SourceKind::Text);
                    }
                });
                ui.separator();
                widgets::section_label(ui, "Devices");
                if ui.button("Rescan webcams").clicked() {
                    self.webcam = probe_devices();
                    self.shell.set_status("Webcam devices rescanned");
                }
            });
    }

    fn add_source(&mut self, kind: SourceKind) {
        self.finish_gesture();
        let mut created = None;
        self.host.apply(|document| {
            created = document.add_source(kind);
        });
        self.selected_source = created;
        self.shell
            .set_status(format!("{} source added", kind.label()));
    }

    fn inspector_panel(&mut self, ui: &mut egui::Ui) {
        let snapshot = self
            .selected_source
            .and_then(|id| self.host.document().source(id).cloned());

        let Some(source) = snapshot else {
            egui::Panel::right("streamcraft-inspector")
                .default_size(250.0)
                .show(ui, |ui| {
                    widgets::section_label(ui, "Inspector");
                    widgets::empty_state(
                        ui,
                        "No source selected",
                        "Select a source in the list or on the canvas.",
                    );
                });
            return;
        };

        let id = source.id;
        let mut end_gesture = false;
        egui::Panel::right("streamcraft-inspector")
            .default_size(250.0)
            .resizable(true)
            .show(ui, |ui| {
                widgets::section_label(ui, "Inspector");
                ui.label(RichText::new(&source.name).strong());
                ui.label(
                    RichText::new(source.kind.label())
                        .size(11.0)
                        .color(theme::MUTED),
                );
                ui.separator();

                let mut name = source.name.clone();
                let response = ui.text_edit_singleline(&mut name);
                if response.changed() {
                    self.edit_name(id, name.clone());
                }
                end_gesture |= response.lost_focus();

                if source.kind == SourceKind::Text {
                    let mut text = source.text.clone();
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                    if response.changed() {
                        self.edit_text(id, text.clone());
                    }
                    end_gesture |= response.lost_focus();
                }

                ui.separator();
                let (x, y, w, h) = source.rect();
                let mut x = x;
                let mut y = y;
                let mut w = w;
                let mut h = h;
                ui.horizontal(|ui| {
                    let response = ui.add(egui::DragValue::new(&mut x).speed(1.0).prefix("X "));
                    if response.changed() {
                        self.edit_transform(id, x, y, w, h);
                    }
                    end_gesture |= response.drag_stopped() || response.lost_focus();
                    let response = ui.add(egui::DragValue::new(&mut y).speed(1.0).prefix("Y "));
                    if response.changed() {
                        self.edit_transform(id, x, y, w, h);
                    }
                    end_gesture |= response.drag_stopped() || response.lost_focus();
                });
                ui.horizontal(|ui| {
                    let response = ui.add(egui::DragValue::new(&mut w).speed(1.0).prefix("W "));
                    if response.changed() {
                        self.edit_transform(id, x, y, w, h);
                    }
                    end_gesture |= response.drag_stopped() || response.lost_focus();
                    let response = ui.add(egui::DragValue::new(&mut h).speed(1.0).prefix("H "));
                    if response.changed() {
                        self.edit_transform(id, x, y, w, h);
                    }
                    end_gesture |= response.drag_stopped() || response.lost_focus();
                });

                let mut opacity = source.opacity;
                let response = ui.add(
                    egui::Slider::new(&mut opacity, 0.0..=1.0)
                        .text("Opacity")
                        .fixed_decimals(2),
                );
                if response.changed() {
                    self.edit_opacity(id, opacity);
                }
                end_gesture |= response.drag_stopped() || response.lost_focus();

                let mut color = to_color32(source.color, 1.0);
                if ui.color_edit_button_srgba(&mut color).changed() {
                    self.finish_property_gesture();
                    let rgb = Color {
                        r: color.r(),
                        g: color.g(),
                        b: color.b(),
                        a: source.color.a,
                    };
                    self.host.apply(|document| {
                        if let Some(source) = document
                            .selected_scene_mut()
                            .and_then(|scene| scene.source_mut(id))
                        {
                            source.color = rgb;
                        }
                    });
                }

                let mut visible = source.visible;
                if ui.checkbox(&mut visible, "Visible").changed() {
                    self.finish_property_gesture();
                    self.host.apply(|document| {
                        if let Some(source) = document
                            .selected_scene_mut()
                            .and_then(|scene| scene.source_mut(id))
                        {
                            source.visible = visible;
                        }
                    });
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.small_button("Raise").clicked() {
                        self.finish_property_gesture();
                        self.host.apply(|document| {
                            document.move_source_z(id, 1);
                        });
                    }
                    if ui.small_button("Lower").clicked() {
                        self.finish_property_gesture();
                        self.host.apply(|document| {
                            document.move_source_z(id, -1);
                        });
                    }
                });
                if ui.button("Delete source").clicked() {
                    self.finish_property_gesture();
                    self.host.apply(|document| {
                        document.remove_source(id);
                    });
                    self.selected_source = None;
                    self.shell.set_status("Source deleted");
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Drag on the canvas to move. Dragging records one undo step.")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
        if end_gesture {
            self.finish_property_gesture();
        }
    }

    fn central_panel(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let mut container = self.record_container;
        let mut fps = self.record_fps;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Composition");
                ui.label(
                    RichText::new("Local compositor - capture/recording experimental")
                        .size(11.0)
                        .color(theme::WARNING),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::toolbar_button(ui, "Export PNG", "Export the scene as PNG")
                        .clicked()
                    {
                        action = Some(Action::ExportPng);
                    }
                    if widgets::toolbar_button(ui, "Export SVG", "Export the scene as SVG")
                        .clicked()
                    {
                        action = Some(Action::ExportSvg);
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Recording container")
                        .size(11.0)
                        .color(theme::MUTED),
                );
                ui.add_enabled_ui(self.recording.is_none(), |ui| {
                    egui::ComboBox::from_id_salt("record-container")
                        .selected_text(container.extension())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut container, Container::Mkv, "mkv");
                            ui.selectable_value(&mut container, Container::Mp4, "mp4");
                        });
                });
                ui.label("Frame rate");
                ui.add_enabled_ui(self.recording.is_none(), |ui| {
                    egui::ComboBox::from_id_salt("record-fps")
                        .selected_text(format!("{fps} fps"))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut fps, 15, "15 fps");
                            ui.selectable_value(&mut fps, 24, "24 fps");
                            ui.selectable_value(&mut fps, 30, "30 fps");
                            ui.selectable_value(&mut fps, 60, "60 fps");
                        });
                });
                if self.recording.is_some() {
                    if widgets::toolbar_button(ui, "Stop", "Stop recording").clicked() {
                        action = Some(Action::StopRecording);
                    }
                } else if widgets::toolbar_button(ui, "Record", "Start local recording").clicked() {
                    action = Some(Action::StartRecording);
                }
            });
            ui.add_space(6.0);
            self.paint_canvas(ui);
        });
        if container != self.record_container && self.recording.is_none() {
            self.record_container = container;
        }
        if fps != self.record_fps && self.recording.is_none() {
            self.record_fps = fps;
        }
        if let Some(action) = action {
            self.handle_action(action);
        }
    }

    fn paint_canvas(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size();
        let scale = (available.x / CANVAS_W)
            .min(available.y / CANVAS_H)
            .clamp(0.05, 2.0);
        let size = Vec2::new(CANVAS_W * scale, CANVAS_H * scale);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let painter = ui.painter().clone();
        painter.rect_filled(rect, 0.0, theme::SUNKEN);
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, theme::BORDER),
            StrokeKind::Outside,
        );

        let to_rect = |x: f32, y: f32, w: f32, h: f32| {
            Rect::from_min_size(
                rect.min + Vec2::new(x * scale, y * scale),
                Vec2::new(w * scale, h * scale),
            )
        };

        let scene = self.host.document().selected_scene().cloned();
        let selected_source = self.selected_source;
        let paint_scene = |scene: &Scene, scene_opacity: f32, show_selection: bool| {
            if scene_opacity <= 0.0 {
                return;
            }
            for source in &scene.sources {
                let opacity = source.opacity * scene_opacity;
                if !source.visible || opacity <= 0.0 {
                    continue;
                }
                let source_rect = to_rect(source.x, source.y, source.w, source.h);
                let color = to_color32(source.color, opacity);
                match source.kind {
                    SourceKind::Color => {
                        painter.rect_filled(source_rect, 0.0, color);
                    }
                    SourceKind::TestPattern => {
                        let top_h = source_rect.height() * 2.0 / 3.0;
                        let bar_w = source_rect.width() / SMPTE_BARS.len() as f32;
                        for (index, bar) in SMPTE_BARS.iter().enumerate() {
                            let bar_rect = Rect::from_min_size(
                                source_rect.min + Vec2::new(index as f32 * bar_w, 0.0),
                                Vec2::new(bar_w + 0.5, top_h),
                            );
                            painter.rect_filled(bar_rect, 0.0, to_color32(*bar, opacity));
                        }
                        let bottom_h = source_rect.height() - top_h;
                        let bottom_w = source_rect.width() / SMPTE_LOWER.len() as f32;
                        for (index, bar) in SMPTE_LOWER.iter().enumerate() {
                            let bar_rect = Rect::from_min_size(
                                source_rect.min + Vec2::new(index as f32 * bottom_w, top_h),
                                Vec2::new(bottom_w + 0.5, bottom_h),
                            );
                            painter.rect_filled(bar_rect, 0.0, to_color32(*bar, opacity));
                        }
                    }
                    SourceKind::Text => {
                        let font_size = (source_rect.height() * 0.7).max(8.0);
                        painter.text(
                            source_rect.min + Vec2::new(4.0, 2.0),
                            Align2::LEFT_TOP,
                            &source.text,
                            FontId::proportional(font_size),
                            color,
                        );
                    }
                }
                if show_selection && selected_source == Some(source.id) {
                    painter.rect_stroke(
                        source_rect,
                        0.0,
                        Stroke::new(2.0, theme::ACCENT),
                        StrokeKind::Inside,
                    );
                }
            }
        };

        let mut transition_complete = false;
        if let Some(start) = self.transition_start {
            let elapsed = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
            let progress = self.transition.progress(elapsed);
            if let Some(from) = self.transition_from.as_ref() {
                paint_scene(from, 1.0 - progress, false);
            }
            if let Some(scene) = scene.as_ref() {
                paint_scene(scene, progress, true);
            }
            transition_complete = progress >= 1.0;
        } else if let Some(scene) = scene.as_ref() {
            paint_scene(scene, 1.0, true);
        }
        if transition_complete {
            self.clear_transition();
        }

        if let Some(pointer) = response.interact_pointer_pos() {
            let scene_point = (pointer - rect.min) / scale;
            if response.drag_started() {
                let hit = scene.as_ref().and_then(|scene| {
                    scene.sources.iter().rev().find(|source| {
                        source.visible
                            && source.opacity > 0.0
                            && source.x <= scene_point.x
                            && scene_point.x <= source.x + source.w
                            && source.y <= scene_point.y
                            && scene_point.y <= source.y + source.h
                    })
                });
                if let Some(hit) = hit {
                    self.selected_source = Some(hit.id);
                    self.drag = Some(DragState {
                        source_id: hit.id,
                        grab_x: scene_point.x - hit.x,
                        grab_y: scene_point.y - hit.y,
                    });
                    self.host.history_mut().begin_gesture();
                }
            }
            if response.dragged()
                && let Some(drag) = self.drag
            {
                let geometry = self
                    .host
                    .document()
                    .source(drag.source_id)
                    .map(|source| (source.w, source.h));
                if let Some((w, h)) = geometry {
                    self.host.history_mut().current_mut().set_transform(
                        drag.source_id,
                        scene_point.x - drag.grab_x,
                        scene_point.y - drag.grab_y,
                        w,
                        h,
                    );
                }
            }
        }
        if (response.drag_stopped() || response.clicked()) && self.drag.take().is_some() {
            self.host.history_mut().end_gesture();
        }
    }

    fn capture_panel(&mut self, ctx: &egui::Context) {
        if !self.capture_enabled {
            return;
        }
        let mut open = true;
        let mut action = None;
        let mut refreshed = false;
        let mut selected = self.capture_target.clone();
        let error = self.capture_preview.error.clone();
        let texture = self.capture_preview.texture.clone();
        let targets = self.capture_targets.clone();
        egui::Window::new("Capture preview")
            .open(&mut open)
            .default_size([340.0, 260.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Live X11 preview only - recording saves the composed scene")
                        .size(10.5)
                        .color(theme::MUTED),
                );
                egui::ComboBox::from_label("Target")
                    .selected_text(selected.to_string())
                    .show_ui(ui, |ui| {
                        for target in &targets {
                            let label = target.to_string();
                            ui.selectable_value(&mut selected, target.clone(), label);
                        }
                    });
                if let Some(error) = &error {
                    ui.label(RichText::new(error).color(theme::ERROR));
                }
                if let Some(texture) = &texture {
                    let width = ui.available_width().clamp(1.0, 320.0);
                    let [texture_width, texture_height] = texture.size();
                    let aspect = if texture_width > 0 && texture_height > 0 {
                        texture_height as f32 / texture_width as f32
                    } else {
                        720.0 / 1280.0
                    };
                    ui.image((texture.id(), Vec2::new(width, (width * aspect).min(240.0))));
                } else if error.is_none() {
                    ui.label(RichText::new("Waiting for first frame...").color(theme::FAINT));
                }
                ui.horizontal(|ui| {
                    if ui.button("Refresh targets").clicked() {
                        self.refresh_capture_targets();
                        self.capture_preview.last_attempt = Instant::now() - CAPTURE_INTERVAL;
                        refreshed = true;
                    }
                    if ui.button("Stop capture").clicked() {
                        action = Some(Action::ToggleCapture);
                    }
                });
            });
        if refreshed {
            selected = self.capture_target.clone();
        }
        if selected != self.capture_target {
            self.capture_target = selected;
            self.capture_preview.texture = None;
            self.capture_preview.error = None;
            self.capture_preview.last_attempt = Instant::now();
        }
        if !open {
            self.capture_enabled = false;
            self.capture_preview.texture = None;
            self.capture_preview.error = None;
            self.shell.set_status("Capture preview stopped");
        }
        if let Some(action) = action {
            self.handle_action(action);
        }
    }

    fn unsaved_window(&mut self, ctx: &egui::Context) {
        if self.pending.is_none() {
            return;
        }
        let mut open = true;
        let mut choice = None;
        egui::Window::new("Unsaved changes")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.label("The current project has unsaved changes.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Save and continue").clicked() {
                        choice = Some(PendingChoice::Save);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(PendingChoice::Discard);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(PendingChoice::Cancel);
                    }
                });
            });
        if let Some(choice) = choice {
            self.resolve_pending(choice);
        } else if !open {
            self.pending = None;
        }
    }
}

fn next_repaint_delay(
    capture_enabled: bool,
    record_pacer: Option<FramePacer>,
    audio_running: bool,
    transitioning: bool,
) -> Option<Duration> {
    let capture_delay = capture_enabled.then_some(CAPTURE_INTERVAL);
    let recording_delay = record_pacer.map(FramePacer::interval);
    let audio_delay = audio_running.then_some(Duration::from_millis(50));
    let transition_delay = transitioning.then_some(Duration::from_millis(16));
    [
        capture_delay,
        recording_delay,
        audio_delay,
        transition_delay,
    ]
    .into_iter()
    .flatten()
    .min()
}

fn to_color32(color: Color, opacity: f32) -> Color32 {
    let alpha = (f32::from(color.a) * opacity.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgba_unmultiplied(color.r, color.g, color.b, alpha)
}

fn slugify(title: &str) -> String {
    let mut slug = String::new();
    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "untitled".to_owned()
    } else {
        slug.to_owned()
    }
}

impl eframe::App for StreamCraftApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.fullscreen {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
            self.fullscreen = false;
        }
        self.handle_shortcuts(ui);
        self.poll_capture(&ctx);
        self.poll_recording();
        let audio_running = matches!(self.audio.status(), AudioStatus::Running(_));
        if let Some(delay) = next_repaint_delay(
            self.capture_enabled,
            self.record_pacer,
            audio_running,
            self.transition_start.is_some(),
        ) {
            ctx.request_repaint_after(delay);
        }
        self.top_bar(ui);
        self.status_bar(ui);
        self.scenes_panel(ui);
        self.sources_panel(ui);
        self.inspector_panel(ui);
        self.central_panel(ui);
        self.capture_panel(&ctx);
        self.unsaved_window(&ctx);
        self.shell.show_windows(&ctx);
        self.shell
            .update_title(&ctx, &self.host.display_name(), self.host.is_dirty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repaint_delay_covers_preview_meter_recording_and_transitions() {
        assert_eq!(next_repaint_delay(false, None, false, false), None);
        assert_eq!(
            next_repaint_delay(true, None, false, false),
            Some(CAPTURE_INTERVAL)
        );
        assert_eq!(
            next_repaint_delay(false, None, true, false),
            Some(Duration::from_millis(50))
        );
        assert_eq!(
            next_repaint_delay(false, None, false, true),
            Some(Duration::from_millis(16))
        );
        let pacer = FramePacer::new(60).expect("pacer");
        assert_eq!(
            next_repaint_delay(true, Some(pacer), true, true),
            Some(Duration::from_millis(16))
        );
    }

    #[test]
    fn slugify_produces_safe_file_names() {
        assert_eq!(slugify("My Scene / 01"), "my-scene-01");
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("---"), "untitled");
    }

    #[test]
    fn colour_conversion_respects_opacity() {
        let color = to_color32(Color::rgb(10, 20, 30), 0.5);
        assert_eq!(color.r(), 5);
        assert_eq!(color.g(), 10);
        assert_eq!(color.b(), 15);
        assert_eq!(color.a(), 128);
    }
}
