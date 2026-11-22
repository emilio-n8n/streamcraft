//! StreamCraft scene-compositor document model.
//!
//! The model is UI-free: it knows nothing about egui, the filesystem or the
//! window system. Geometry is expressed in logical canvas pixels, which makes
//! the same scene reusable for on-screen composition, SVG export and raster
//! export.

use serde::{Deserialize, Serialize};

/// Logical width of the composition canvas, in pixels.
pub const CANVAS_W: f32 = 1280.0;
/// Logical height of the composition canvas, in pixels.
pub const CANVAS_H: f32 = 720.0;
/// Smallest permitted width or height of a source, in pixels.
pub const MIN_SOURCE_SIZE: f32 = 16.0;
/// Format identifier written into project envelopes.
pub const FORMAT: &str = "streamcraft";
/// Document schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// The seven upper bars of a simplified SMPTE test pattern.
pub const SMPTE_BARS: [Color; 7] = [
    Color::rgb(191, 191, 191),
    Color::rgb(191, 191, 0),
    Color::rgb(0, 191, 191),
    Color::rgb(0, 191, 0),
    Color::rgb(191, 0, 191),
    Color::rgb(191, 0, 0),
    Color::rgb(0, 0, 191),
];

/// The seven lower bars of a simplified SMPTE test pattern.
pub const SMPTE_LOWER: [Color; 7] = [
    Color::rgb(0, 0, 191),
    Color::rgb(0, 0, 0),
    Color::rgb(191, 0, 191),
    Color::rgb(0, 0, 0),
    Color::rgb(0, 191, 191),
    Color::rgb(0, 0, 0),
    Color::rgb(191, 191, 191),
];

/// An RGBA colour, independent of any UI toolkit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    /// Red channel, 0..=255.
    pub r: u8,
    /// Green channel, 0..=255.
    pub g: u8,
    /// Blue channel, 0..=255.
    pub b: u8,
    /// Alpha channel, 0..=255.
    pub a: u8,
}

impl Color {
    /// Opaque colour from three channels.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// CSS-style hexadecimal representation, without the leading `#`.
    #[must_use]
    pub fn hex(self) -> String {
        format!("{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// What a source draws on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    /// A flat coloured rectangle.
    Color,
    /// A simplified SMPTE colour-bar test pattern.
    TestPattern,
    /// A block of text.
    Text,
}

impl SourceKind {
    /// Human-readable label used in the source list.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Color => "Colour",
            Self::TestPattern => "Test pattern",
            Self::Text => "Text",
        }
    }
}

/// One placed element of a scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// Stable identifier, unique inside the document.
    pub id: u64,
    /// Display name shown in the source list.
    pub name: String,
    /// Renderer used for this source.
    pub kind: SourceKind,
    /// Left edge in canvas pixels.
    pub x: f32,
    /// Top edge in canvas pixels.
    pub y: f32,
    /// Width in canvas pixels.
    pub w: f32,
    /// Height in canvas pixels.
    pub h: f32,
    /// Opacity in `0.0..=1.0`.
    pub opacity: f32,
    /// Whether the source is drawn and exported.
    #[serde(default = "default_visible")]
    pub visible: bool,
    /// Text content for [`SourceKind::Text`] sources.
    pub text: String,
    /// Base colour for colour and text sources.
    pub color: Color,
}

fn default_visible() -> bool {
    true
}

impl Source {
    /// Create a source with sensible defaults for its kind.
    #[must_use]
    pub fn new(id: u64, name: impl Into<String>, kind: SourceKind) -> Self {
        let (text, color) = match kind {
            SourceKind::Color => (String::new(), Color::rgb(48, 96, 168)),
            SourceKind::TestPattern => (String::new(), Color::rgb(255, 255, 255)),
            SourceKind::Text => ("New text".to_owned(), Color::rgb(240, 244, 255)),
        };
        Self {
            id,
            name: name.into(),
            kind,
            x: 40.0,
            y: 40.0,
            w: 320.0,
            h: 180.0,
            opacity: 1.0,
            visible: true,
            text,
            color,
        }
    }

    /// Constrain geometry and opacity to valid canvas values.
    ///
    /// Width and height are clamped to [`MIN_SOURCE_SIZE`] and the canvas
    /// dimensions; the origin is then clamped so the whole source stays inside
    /// the canvas. Opacity is clamped to `0.0..=1.0`.
    pub fn clamp_to_canvas(&mut self) {
        self.w = self.w.clamp(MIN_SOURCE_SIZE, CANVAS_W);
        self.h = self.h.clamp(MIN_SOURCE_SIZE, CANVAS_H);
        self.x = self.x.clamp(0.0, CANVAS_W - self.w);
        self.y = self.y.clamp(0.0, CANVAS_H - self.h);
        self.opacity = self.opacity.clamp(0.0, 1.0);
    }

    /// Rectangle `(x, y, w, h)` in canvas coordinates.
    #[must_use]
    pub const fn rect(&self) -> (f32, f32, f32, f32) {
        (self.x, self.y, self.w, self.h)
    }
}

/// An ordered collection of sources. Later entries are drawn on top.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// Stable identifier, unique inside the document.
    pub id: u64,
    /// Display name shown in the scene list.
    pub name: String,
    /// Sources in z order, from back to front.
    pub sources: Vec<Source>,
}

impl Scene {
    /// Create an empty scene.
    #[must_use]
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            sources: Vec::new(),
        }
    }

    /// Find a source by identifier.
    #[must_use]
    pub fn source(&self, id: u64) -> Option<&Source> {
        self.sources.iter().find(|source| source.id == id)
    }

    /// Find a source by identifier, mutably.
    pub fn source_mut(&mut self, id: u64) -> Option<&mut Source> {
        self.sources.iter_mut().find(|source| source.id == id)
    }

    /// Position of a source in the z stack, from the back.
    #[must_use]
    pub fn source_index(&self, id: u64) -> Option<usize> {
        self.sources.iter().position(|source| source.id == id)
    }

    /// Append a source at the top of the z stack.
    pub fn add_source(&mut self, source: Source) {
        self.sources.push(source);
    }

    /// Remove a source by identifier. Returns `true` when one was removed.
    pub fn remove_source(&mut self, id: u64) -> bool {
        let before = self.sources.len();
        self.sources.retain(|source| source.id != id);
        self.sources.len() != before
    }

    /// Move a source one step towards the front (`1`) or back (`-1`).
    ///
    /// Returns `true` when the source exists and actually changed position.
    pub fn move_z(&mut self, id: u64, direction: i32) -> bool {
        let Some(index) = self.source_index(id) else {
            return false;
        };
        let target = (index as i32 + direction).clamp(0, self.sources.len() as i32 - 1);
        if target == index as i32 {
            return false;
        }
        let source = self.sources.remove(index);
        self.sources.insert(target as usize, source);
        true
    }

    /// Serialize the scene to a self-contained SVG document.
    #[must_use]
    pub fn to_svg(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" \
             viewBox=\"0 0 {} {}\">\n",
            CANVAS_W as u32, CANVAS_H as u32, CANVAS_W as u32, CANVAS_H as u32
        ));
        out.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#000000\"/>\n");
        for source in &self.sources {
            if !source.visible || source.opacity <= 0.0 {
                continue;
            }
            let (x, y, w, h) = source.rect();
            out.push_str(&format!("<g opacity=\"{:.3}\">\n", source.opacity));
            match source.kind {
                SourceKind::Color => {
                    out.push_str(&rectangle(x, y, w, h, &source.color.hex()));
                }
                SourceKind::TestPattern => {
                    let top_h = h * 2.0 / 3.0;
                    let bottom_y = y + top_h;
                    let bottom_h = h - top_h;
                    for (index, color) in SMPTE_BARS.iter().enumerate() {
                        let bar_w = w / SMPTE_BARS.len() as f32;
                        out.push_str(&rectangle(
                            x + index as f32 * bar_w,
                            y,
                            bar_w + 0.5,
                            top_h,
                            &color.hex(),
                        ));
                    }
                    for (index, color) in SMPTE_LOWER.iter().enumerate() {
                        let bar_w = w / SMPTE_LOWER.len() as f32;
                        out.push_str(&rectangle(
                            x + index as f32 * bar_w,
                            bottom_y,
                            bar_w + 0.5,
                            bottom_h,
                            &color.hex(),
                        ));
                    }
                }
                SourceKind::Text => {
                    let size = (h * 0.7).max(8.0);
                    out.push_str(&format!(
                        "<text x=\"{:.1}\" y=\"{:.1}\" fill=\"#{}\" \
                         font-family=\"sans-serif\" font-size=\"{:.1}\">{}</text>\n",
                        x + 4.0,
                        y + h * 0.8,
                        source.color.hex(),
                        size,
                        escape_xml(&source.text)
                    ));
                }
            }
            out.push_str("</g>\n");
        }
        out.push_str("</svg>\n");
        out
    }
}

fn rectangle(x: f32, y: f32, w: f32, h: f32, color: &str) -> String {
    format!(
        "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"#{}\"/>\n",
        x, y, w, h, color
    )
}

fn escape_xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// A versioned scene-composition project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// Project title.
    pub title: String,
    /// Scenes in display order.
    pub scenes: Vec<Scene>,
    /// Index of the scene currently being edited.
    pub selected_scene: usize,
    /// Next identifier handed out by [`Document::alloc_id`].
    pub next_id: u64,
}

impl Default for Document {
    fn default() -> Self {
        let mut document = Self {
            title: "Untitled".to_owned(),
            scenes: Vec::new(),
            selected_scene: 0,
            next_id: 1,
        };
        let id = document.alloc_id();
        document.scenes.push(Scene::new(id, "Scene 1"));
        document
    }
}

impl Document {
    /// Built-in demonstration scene: coloured background, simplified SMPTE
    /// bars and a title.
    #[must_use]
    pub fn sample() -> Self {
        let mut document = Self {
            title: "StreamCraft sample".to_owned(),
            ..Self::default()
        };

        let background_id = document.alloc_id();
        let bars_id = document.alloc_id();
        let title_id = document.alloc_id();

        if let Some(scene) = document.scenes.first_mut() {
            scene.name = "Welcome".to_owned();

            let mut background = Source::new(background_id, "Background", SourceKind::Color);
            background.x = 0.0;
            background.y = 0.0;
            background.w = CANVAS_W;
            background.h = CANVAS_H;
            background.color = Color::rgb(18, 26, 46);
            scene.add_source(background);

            let mut bars = Source::new(bars_id, "SMPTE bars", SourceKind::TestPattern);
            bars.x = 240.0;
            bars.y = 170.0;
            bars.w = 800.0;
            bars.h = 380.0;
            scene.add_source(bars);

            let mut title = Source::new(title_id, "Title", SourceKind::Text);
            title.x = 40.0;
            title.y = 36.0;
            title.w = 900.0;
            title.h = 80.0;
            title.text = "StreamCraft - scene compositor".to_owned();
            title.color = Color::rgb(240, 244, 255);
            scene.add_source(title);
        }
        document
    }

    /// Reserve a document-wide unique identifier.
    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Append a new scene and select it. Returns the new scene identifier.
    pub fn add_scene(&mut self) -> u64 {
        let id = self.alloc_id();
        let name = format!("Scene {}", self.scenes.len() + 1);
        self.scenes.push(Scene::new(id, name));
        self.selected_scene = self.scenes.len() - 1;
        id
    }

    /// Remove a scene, keeping at least one editable scene in the document.
    ///
    /// Returns `true` when a scene was actually removed.
    pub fn remove_scene(&mut self, index: usize) -> bool {
        if index >= self.scenes.len() {
            return false;
        }
        if self.scenes.len() == 1 {
            let id = self.scenes[0].id;
            self.scenes[0] = Scene::new(id, "Scene 1".to_owned());
        } else {
            self.scenes.remove(index);
        }
        self.clamp_scene_selection();
        true
    }

    /// Select a scene by index.
    pub fn select_scene(&mut self, index: usize) -> bool {
        if index >= self.scenes.len() {
            return false;
        }
        self.selected_scene = index;
        true
    }

    /// Clamp the selected-scene index after a structural change.
    pub fn clamp_scene_selection(&mut self) {
        if self.scenes.is_empty() {
            self.selected_scene = 0;
        } else if self.selected_scene >= self.scenes.len() {
            self.selected_scene = self.scenes.len() - 1;
        }
    }

    /// Currently selected scene.
    #[must_use]
    pub fn selected_scene(&self) -> Option<&Scene> {
        self.scenes.get(self.selected_scene)
    }

    /// Currently selected scene, mutably.
    pub fn selected_scene_mut(&mut self) -> Option<&mut Scene> {
        let index = self.selected_scene;
        self.scenes.get_mut(index)
    }

    /// Add a source of the given kind to the selected scene.
    pub fn add_source(&mut self, kind: SourceKind) -> Option<u64> {
        let id = self.alloc_id();
        let name = format!("{} {}", kind.label(), self.next_id.saturating_sub(1));
        let source = Source::new(id, name, kind);
        self.selected_scene_mut()?.add_source(source);
        Some(id)
    }

    /// Remove a source from the selected scene.
    pub fn remove_source(&mut self, id: u64) -> bool {
        self.selected_scene_mut()
            .is_some_and(|scene| scene.remove_source(id))
    }

    /// Find a source in the selected scene.
    #[must_use]
    pub fn source(&self, id: u64) -> Option<&Source> {
        self.selected_scene()?.source(id)
    }

    /// Move a source within the selected scene's z stack.
    pub fn move_source_z(&mut self, id: u64, direction: i32) -> bool {
        self.selected_scene_mut()
            .is_some_and(|scene| scene.move_z(id, direction))
    }

    /// Set a source's opacity, clamped to `0.0..=1.0`.
    pub fn set_opacity(&mut self, id: u64, opacity: f32) -> bool {
        let Some(source) = self
            .selected_scene_mut()
            .and_then(|scene| scene.source_mut(id))
        else {
            return false;
        };
        let opacity = opacity.clamp(0.0, 1.0);
        if source.opacity == opacity {
            return false;
        }
        source.opacity = opacity;
        true
    }

    /// Set a source geometry, clamping it to the canvas.
    pub fn set_transform(&mut self, id: u64, x: f32, y: f32, w: f32, h: f32) -> bool {
        let Some(source) = self
            .selected_scene_mut()
            .and_then(|scene| scene.source_mut(id))
        else {
            return false;
        };
        let geometry = (source.x, source.y, source.w, source.h);
        source.x = x;
        source.y = y;
        source.w = w;
        source.h = h;
        source.clamp_to_canvas();
        geometry != source.rect()
    }

    /// Export the selected scene as an SVG document.
    #[must_use]
    pub fn export_svg(&self) -> Option<String> {
        self.selected_scene().map(Scene::to_svg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craftkit_core::{History, ProjectEnvelope};

    #[test]
    fn scene_and_source_lifecycle() {
        let mut document = Document::default();
        assert_eq!(document.scenes.len(), 1);
        assert_eq!(document.selected_scene, 0);

        let second = document.add_scene();
        assert_eq!(document.scenes.len(), 2);
        assert_eq!(document.selected_scene, 1);
        assert_eq!(document.scenes[1].id, second);

        let source = document.add_source(SourceKind::Color).expect("source");
        assert_eq!(document.selected_scene().expect("scene").sources.len(), 1);
        assert!(document.source(source).is_some());

        assert!(document.remove_source(source));
        assert!(!document.remove_source(source));
        assert!(document.selected_scene().expect("scene").sources.is_empty());

        assert!(document.remove_scene(1));
        assert_eq!(document.scenes.len(), 1);
        assert_eq!(document.selected_scene, 0);
        assert!(!document.remove_scene(5));
    }

    #[test]
    fn removing_the_last_scene_keeps_an_empty_scene() {
        let mut document = Document::default();
        assert!(document.remove_scene(0));
        assert_eq!(document.scenes.len(), 1);
        assert!(document.selected_scene().expect("scene").sources.is_empty());
    }

    #[test]
    fn z_order_and_opacity() {
        let mut document = Document::default();
        let back = document.add_source(SourceKind::Color).expect("back");
        let middle = document.add_source(SourceKind::Text).expect("middle");
        let front = document.add_source(SourceKind::TestPattern).expect("front");
        let ids: Vec<u64> = document
            .selected_scene()
            .expect("scene")
            .sources
            .iter()
            .map(|source| source.id)
            .collect();
        assert_eq!(ids, vec![back, middle, front]);

        assert!(document.move_source_z(back, 1));
        let ids: Vec<u64> = document
            .selected_scene()
            .expect("scene")
            .sources
            .iter()
            .map(|source| source.id)
            .collect();
        assert_eq!(ids, vec![middle, back, front]);

        assert!(!document.move_source_z(front, 1), "front is already on top");
        assert!(
            !document.move_source_z(middle, -1),
            "middle is already at the bottom"
        );
        assert!(!document.move_source_z(999, 1), "unknown source");

        assert!(document.set_opacity(front, 0.35));
        assert!((document.source(front).expect("front").opacity - 0.35).abs() < f32::EPSILON);
        assert!(document.set_opacity(front, 4.0));
        assert!((document.source(front).expect("front").opacity - 1.0).abs() < f32::EPSILON);
        assert!(document.set_opacity(front, -2.0));
        assert_eq!(document.source(front).expect("front").opacity, 0.0);
        assert!(!document.set_opacity(999, 0.5));
    }

    #[test]
    fn transform_clamping() {
        let mut source = Source::new(1, "Out of bounds", SourceKind::Color);
        source.x = -500.0;
        source.y = -500.0;
        source.w = 5000.0;
        source.h = 5000.0;
        source.opacity = 9.0;
        source.clamp_to_canvas();

        assert_eq!(source.w, CANVAS_W);
        assert_eq!(source.h, CANVAS_H);
        assert_eq!(source.x, 0.0);
        assert_eq!(source.y, 0.0);
        assert_eq!(source.opacity, 1.0);

        source.w = 1.0;
        source.h = 1.0;
        source.clamp_to_canvas();
        assert_eq!(source.w, MIN_SOURCE_SIZE);
        assert_eq!(source.h, MIN_SOURCE_SIZE);

        let mut document = Document::default();
        let id = document.add_source(SourceKind::Color).expect("source");
        assert!(document.set_transform(id, -100.0, -100.0, 2000.0, 2000.0));
        let rect = document.source(id).expect("source").rect();
        assert_eq!(rect, (0.0, 0.0, CANVAS_W, CANVAS_H));

        let before = document.source(id).expect("source").rect();
        assert!(!document.set_transform(id, before.0, before.1, before.2, before.3));
        assert!(!document.set_transform(999, 0.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn json_round_trip() {
        let document = Document::sample();
        let json = serde_json::to_string(&document).expect("serialize");
        let parsed: Document = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(document, parsed);

        let envelope = ProjectEnvelope::new(FORMAT, SCHEMA_VERSION, "0.1.0", &parsed);
        assert!(envelope.validate(FORMAT, SCHEMA_VERSION).is_ok());
        let json = serde_json::to_string(&envelope).expect("serialize envelope");
        let parsed: ProjectEnvelope<Document> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.document, document);
        assert_eq!(parsed.format, FORMAT);
        assert_eq!(parsed.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn undo_redo_of_a_move_is_one_step() {
        let mut history = History::new(Document::sample());
        let id = history
            .current()
            .selected_scene()
            .expect("scene")
            .sources
            .first()
            .expect("source")
            .id;
        let before = history.current().source(id).expect("source").rect();

        history.begin_gesture();
        let moved = {
            let document = history.current_mut();
            let source = document
                .selected_scene_mut()
                .expect("scene")
                .source_mut(id)
                .expect("source");
            source.x += 120.0;
            source.y += 35.0;
            source.rect()
        };
        history.end_gesture();

        assert_eq!(history.undo_depth(), 1);
        assert_ne!(moved, before);
        assert!(history.undo());
        assert_eq!(history.current().source(id).expect("source").rect(), before);
        assert!(history.redo());
        assert_eq!(history.current().source(id).expect("source").rect(), moved);
    }

    #[test]
    fn sample_document_has_background_bars_and_title() {
        let document = Document::sample();
        let scene = document.selected_scene().expect("scene");
        assert_eq!(scene.sources.len(), 3);
        assert_eq!(scene.sources[0].kind, SourceKind::Color);
        assert_eq!(scene.sources[1].kind, SourceKind::TestPattern);
        assert_eq!(scene.sources[2].kind, SourceKind::Text);
        assert!(scene.sources[2].text.contains("StreamCraft"));
    }

    #[test]
    fn svg_export_is_non_empty_and_escapes_text() {
        let mut document = Document::default();
        let id = document.add_source(SourceKind::Text).expect("source");
        if let Some(source) = document
            .selected_scene_mut()
            .and_then(|scene| scene.source_mut(id))
        {
            source.text = "A < B & C".to_owned();
        }
        let svg = document.export_svg().expect("svg");
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>\n"));
        assert!(svg.contains("A &lt; B &amp; C"));
        assert!(svg.contains("<text"));
        assert!(svg.contains("opacity="));
        assert!(svg.len() > 200);
    }
}
