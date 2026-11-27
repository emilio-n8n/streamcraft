//! Capture abstraction with a Linux/X11 implementation.
//!
//! The rest of the application only sees [`CaptureBackend`], [`CaptureTarget`]
//! and [`CaptureFrame`]. On other operating systems, or when no X display is
//! available, the fallback returns a descriptive error instead of pretending
//! that capture works. No capture is attempted in unit tests unless a real
//! display is present and the test is run with `--ignored`.

use std::fmt;

/// Maximum RGBA capture buffer accepted from a display server (64 MiB).
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// Maximum width or height accepted for a capture target.
pub const MAX_CAPTURE_DIMENSION: u32 = 16_384;

fn frame_byte_count(width: u32, height: u32) -> Option<usize> {
    let pixels = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    pixels
        .checked_mul(4)
        .filter(|bytes| *bytes <= MAX_FRAME_BYTES)
}

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
        let expected = frame_byte_count(width, height);
        if width == 0
            || height == 0
            || width > MAX_CAPTURE_DIMENSION
            || height > MAX_CAPTURE_DIMENSION
            || expected != Some(rgba.len())
        {
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
    /// An X11 window.
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
    use super::{
        CaptureBackend, CaptureError, CaptureFrame, CaptureTarget, MAX_CAPTURE_DIMENSION,
        MAX_FRAME_BYTES, frame_byte_count,
    };
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        ConnectionExt, ImageFormat, ImageOrder, MapState, VisualClass, Window,
    };
    use x11rb::rust_connection::RustConnection;

    const MAX_TARGET_WINDOWS: usize = 256;
    const MAX_INSPECTED_WINDOWS: usize = 1024;
    const MAX_WINDOW_DEPTH: usize = 8;

    fn property_title(connection: &RustConnection, window: Window, name: &[u8]) -> Option<String> {
        let atom = connection.intern_atom(false, name).ok()?.reply().ok()?.atom;
        let property = connection
            .get_property(false, window, atom, 0_u32, 0, 128)
            .ok()?
            .reply()
            .ok()?;
        let bytes = property.value.strip_suffix(&[0]).unwrap_or(&property.value);
        if bytes.is_empty() {
            return None;
        }
        Some(String::from_utf8_lossy(bytes).into_owned())
    }

    fn window_title(connection: &RustConnection, window: Window) -> Option<String> {
        property_title(connection, window, b"_NET_WM_NAME")
            .or_else(|| property_title(connection, window, b"WM_NAME"))
    }

    fn collect_window_targets(
        connection: &RustConnection,
        parent: Window,
        depth: usize,
        targets: &mut Vec<CaptureTarget>,
        inspected: &mut usize,
    ) {
        if depth >= MAX_WINDOW_DEPTH
            || targets.len() >= MAX_TARGET_WINDOWS
            || *inspected >= MAX_INSPECTED_WINDOWS
        {
            return;
        }
        let Ok(cookie) = connection.query_tree(parent) else {
            return;
        };
        let Ok(children) = cookie.reply() else {
            return;
        };
        for window in children.children {
            if targets.len() >= MAX_TARGET_WINDOWS || *inspected >= MAX_INSPECTED_WINDOWS {
                break;
            }
            *inspected += 1;
            let Ok(cookie) = connection.get_window_attributes(window) else {
                continue;
            };
            let Ok(attributes) = cookie.reply() else {
                continue;
            };
            if attributes.map_state != MapState::VIEWABLE {
                continue;
            }
            if let Some(title) = window_title(connection, window)
                && !title.trim().is_empty()
            {
                targets.push(CaptureTarget::Window { id: window, title });
            }
            // Dialogs and utility windows are often nested below the main
            // top-level window. A bounded walk includes them without allowing
            // a malformed tree to cause unbounded work.
            collect_window_targets(connection, window, depth + 1, targets, inspected);
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct PixelFormat {
        bytes_per_pixel: u8,
        little_endian: bool,
        masks: [u32; 3],
    }

    fn decode_pixels(
        data: &[u8],
        width: u32,
        height: u32,
        format: PixelFormat,
        scanline_pad: u8,
    ) -> Result<Vec<u8>, CaptureError> {
        let byte_count = frame_byte_count(width, height)
            .ok_or_else(|| CaptureError::Target("capture dimensions are too large".to_owned()))?;
        let bits_per_pixel = u32::from(format.bytes_per_pixel) * 8;
        let row_bits = width
            .checked_mul(bits_per_pixel)
            .ok_or_else(|| CaptureError::Target("capture row is too large".to_owned()))?;
        let pad = u32::from(scanline_pad);
        let padded_bits = row_bits
            .checked_add(pad.saturating_sub(1))
            .and_then(|bits| bits.checked_div(pad))
            .and_then(|rows| rows.checked_mul(pad))
            .ok_or_else(|| CaptureError::Target("invalid scanline padding".to_owned()))?;
        let stride = usize::try_from(padded_bits / 8)
            .map_err(|_| CaptureError::Target("capture stride is too large".to_owned()))?;
        let expected = stride
            .checked_mul(usize::try_from(height).unwrap_or(usize::MAX))
            .filter(|bytes| {
                *bytes <= MAX_FRAME_BYTES + usize::try_from(MAX_CAPTURE_DIMENSION).unwrap_or(0) * 4
            })
            .ok_or_else(|| CaptureError::Target("capture data is too large".to_owned()))?;
        if data.len() < expected {
            return Err(CaptureError::Target(format!(
                "truncated image data: expected {expected} bytes, got {}",
                data.len()
            )));
        }

        let channels = [
            ColorChannel::new(format.masks[0])?,
            ColorChannel::new(format.masks[1])?,
            ColorChannel::new(format.masks[2])?,
        ];
        let mut rgba = vec![0_u8; byte_count];
        let pixel_bytes = usize::from(format.bytes_per_pixel);
        for y in 0..usize::try_from(height).unwrap_or(usize::MAX) {
            for x in 0..usize::try_from(width).unwrap_or(usize::MAX) {
                let source = y * stride + x * pixel_bytes;
                let mut raw = 0_u32;
                for (offset, &byte) in data[source..source + pixel_bytes].iter().enumerate() {
                    if format.little_endian {
                        raw |= u32::from(byte) << (offset * 8);
                    } else {
                        raw = (raw << 8) | u32::from(byte);
                    }
                }
                let destination = (y * usize::try_from(width).unwrap_or(usize::MAX) + x) * 4;
                for (channel, layout) in channels.into_iter().enumerate() {
                    rgba[destination + channel] = layout.decode(raw);
                }
                rgba[destination + 3] = 255;
            }
        }
        Ok(rgba)
    }

    #[derive(Debug, Clone, Copy)]
    struct ColorChannel {
        mask: u32,
        shift: u32,
        maximum: u32,
    }

    impl ColorChannel {
        fn new(mask: u32) -> Result<Self, CaptureError> {
            if mask == 0 {
                return Ok(Self {
                    mask,
                    shift: 0,
                    maximum: 0,
                });
            }
            let shift = mask.trailing_zeros();
            let maximum = mask >> shift;
            if maximum
                .checked_add(1)
                .is_some_and(|next| next & (next - 1) != 0)
            {
                return Err(CaptureError::Target(format!(
                    "unsupported non-contiguous colour mask {mask:#x}"
                )));
            }
            Ok(Self {
                mask,
                shift,
                maximum,
            })
        }

        fn decode(self, raw: u32) -> u8 {
            if self.mask == 0 {
                return 0;
            }
            let value = u64::from((raw & self.mask) >> self.shift);
            let maximum = u64::from(self.maximum);
            ((value * 255 + maximum / 2) / maximum) as u8
        }
    }

    #[cfg(test)]
    fn decode_channel(raw: u32, mask: u32) -> Result<u8, CaptureError> {
        ColorChannel::new(mask).map(|channel| channel.decode(raw))
    }

    /// Linux/X11 capture backend using core `GetImage`.
    pub(super) struct X11Backend;

    impl X11Backend {
        pub(super) fn new() -> Self {
            Self
        }

        fn connect(&self) -> Result<(RustConnection, usize), CaptureError> {
            // A fresh connection on every attempt reconnects automatically if
            // the X server restarts; no stale socket is cached in the backend.
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
            let mut inspected = 0;
            collect_window_targets(&connection, root, 0, &mut targets, &mut inspected);
            Ok(targets)
        }

        fn capture(&self, target: &CaptureTarget) -> Result<CaptureFrame, CaptureError> {
            let (connection, screen_number) = self.connect()?;
            let screen = &connection.setup().roots[screen_number];
            let drawable: Window = match target {
                CaptureTarget::Screen => screen.root,
                CaptureTarget::Window { id, .. } => *id,
            };
            if let CaptureTarget::Window { id, title } = target
                && window_title(&connection, *id).as_deref() != Some(title.as_str())
            {
                return Err(CaptureError::Target(
                    "window is gone or its title changed; refresh targets".to_owned(),
                ));
            }
            let geometry = connection
                .get_geometry(drawable)
                .map_err(|error| CaptureError::Display(error.to_string()))?
                .reply()
                .map_err(|error| CaptureError::Display(error.to_string()))?;
            if geometry.width == 0 || geometry.height == 0 {
                return Err(CaptureError::Target("target has zero size".to_owned()));
            }
            let width = u32::from(geometry.width);
            let height = u32::from(geometry.height);
            if width > MAX_CAPTURE_DIMENSION
                || height > MAX_CAPTURE_DIMENSION
                || frame_byte_count(width, height).is_none()
            {
                return Err(CaptureError::Target(format!(
                    "target {width}x{height} exceeds the 64 MiB capture limit"
                )));
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

            let visual = screen
                .allowed_depths
                .iter()
                .flat_map(|depth| &depth.visuals)
                .find(|visual| visual.visual_id == reply.visual)
                .ok_or_else(|| {
                    CaptureError::Target(format!("unknown X11 visual {}", reply.visual))
                })?;
            if visual.class != VisualClass::TRUE_COLOR && visual.class != VisualClass::DIRECT_COLOR
            {
                return Err(CaptureError::Target(format!(
                    "unsupported X11 visual class {:?}",
                    visual.class
                )));
            }
            let format = connection
                .setup()
                .pixmap_formats
                .iter()
                .filter(|format| format.depth == reply.depth)
                .max_by_key(|format| format.bits_per_pixel)
                .filter(|format| matches!(format.bits_per_pixel, 16 | 24 | 32))
                .ok_or_else(|| {
                    CaptureError::Target(format!(
                        "unsupported X11 depth {} (expected 16, 24 or 32 bits)",
                        reply.depth
                    ))
                })?;
            let little_endian = match connection.setup().image_byte_order {
                ImageOrder::LSB_FIRST => true,
                ImageOrder::MSB_FIRST => false,
                order => {
                    return Err(CaptureError::Target(format!(
                        "unsupported X11 image byte order {order:?}"
                    )));
                }
            };
            let rgba = decode_pixels(
                &reply.data,
                width,
                height,
                PixelFormat {
                    bytes_per_pixel: format.bits_per_pixel / 8,
                    little_endian,
                    masks: [visual.red_mask, visual.green_mask, visual.blue_mask],
                },
                format.scanline_pad,
            )?;
            CaptureFrame::new(width, height, rgba)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn format(bytes_per_pixel: u8, little_endian: bool, masks: [u32; 3]) -> PixelFormat {
            PixelFormat {
                bytes_per_pixel,
                little_endian,
                masks,
            }
        }

        #[test]
        fn decodes_bgrx_and_rgba_without_using_depth_as_stride() {
            let bgrx = [0, 0, 255, 0, 255, 0, 0, 0];
            assert_eq!(
                decode_pixels(
                    &bgrx,
                    2,
                    1,
                    format(4, true, [0x00ff_0000, 0x0000_ff00, 0x0000_00ff]),
                    32,
                )
                .expect("BGRX"),
                [255, 0, 0, 255, 0, 0, 255, 255]
            );
            assert_eq!(
                decode_pixels(
                    &[0, 0, 255, 255],
                    1,
                    1,
                    format(4, true, [0x0000_00ff, 0x0000_ff00, 0x00ff_0000]),
                    32,
                )
                .expect("RGBA"),
                [0, 0, 255, 255]
            );
        }

        #[test]
        fn decodes_rgb565_and_big_endian_pixels() {
            assert_eq!(
                decode_pixels(
                    &[0, 0xf8, 0x1f, 0],
                    2,
                    1,
                    format(2, true, [0xf800, 0x07e0, 0x001f]),
                    16,
                )
                .expect("RGB565"),
                [255, 0, 0, 255, 0, 0, 255, 255]
            );
            assert_eq!(
                decode_pixels(
                    &[0, 255, 0, 0],
                    1,
                    1,
                    format(4, false, [0x00ff_0000, 0x0000_ff00, 0x0000_00ff]),
                    32,
                )
                .expect("big endian"),
                [255, 0, 0, 255]
            );
        }

        #[test]
        fn rejects_truncated_data_and_invalid_masks() {
            assert!(matches!(
                decode_pixels(
                    &[0, 0, 0],
                    1,
                    1,
                    format(4, true, [0x00ff_0000, 0x0000_ff00, 0x0000_00ff]),
                    32,
                ),
                Err(CaptureError::Target(_))
            ));
            assert!(matches!(
                decode_channel(0, 0b0101),
                Err(CaptureError::Target(_))
            ));
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
        assert!(matches!(
            CaptureFrame::new(u32::MAX, u32::MAX, Vec::new()),
            Err(CaptureError::InvalidFrame { .. })
        ));
        assert!(matches!(
            CaptureFrame::new(MAX_CAPTURE_DIMENSION + 1, 1, vec![0; 4]),
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
    fn x11_lists_and_captures_a_window_after_reconnecting() {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{ConnectionExt, CreateWindowAux, PropMode, WindowClass};
        use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

        let (connection, screen_number) = x11rb::connect(None).expect("connect");
        let screen = &connection.setup().roots[screen_number];
        let window = connection.generate_id().expect("window id");
        connection
            .create_window(
                0,
                window,
                screen.root,
                10,
                10,
                80,
                50,
                0,
                WindowClass::INPUT_OUTPUT,
                screen.root_visual,
                &CreateWindowAux::new(),
            )
            .expect("create window");
        let utf8 = connection
            .intern_atom(false, b"UTF8_STRING")
            .expect("intern UTF8")
            .reply()
            .expect("UTF8 atom")
            .atom;
        connection
            .change_property8(
                PropMode::REPLACE,
                window,
                connection
                    .intern_atom(false, b"_NET_WM_NAME")
                    .expect("intern title")
                    .reply()
                    .expect("title atom")
                    .atom,
                utf8,
                b"StreamCraft capture test",
            )
            .expect("set title");
        connection.map_window(window).expect("map window");
        connection.sync().expect("sync");

        let backend = default_backend();
        let targets = backend.targets().expect("targets");
        assert!(targets.iter().any(|target| matches!(
            target,
            CaptureTarget::Window { title, .. } if title == "StreamCraft capture test"
        )));
        for _ in 0..2 {
            let frame = backend
                .capture(&CaptureTarget::Window {
                    id: window,
                    title: "StreamCraft capture test".to_owned(),
                })
                .expect("capture window");
            assert_eq!((frame.width, frame.height), (80, 50));
        }
        connection.destroy_window(window).expect("destroy window");
        connection.sync().expect("sync destruction");
        assert!(matches!(
            backend.capture(&CaptureTarget::Window {
                id: window,
                title: "StreamCraft capture test".to_owned(),
            }),
            Err(CaptureError::Target(_))
        ));
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
