//! Reference application UI and lifecycle.

use std::path::PathBuf;
use std::time::Duration;

use craftkit_io::DocumentHost;
use craftkit_jobs::JobRunner;
use craftkit_ui::cli::CliOptions;
use craftkit_ui::eframe::{self, egui};
use craftkit_ui::egui::{Align, Layout, RichText};
use craftkit_ui::{AppIdentity, AppShell, Tone, dialogs, theme, widgets};

use crate::document::Document;

const FORMAT: &str = "streamcraft";
const SCHEMA_VERSION: u32 = 1;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    New,
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
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

/// The reference application state.
pub struct DemoApp {
    shell: AppShell,
    host: DocumentHost<Document>,
    selected: usize,
    new_note: String,
    jobs: JobRunner<String>,
    pending: Option<PendingAction>,
    gesture_active: bool,
}

impl DemoApp {
    /// Build the application, opening the startup project when requested.
    pub fn new(context: &eframe::CreationContext<'_>, options: CliOptions) -> Self {
        let identity = AppIdentity {
            id: "streamcraft",
            name: "StreamCraft",
            version: APP_VERSION,
            tagline: "Scenes, sources and a compositor for local recording",
            repository: Some("https://github.com/emilio-n8n/streamcraft"),
        };
        let shell = AppShell::new(identity);
        let document = if options.sample {
            Document::sample()
        } else {
            Document::default()
        };
        let mut app = Self {
            shell,
            host: DocumentHost::new(document, FORMAT, SCHEMA_VERSION, APP_VERSION),
            selected: 0,
            new_note: String::new(),
            jobs: JobRunner::new(),
            pending: None,
            gesture_active: false,
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
                self.selected = 0;
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
        self.selected = 0;
        self.shell.set_status("New document");
    }

    fn finish_gesture(&mut self) {
        if self.gesture_active {
            self.host.history_mut().end_gesture();
            self.gesture_active = false;
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
        let count = self.host.document().notes.len();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    fn start_job(&mut self) {
        let result = self.jobs.spawn("Demo job", |context| {
            for step in 0..=20 {
                context
                    .check_cancelled()
                    .map_err(|error| error.to_string())?;
                context.set_progress_message(step as f32 / 20.0, format!("Step {step}/20"));
                std::thread::sleep(Duration::from_millis(60));
            }
            Ok("Processed 20 steps".to_owned())
        });
        if let Err(error) = result {
            self.shell
                .notify(Tone::Error, "Could not start job", error.to_string());
        }
    }

    fn poll_jobs(&mut self) {
        for (_, result) in self.jobs.poll() {
            match result {
                Ok(message) => self.shell.notify(Tone::Success, "Job finished", message),
                Err(error) => self.shell.notify(Tone::Error, "Job failed", error),
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
            Action::Undo => {
                self.finish_gesture();
                if self.host.undo() {
                    self.clamp_selection();
                    self.shell.set_status("Undo");
                }
            }
            Action::Redo => {
                self.finish_gesture();
                if self.host.redo() {
                    self.clamp_selection();
                    self.shell.set_status("Redo");
                }
            }
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
        let running = self.jobs.jobs();
        self.shell.status_bar_with(ui, |ui| {
            for job in running {
                widgets::job_row(ui, job);
            }
        });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        let DemoApp {
            host,
            selected,
            new_note,
            ..
        } = self;
        egui::Panel::left("notes-panel")
            .resizable(true)
            .default_size(230.0)
            .min_size(170.0)
            .show(ui, |ui| {
                widgets::section_label(ui, "Notes");
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (index, note) in host.document().notes.iter().enumerate() {
                            if ui
                                .selectable_label(index == *selected, note.as_str())
                                .clicked()
                            {
                                *selected = index;
                            }
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    let response =
                        ui.add(egui::TextEdit::singleline(new_note).hint_text("New note"));
                    let enter = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if ui.button("Add").clicked() || enter {
                        let text = new_note.trim().to_owned();
                        if !text.is_empty() {
                            host.apply(|document| document.notes.push(text));
                            *selected = host.document().notes.len().saturating_sub(1);
                            new_note.clear();
                        }
                    }
                });
            });
    }

    fn central_panel(&mut self, ui: &mut egui::Ui) {
        let selected = self.selected;
        let mut gained_focus = false;
        let mut lost_focus = false;
        let mut run_job = false;
        let mut open_settings = false;

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Notes");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::toolbar_button(ui, "Run job", "Start a background job").clicked() {
                        run_job = true;
                    }
                    if widgets::toolbar_button(ui, "Settings", "Preferences").clicked() {
                        open_settings = true;
                    }
                });
            });
            ui.add_space(6.0);

            let document = self.host.history_mut().current_mut();
            let response =
                ui.add(egui::TextEdit::singleline(&mut document.title).hint_text("Document title"));
            gained_focus |= response.gained_focus();
            lost_focus |= response.lost_focus();

            ui.separator();
            if let Some(note) = document.notes.get_mut(selected) {
                ui.label(
                    RichText::new("Selected note")
                        .size(11.0)
                        .color(theme::MUTED),
                );
                let response = ui.add(
                    egui::TextEdit::multiline(note)
                        .desired_rows(8)
                        .desired_width(f32::INFINITY),
                );
                gained_focus |= response.gained_focus();
                lost_focus |= response.lost_focus();
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Click outside the field to record one undo step.")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            } else {
                widgets::empty_state(
                    ui,
                    "No note selected",
                    "Add a note on the left and select it to edit.",
                );
            }
        });

        if gained_focus {
            self.gesture_active = true;
        }
        if lost_focus {
            self.finish_gesture();
        }
        if run_job {
            self.start_job();
        }
        if open_settings {
            self.shell.open_settings();
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

impl eframe::App for DemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_jobs();
        self.handle_shortcuts(ui);
        if self.gesture_active {
            self.host.history_mut().begin_gesture();
        }
        self.top_bar(ui);
        self.status_bar(ui);
        self.left_panel(ui);
        self.central_panel(ui);
        self.unsaved_window(ui.ctx());
        self.shell.show_windows(ui.ctx());
        self.shell
            .update_title(ui.ctx(), &self.host.display_name(), self.host.is_dirty());
    }
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
