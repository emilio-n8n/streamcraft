//! Dependency-free raster export for scenes.
//!
//! The on-screen canvas uses egui's `Painter`; this module draws the same
//! document model into a plain RGBA buffer and encodes it as a PNG. It keeps
//! the export path testable without a GPU or a GUI, and adds no third-party
//! image dependency to the project.

use crate::document::{
    CANVAS_H, CANVAS_W, Color, SMPTE_BARS, SMPTE_LOWER, Scene, Source, SourceKind,
};

/// A simple RGBA framebuffer.
struct Framebuffer {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Framebuffer {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width * height * 4],
        }
    }

    fn blend(&mut self, x: i32, y: i32, color: Color, opacity: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let alpha = (f32::from(color.a) / 255.0 * opacity).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let index = (y as usize * self.width + x as usize) * 4;
        let channels = [color.r, color.g, color.b];
        for (offset, value) in channels.iter().enumerate() {
            let source = f32::from(*value) * alpha;
            let destination = f32::from(self.pixels[index + offset]) * (1.0 - alpha);
            self.pixels[index + offset] = (source + destination).round() as u8;
        }
        self.pixels[index + 3] = 255;
    }

    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color, opacity: f32) {
        if w <= 0.0 || h <= 0.0 || opacity <= 0.0 {
            return;
        }
        let x0 = x.round().max(0.0) as i32;
        let y0 = y.round().max(0.0) as i32;
        let x1 = (x + w).round().min(self.width as f32) as i32;
        let y1 = (y + h).round().min(self.height as f32) as i32;
        for py in y0..y1 {
            for px in x0..x1 {
                self.blend(px, py, color, opacity);
            }
        }
    }

    fn draw_source(&mut self, source: &Source) {
        if !source.visible || source.opacity <= 0.0 {
            return;
        }
        let (x, y, w, h) = source.rect();
        match source.kind {
            SourceKind::Color => self.fill_rect(x, y, w, h, source.color, source.opacity),
            SourceKind::TestPattern => {
                let top_h = h * 2.0 / 3.0;
                let bottom_h = h - top_h;
                let bar_w = w / SMPTE_BARS.len() as f32;
                for (index, color) in SMPTE_BARS.iter().enumerate() {
                    self.fill_rect(
                        x + index as f32 * bar_w,
                        y,
                        bar_w,
                        top_h,
                        *color,
                        source.opacity,
                    );
                }
                let bottom_w = w / SMPTE_LOWER.len() as f32;
                for (index, color) in SMPTE_LOWER.iter().enumerate() {
                    self.fill_rect(
                        x + index as f32 * bottom_w,
                        y + top_h,
                        bottom_w,
                        bottom_h,
                        *color,
                        source.opacity,
                    );
                }
            }
            SourceKind::Text => {
                let scale = ((h / 10.0).floor() as usize).clamp(1, 16);
                let mut cursor_x = x.round() as i32;
                let mut cursor_y = y.round() as i32;
                for character in source.text.chars() {
                    if character == '\n' {
                        cursor_x = x.round() as i32;
                        cursor_y += i32::try_from(8 * scale).unwrap_or(128);
                        continue;
                    }
                    if character == ' ' {
                        cursor_x += i32::try_from(4 * scale).unwrap_or(64);
                        continue;
                    }
                    if let Some(glyph) = glyph(character) {
                        for (row, bits) in glyph.iter().enumerate() {
                            for column in 0..3 {
                                if bits & (1 << column) != 0 {
                                    let px = cursor_x + (column * scale) as i32;
                                    let py = cursor_y + (row * scale) as i32;
                                    self.fill_rect(
                                        px as f32,
                                        py as f32,
                                        scale as f32,
                                        scale as f32,
                                        source.color,
                                        source.opacity,
                                    );
                                }
                            }
                        }
                    }
                    cursor_x += i32::try_from(4 * scale).unwrap_or(64);
                }
            }
        }
    }
}

/// Look up a 3x5 bitmap glyph. Bit 0 is the leftmost column.
fn glyph(character: char) -> Option<&'static [u8; 5]> {
    const FONT: &[(char, [u8; 5])] = &[
        ('A', [2, 5, 7, 5, 5]),
        ('B', [3, 5, 3, 5, 3]),
        ('C', [6, 1, 1, 1, 6]),
        ('D', [3, 5, 5, 5, 3]),
        ('E', [7, 1, 3, 1, 7]),
        ('F', [7, 1, 3, 1, 1]),
        ('G', [6, 1, 5, 5, 6]),
        ('H', [5, 5, 7, 5, 5]),
        ('I', [7, 2, 2, 2, 7]),
        ('J', [4, 4, 4, 5, 6]),
        ('K', [5, 5, 3, 5, 5]),
        ('L', [1, 1, 1, 1, 7]),
        ('M', [5, 7, 7, 5, 5]),
        ('N', [5, 7, 7, 7, 5]),
        ('O', [2, 5, 5, 5, 2]),
        ('P', [3, 5, 3, 1, 1]),
        ('Q', [2, 5, 5, 7, 3]),
        ('R', [3, 5, 3, 5, 5]),
        ('S', [6, 1, 2, 4, 3]),
        ('T', [7, 2, 2, 2, 2]),
        ('U', [5, 5, 5, 5, 7]),
        ('V', [5, 5, 5, 5, 2]),
        ('W', [5, 5, 7, 7, 5]),
        ('X', [5, 5, 2, 5, 5]),
        ('Y', [5, 5, 2, 2, 2]),
        ('Z', [7, 4, 2, 1, 7]),
        ('0', [2, 5, 5, 5, 2]),
        ('1', [2, 3, 2, 2, 7]),
        ('2', [3, 4, 2, 1, 7]),
        ('3', [3, 4, 2, 4, 3]),
        ('4', [5, 5, 7, 4, 4]),
        ('5', [7, 1, 3, 4, 3]),
        ('6', [6, 1, 3, 5, 2]),
        ('7', [7, 4, 2, 1, 1]),
        ('8', [2, 5, 2, 5, 2]),
        ('9', [2, 5, 6, 4, 3]),
        ('.', [0, 0, 0, 0, 2]),
        (',', [0, 0, 0, 2, 1]),
        (':', [0, 2, 0, 2, 0]),
        ('!', [2, 2, 2, 0, 2]),
        ('?', [3, 4, 2, 0, 2]),
        ('-', [0, 0, 7, 0, 0]),
        ('_', [0, 0, 0, 0, 7]),
        ('/', [4, 4, 2, 1, 1]),
        ('(', [4, 2, 2, 2, 4]),
        (')', [1, 2, 2, 2, 1]),
        ('+', [0, 2, 7, 2, 0]),
        ('=', [0, 7, 0, 7, 0]),
        ('#', [5, 7, 5, 7, 5]),
        ('%', [5, 4, 2, 1, 5]),
        ('\'', [2, 2, 0, 0, 0]),
    ];
    FONT.iter()
        .find(|(key, _)| *key == character)
        .map(|(_, bits)| bits)
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = index as u32;
        let mut step = 0;
        while step < 8 {
            value = if value & 1 != 0 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            step += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in data {
        crc = CRC_TABLE[((crc ^ u32::from(byte)) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut a = 1_u32;
    let mut b = 0_u32;
    for &byte in data {
        a = (a + u32::from(byte)) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

fn push_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    output.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// Encode a packed RGBA buffer as a PNG image with uncompressed DEFLATE blocks.
///
/// The result is a valid, portable PNG (8-bit RGBA, no interlace); stored
/// blocks keep the encoder small and deterministic.
#[must_use]
pub fn encode_png(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let row_bytes = width as usize * 4;
    let mut raw = Vec::with_capacity((row_bytes + 1) * height as usize);
    for row in 0..height as usize {
        raw.push(0); // filter type: None
        let start = row * row_bytes;
        raw.extend_from_slice(&pixels[start..start + row_bytes]);
    }

    // zlib wrapper (RFC 1950) around a stream of stored DEFLATE blocks.
    let mut compressed = vec![0x78, 0x01];
    let mut offset = 0;
    while offset < raw.len() {
        let length = (raw.len() - offset).min(u16::MAX as usize);
        let final_block = u8::from(offset + length == raw.len());
        let length = length as u16;
        compressed.push(final_block);
        compressed.extend_from_slice(&length.to_le_bytes());
        compressed.extend_from_slice(&(!length).to_le_bytes());
        compressed.extend_from_slice(&raw[offset..offset + usize::from(length)]);
        offset += usize::from(length);
    }
    compressed.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut output = Vec::with_capacity(compressed.len() + 57);
    output.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    push_chunk(&mut output, b"IHDR", &ihdr);
    push_chunk(&mut output, b"IDAT", &compressed);
    push_chunk(&mut output, b"IEND", &[]);
    output
}

/// Render a scene to a PNG image of the logical canvas size.
#[must_use]
pub fn render_scene_png(scene: &Scene) -> Vec<u8> {
    let width = CANVAS_W as usize;
    let height = CANVAS_H as usize;
    let mut framebuffer = Framebuffer::new(width, height);
    framebuffer.fill_rect(0.0, 0.0, CANVAS_W, CANVAS_H, Color::rgb(0, 0, 0), 1.0);
    for source in &scene.sources {
        framebuffer.draw_source(source);
    }
    encode_png(&framebuffer.pixels, CANVAS_W as u32, CANVAS_H as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, SourceKind};

    const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

    fn chunk_lengths(png: &[u8]) -> Vec<(&[u8; 4], usize)> {
        let mut chunks = Vec::new();
        let mut offset = 8;
        while offset + 12 <= png.len() {
            let length = u32::from_be_bytes(
                png[offset..offset + 4]
                    .try_into()
                    .expect("four length bytes"),
            ) as usize;
            let kind: &[u8; 4] = png[offset + 4..offset + 8]
                .try_into()
                .expect("four kind bytes");
            chunks.push((kind, length));
            offset += 12 + length;
        }
        chunks
    }

    #[test]
    fn sample_scene_exports_a_valid_non_empty_png() {
        let scene = Document::sample();
        let scene = scene.selected_scene().expect("scene");
        let png = render_scene_png(scene);
        assert!(png.len() > 1000);
        assert_eq!(&png[..8], &SIGNATURE);

        let chunks = chunk_lengths(&png);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], (b"IHDR", 13));
        assert!(chunks[1].0 == b"IDAT" && chunks[1].1 > 0);
        assert_eq!(chunks[2], (b"IEND", 0));

        let width = u32::from_be_bytes(png[16..20].try_into().expect("width"));
        let height = u32::from_be_bytes(png[20..24].try_into().expect("height"));
        assert_eq!(width, CANVAS_W as u32);
        assert_eq!(height, CANVAS_H as u32);
    }

    #[test]
    fn colour_and_opacity_are_rasterised() {
        let mut document = Document::default();
        let opaque = document.add_source(SourceKind::Color).expect("opaque");
        if let Some(source) = document
            .selected_scene_mut()
            .and_then(|scene| scene.source_mut(opaque))
        {
            source.x = 0.0;
            source.y = 0.0;
            source.w = 100.0;
            source.h = 100.0;
            source.color = Color::rgb(255, 0, 0);
        }
        let translucent = document.add_source(SourceKind::Color).expect("translucent");
        if let Some(source) = document
            .selected_scene_mut()
            .and_then(|scene| scene.source_mut(translucent))
        {
            source.x = 0.0;
            source.y = 0.0;
            source.w = 100.0;
            source.h = 100.0;
            source.color = Color::rgb(0, 0, 255);
            source.opacity = 0.5;
        }

        let scene = document.selected_scene().expect("scene");
        let mut framebuffer = Framebuffer::new(100, 100);
        framebuffer.fill_rect(0.0, 0.0, 100.0, 100.0, Color::rgb(0, 0, 0), 1.0);
        for source in &scene.sources {
            framebuffer.draw_source(source);
        }
        let pixel = &framebuffer.pixels[50 * 4..50 * 4 + 4];
        assert_eq!(pixel, &[128, 0, 128, 255]);
    }

    #[test]
    fn hidden_sources_are_not_drawn() {
        let mut document = Document::default();
        let id = document.add_source(SourceKind::Color).expect("source");
        if let Some(source) = document
            .selected_scene_mut()
            .and_then(|scene| scene.source_mut(id))
        {
            source.x = 0.0;
            source.y = 0.0;
            source.w = 10.0;
            source.h = 10.0;
            source.visible = false;
        }
        let scene = document.selected_scene().expect("scene");
        let png = render_scene_png(scene);
        // IDAT data starts after the signature, IHDR chunk and IDAT header.
        // Its first seven bytes are the zlib header and stored-block header.
        let idat_payload = 8 + (12 + 13) + 8;
        let first_scanline = idat_payload + 7;
        assert_eq!(png[first_scanline], 0);
        assert_eq!(
            &png[first_scanline + 1..first_scanline + 5],
            &[0, 0, 0, 255]
        );
    }
}
