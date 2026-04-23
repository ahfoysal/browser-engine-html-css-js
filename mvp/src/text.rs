//! Text shaping via rustybuzz (pure-Rust harfbuzz port). Produces per-glyph
//! advances and positions honoring kerning + OpenType features. We keep
//! fontdue for rasterization, feeding it glyph indices (u16) from the
//! shaper so the two agree on the same glyph table.
//!
//! Glyph ids from rustybuzz are u32 but in practice fit in u16 for the
//! TTFs we bundle. `rasterize_indexed` takes u16.

use rustybuzz::{Face, UnicodeBuffer};

/// One shaped glyph, positioned along a horizontal baseline.
#[derive(Debug, Clone, Copy)]
pub struct ShapedGlyph {
    pub glyph_id: u16,
    /// Pen x in pixels, relative to the start of the run.
    pub x: f32,
    /// Vertical offset in pixels (rarely non-zero for Latin).
    pub y_offset: f32,
    /// Advance width used for this glyph.
    pub advance: f32,
}

/// Shape a UTF-8 string with the given face at the given pixel size.
/// Returns the glyphs plus the total run width (sum of advances).
pub fn shape(face: &Face, text: &str, px: f32) -> (Vec<ShapedGlyph>, f32) {
    if text.is_empty() {
        return (Vec::new(), 0.0);
    }
    let upem = face.units_per_em() as f32;
    if upem <= 0.0 {
        return (Vec::new(), 0.0);
    }
    let scale = px / upem;

    let mut buf = UnicodeBuffer::new();
    buf.push_str(text);
    // Let rustybuzz infer direction/script/language from the content.
    let glyphs = rustybuzz::shape(face, &[], buf);

    let infos = glyphs.glyph_infos();
    let positions = glyphs.glyph_positions();
    let mut out = Vec::with_capacity(infos.len());
    let mut cursor = 0.0f32;
    for (info, pos) in infos.iter().zip(positions.iter()) {
        let x_off = pos.x_offset as f32 * scale;
        let y_off = pos.y_offset as f32 * scale;
        let adv = pos.x_advance as f32 * scale;
        out.push(ShapedGlyph {
            glyph_id: info.glyph_id as u16,
            x: cursor + x_off,
            y_offset: -y_off, // buzz y up, screen y down
            advance: adv,
        });
        cursor += adv;
    }
    (out, cursor)
}

/// Just measure the width of a shaped run — used by the line breaker.
pub fn measure(face: &Face, text: &str, px: f32) -> f32 {
    shape(face, text, px).1
}
