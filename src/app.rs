//! StreamCraft UI, menus, actions and project lifecycle.
//!
//! Screen rendering uses egui's `Painter`; export reuses the same document
//! model through the dependency-free renderer in [`crate::render`].

use std::path::PathBuf;

use craftkit_io::DocumentHost;
use craftkit_ui::cli::CliOptions;
use craftkit_ui::eframe::{self, egui};
use craftkit_ui::egui::{
    Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};
use craftkit_ui::{AppIdentity, AppShell, Tone, dialogs, theme, widgets};

use crate::document::{
    CANVAS_H, CANVAS_W, Color, Document, FORMAT, SCHEMA_VERSION, SMPTE_BARS, SMPTE_LOWER,
    SourceKind,
};
use crate::render::render_scene_png;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

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

/// The application state.
pub struct StreamCraftApp {
    shell: AppShell,
    host: DocumentHost<Document>,
    selected_source: Option<u64>,
    drag: Option<DragState>,
    property_gesture: bool,
    pending: Option<PendingAction>,
}

impl StreamCraftApp {
    /// Build the application, opening the startup project when requested.
    pub fn new(context: &eframe::CreationContext<'_>, options: CliOptions) -> Self {
        let identity = AppIdentity {
            id: "streamcraft",
            name: "StreamCraft",
            version: APP_VERSION,
            tagline: "Compositor only - capture/recording next",
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
            selected_source: None,
            drag: None,
            property_gesture: false,
            pending: None,
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
                self.shell.set_status(format!("Opened {}", path.display()));
            }
            Err(error) => {
                self.shell
                    .notify(Tone::Error, "Could not open project", error.to_string());
            }
        }
    }

    fn new_document(&mut self) {
        self.host.reset(Document::default());
        self.selected_source = None;
        self.shell.set_status("New document");
    }

    fn finish_property_gesture(&mut self) {
        if self.property_gesture {
            self.host.history_mut().end_gesture();
            self.property_gesture = false;
        }
    }

    fn finish_gesture(&mut self) {
        if self.drag.take().is_some() {
            self.host.history_mut().end_gesture();
        }
        self.finish_property_gesture();
    }

    fn begin_property_gesture(&mut self) {
        if !self.property_gesture {
            self.host.history_mut().begin_gesture();
            self.property_gesture = true;
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
        let title = if png {
            "Export scene as PNG"
        } else {
            "Export scene as SVG"
        };
        let extension = if png { "png" } else { "svg" };
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
                    .notify(Tone::Error, "Could not export scene", error.to_string());
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
        self.shell.status_bar(ui);
    }

    fn scenes_panel(&mut self, ui: &mut egui::Ui) {
        let scenes: Vec<(usize, String)> = self
            .host
            .document()
            .scenes
            .iter()
            .enumerate()
            .map(|(index, scene)| (index, scene.name.clone()))
            .collect();
        let selected_scene = self.host.document().selected_scene;

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
                        for (index, name) in &scenes {
                            if ui
                                .selectable_label(*index == selected_scene, name)
                                .clicked()
                            {
                                self.host.apply(|document| {
                                    document.select_scene(*index);
                                });
                                self.selected_source = None;
                            }
                        }
                    });
                ui.separator();
                if ui.button("Add scene").clicked() {
                    self.host.apply(|document| {
                        document.add_scene();
                    });
                    self.selected_source = None;
                    self.shell.set_status("Scene added");
                }
                if ui.button("Remove selected scene").clicked() {
                    let index = self.host.document().selected_scene;
                    self.host.apply(|document| {
                        document.remove_scene(index);
                    });
                    self.clamp_selection();
                    self.shell.set_status("Scene removed");
                }
            });
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
                                    self.host.apply(|document| {
                                        document.move_source_z(id, 1);
                                    });
                                }
                                if ui.small_button("v").on_hover_text("Lower").clicked() {
                                    self.host.apply(|document| {
                                        document.move_source_z(id, -1);
                                    });
                                }
                                let mut visible = *visible;
                                if ui.checkbox(&mut visible, "").changed() {
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
            });
    }

    fn add_source(&mut self, kind: SourceKind) {
        let id = {
            let mut created = None;
            self.host.apply(|document| {
                created = document.add_source(kind);
            });
            created
        };
        self.selected_source = id;
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
                        self.host.apply(|document| {
                            document.move_source_z(id, 1);
                        });
                    }
                    if ui.small_button("Lower").clicked() {
                        self.host.apply(|document| {
                            document.move_source_z(id, -1);
                        });
                    }
                });
                if ui.button("Delete source").clicked() {
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
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Composition");
                ui.label(
                    RichText::new("Compositor only - capture/recording next")
                        .size(11.0)
                        .color(theme::WARNING),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::toolbar_button(ui, "Export SVG", "Export the scene as SVG")
                        .clicked()
                    {
                        action = Some(Action::ExportSvg);
                    }
                    if widgets::toolbar_button(ui, "Export PNG", "Export the scene as PNG")
                        .clicked()
                    {
                        action = Some(Action::ExportPng);
                    }
                });
            });
            ui.add_space(6.0);
            self.paint_canvas(ui);
        });
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
        if let Some(scene) = &scene {
            for source in &scene.sources {
                if !source.visible || source.opacity <= 0.0 {
                    continue;
                }
                let source_rect = to_rect(source.x, source.y, source.w, source.h);
                let color = to_color32(source.color, source.opacity);
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
                            painter.rect_filled(bar_rect, 0.0, to_color32(*bar, source.opacity));
                        }
                        let bottom_h = source_rect.height() - top_h;
                        let bottom_w = source_rect.width() / SMPTE_LOWER.len() as f32;
                        for (index, bar) in SMPTE_LOWER.iter().enumerate() {
                            let bar_rect = Rect::from_min_size(
                                source_rect.min + Vec2::new(index as f32 * bottom_w, top_h),
                                Vec2::new(bottom_w + 0.5, bottom_h),
                            );
                            painter.rect_filled(bar_rect, 0.0, to_color32(*bar, source.opacity));
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
                if self.selected_source == Some(source.id) {
                    painter.rect_stroke(
                        source_rect,
                        0.0,
                        Stroke::new(2.0, theme::ACCENT),
                        StrokeKind::Inside,
                    );
                }
            }
        }

        if let Some(pointer) = response.interact_pointer_pos() {
            let scene_point = (pointer - rect.min) / scale;
            if response.drag_started() {
                let hit = scene.as_ref().and_then(|scene| {
                    scene.sources.iter().rev().find(|source| {
                        source.visible
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
        self.handle_shortcuts(ui);
        self.top_bar(ui);
        self.status_bar(ui);
        self.scenes_panel(ui);
        self.sources_panel(ui);
        self.inspector_panel(ui);
        self.central_panel(ui);
        self.unsaved_window(ui.ctx());
        self.shell.show_windows(ui.ctx());
        self.shell
            .update_title(ui.ctx(), &self.host.display_name(), self.host.is_dirty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_produces_safe_file_names() {
        assert_eq!(slugify("My Scene / 01"), "my-scene-01");
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("---"), "untitled");
    }

    #[test]
    fn colour_conversion_respects_opacity() {
        // egui's Color32 stores premultiplied alpha: 50% of 10/20/30 is 5/10/15.
        let color = to_color32(Color::rgb(10, 20, 30), 0.5);
        assert_eq!(color.r(), 5);
        assert_eq!(color.g(), 10);
        assert_eq!(color.b(), 15);
        assert_eq!(color.a(), 128);
    }
}
