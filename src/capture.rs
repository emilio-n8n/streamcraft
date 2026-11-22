//! Capture abstraction with a Linux/X11 implementation.
//!
//! The rest of the application only sees [`CaptureBackend`], [`CaptureTarget`]
//! and [`CaptureFrame`]. On other operating systems, or when no X display is
//! available, the fallback returns a descriptive error instead of pretending
//! that capture works. No capture is attempted in unit tests unless a real
//! display is present and the test is run with `--ignored`.

use std::fmt;

/// A captured frame in packed RGBA format, top-down rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFrame {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Packed RGBA pixels, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl CaptureFrame {
    /// Create a frame from raw RGBA data, rejecting a mismatched buffer size.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, CaptureError> {
        let expected = width as usize * height as usize * 4;
        if width == 0 || height == 0 || rgba.len() != expected {
            return Err(CaptureError::InvalidFrame {
                width,
                height,
                bytes: rgba.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// Convert one pixel to `(r, g, b, a)`.
    #[must_use]
    #[cfg(test)]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        Some([
            self.rgba[offset],
            self.rgba[offset + 1],
            self.rgba[offset + 2],
            self.rgba[offset + 3],
        ])
    }

    /// Convert to an egui colour image for preview.
    #[must_use]
    pub fn to_color_image(&self) -> craftkit_ui::egui::ColorImage {
        use craftkit_ui::egui::{Color32, ColorImage, Vec2};
        let pixels = self
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| Color32::from_rgba_unmultiplied(chunk[0], chunk[1], chunk[2], chunk[3]))
            .collect();
        ColorImage {
            size: [self.width as usize, self.height as usize],
            pixels,
            source_size: Vec2::new(self.width as f32, self.height as f32),
        }
    }
}

/// A selectable capture target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    /// The whole virtual screen (root window).
    Screen,
    /// A top-level X11 window.
    Window {
        /// X11 window identifier.
        id: u32,
        /// Human-readable window title.
        title: String,
    },
}

impl fmt::Display for CaptureTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Screen => formatter.write_str("Entire screen"),
            Self::Window { title, .. } => formatter.write_str(title),
        }
    }
}

/// Why a capture could not be produced.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    /// The platform has no capture implementation in this build.
    #[error("capture is not available on this platform: {0}")]
    #[cfg(not(target_os = "linux"))]
    Unsupported(&'static str),
    /// No graphical display is reachable.
    #[error("no display available: {0}")]
    NoDisplay(String),
    /// Communicating with the display server failed.
    #[error("display server error: {0}")]
    Display(String),
    /// The requested target is gone or cannot be read.
    #[error("capture target unavailable: {0}")]
    Target(String),
    /// The backend produced an ill-formed frame.
    #[error("invalid frame {width}x{height} with {bytes} bytes")]
    InvalidFrame {
        /// Declared width.
        width: u32,
        /// Declared height.
        height: u32,
        /// Number of bytes supplied.
        bytes: usize,
    },
}

/// A platform capture backend.
pub trait CaptureBackend: Send + Sync {
    /// Short identifier shown in the UI.
    fn name(&self) -> &'static str;
    /// Whether the backend can currently reach a display server.
    fn available(&self) -> bool;
    /// Enumerate selectable targets.
    fn targets(&self) -> Result<Vec<CaptureTarget>, CaptureError>;
    /// Capture one frame from a target.
    fn capture(&self, target: &CaptureTarget) -> Result<CaptureFrame, CaptureError>;
}

/// Pick the capture backend for this build.
///
/// Linux builds use X11; every other platform gets an explicit fallback that
/// reports itself unavailable. No function here panics when a display is
/// missing.
#[must_use]
pub fn default_backend() -> Box<dyn CaptureBackend> {
    #[cfg(target_os = "linux")]
    {
        Box::new(x11::X11Backend::new())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(UnsupportedBackend)
    }
}

#[cfg(not(target_os = "linux"))]
struct UnsupportedBackend;

#[cfg(not(target_os = "linux"))]
impl CaptureBackend for UnsupportedBackend {
    fn name(&self) -> &'static str {
        "unsupported"
    }

    fn available(&self) -> bool {
        false
    }

    fn targets(&self) -> Result<Vec<CaptureTarget>, CaptureError> {
        Err(CaptureError::Unsupported(
            "this build only implements Linux/X11 capture",
        ))
    }

    fn capture(&self, _target: &CaptureTarget) -> Result<CaptureFrame, CaptureError> {
        Err(CaptureError::Unsupported(
            "this build only implements Linux/X11 capture",
        ))
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use super::{CaptureBackend, CaptureError, CaptureFrame, CaptureTarget};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, Window};
    use x11rb::rust_connection::RustConnection;

    fn window_title(connection: &RustConnection, window: Window) -> Option<String> {
        let atom = connection
            .intern_atom(false, b"WM_NAME")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let property = connection
            .get_property(false, window, atom, 0_u32, 0, 128)
            .ok()?
            .reply()
            .ok()?;
        if property.value.is_empty() {
            return None;
        }
        Some(String::from_utf8_lossy(&property.value).into_owned())
    }

    /// Linux/X11 capture backend using core `GetImage`.
    pub(super) struct X11Backend;

    impl X11Backend {
        pub(super) fn new() -> Self {
            Self
        }

        fn connect(&self) -> Result<(RustConnection, usize), CaptureError> {
            let (connection, screen_number) =
                x11rb::connect(None).map_err(|error| CaptureError::NoDisplay(error.to_string()))?;
            if screen_number >= connection.setup().roots.len() {
                return Err(CaptureError::Display(format!(
                    "screen {screen_number} is not present"
                )));
            }
            Ok((connection, screen_number))
        }
    }

    impl CaptureBackend for X11Backend {
        fn name(&self) -> &'static str {
            "X11"
        }

        fn available(&self) -> bool {
            self.connect().is_ok()
        }

        fn targets(&self) -> Result<Vec<CaptureTarget>, CaptureError> {
            let (connection, screen_number) = self.connect()?;
            let root = connection.setup().roots[screen_number].root;
            let mut targets = vec![CaptureTarget::Screen];
            let children = connection
                .query_tree(root)
                .map_err(|error| CaptureError::Display(error.to_string()))?
                .reply()
                .map_err(|error| CaptureError::Display(error.to_string()))?;
            for window in children.children {
                if let Some(title) = window_title(&connection, window)
                    && !title.is_empty()
                {
                    targets.push(CaptureTarget::Window { id: window, title });
                }
            }
            Ok(targets)
        }

        fn capture(&self, target: &CaptureTarget) -> Result<CaptureFrame, CaptureError> {
            let (connection, screen_number) = self.connect()?;
            let screen = &connection.setup().roots[screen_number];
            let drawable: Window = match target {
                CaptureTarget::Screen => screen.root,
                CaptureTarget::Window { id, .. } => *id,
            };
            let geometry = connection
                .get_geometry(drawable)
                .map_err(|error| CaptureError::Display(error.to_string()))?
                .reply()
                .map_err(|error| CaptureError::Display(error.to_string()))?;
            if geometry.width == 0 || geometry.height == 0 {
                return Err(CaptureError::Target("target has zero size".to_owned()));
            }

            let reply = connection
                .get_image(
                    ImageFormat::Z_PIXMAP,
                    drawable,
                    0,
                    0,
                    geometry.width,
                    geometry.height,
                    u32::MAX,
                )
                .map_err(|error| CaptureError::Display(error.to_string()))?
                .reply()
                .map_err(|error| CaptureError::Display(error.to_string()))?;

            let width = geometry.width;
            let height = geometry.height;
            let bpp = reply.depth;
            if bpp != 24 && bpp != 32 {
                return Err(CaptureError::Target(format!(
                    "unsupported screen depth {bpp} (expected 24 or 32)"
                )));
            }
            let stride = reply
                .data
                .len()
                .checked_div(height as usize)
                .ok_or_else(|| CaptureError::Target("empty image data".to_owned()))?;
            let pixel_stride = stride.checked_div(width as usize).unwrap_or(4);
            if pixel_stride < 3 {
                return Err(CaptureError::Target(format!(
                    "unsupported pixel stride {pixel_stride}"
                )));
            }

            let visual = screen
                .allowed_depths
                .iter()
                .flat_map(|depth| &depth.visuals)
                .find(|visual| visual.visual_id == screen.root_visual)
                .or_else(|| {
                    screen
                        .allowed_depths
                        .first()
                        .and_then(|depth| depth.visuals.first())
                });
            let (red_mask, blue_mask) = visual
                .map(|visual| (visual.red_mask, visual.blue_mask))
                .unwrap_or((0x00ff_0000, 0x0000_00ff));
            let bgr = red_mask == 0x00ff_0000 && blue_mask == 0x0000_00ff;

            let mut rgba = vec![0_u8; width as usize * height as usize * 4];
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let source = y * stride + x * pixel_stride;
                    if source + 2 >= reply.data.len() {
                        return Err(CaptureError::Target("truncated image data".to_owned()));
                    }
                    let (red, blue) = if bgr {
                        (reply.data[source + 2], reply.data[source])
                    } else {
                        (reply.data[source], reply.data[source + 2])
                    };
                    let destination = (y * width as usize + x) * 4;
                    rgba[destination] = red;
                    rgba[destination + 1] = reply.data[source + 1];
                    rgba[destination + 2] = blue;
                    rgba[destination + 3] = 255;
                }
            }
            CaptureFrame::new(u32::from(width), u32::from(height), rgba)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_validation_rejects_bad_sizes() {
        assert!(CaptureFrame::new(2, 2, vec![0; 16]).is_ok());
        assert!(matches!(
            CaptureFrame::new(2, 2, vec![0; 15]),
            Err(CaptureError::InvalidFrame { .. })
        ));
        assert!(matches!(
            CaptureFrame::new(0, 10, Vec::new()),
            Err(CaptureError::InvalidFrame { .. })
        ));
    }

    #[test]
    fn frame_pixel_and_image_conversion() {
        let frame = CaptureFrame::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).expect("frame");
        assert_eq!(frame.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(frame.pixel(1, 0), Some([0, 0, 255, 128]));
        assert_eq!(frame.pixel(2, 0), None);
        let image = frame.to_color_image();
        assert_eq!(image.size, [2, 1]);
        assert_eq!(image.pixels.len(), 2);
    }

    #[test]
    fn backend_never_panics_without_a_display() {
        let backend = default_backend();
        assert!(!backend.name().is_empty());
        // These calls must return errors gracefully on a headless machine.
        let _ = backend.available();
        let _ = backend.targets();
        let _ = backend.capture(&CaptureTarget::Screen);
    }

    #[test]
    #[ignore = "requires a real X display; run with `xvfb-run -a cargo test -- --ignored`"]
    fn x11_captures_the_root_window() {
        let backend = default_backend();
        assert_eq!(backend.name(), "X11");
        let frame = backend
            .capture(&CaptureTarget::Screen)
            .expect("capture root window");
        assert!(frame.width > 0 && frame.height > 0);
        assert_eq!(
            frame.rgba.len(),
            frame.width as usize * frame.height as usize * 4
        );
        // Every pixel must be opaque; a BGRX->RGBA conversion that silently
        // dropped the alpha channel would fail here.
        assert!(
            frame
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 255)
        );
    }
}
