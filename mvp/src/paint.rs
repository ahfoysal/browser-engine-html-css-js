//! Paint the layout tree onto a tiny-skia Pixmap and write PNG.

use tiny_skia::{Paint, Pixmap, Rect as SkRect, Transform};

use crate::css::Color;
use crate::layout::{LayoutBox, BoxType};

pub fn paint(root: &LayoutBox, width: u32, height: u32, font: &fontdue::Font) -> Pixmap {
    let mut pm = Pixmap::new(width, height).expect("pixmap");
    // Background: white
    pm.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
    paint_box(root, &mut pm, font);
    pm
}

fn paint_box(lb: &LayoutBox, pm: &mut Pixmap, font: &fontdue::Font) {
    // Background color
    if let BoxType::Block(node) = &lb.box_type {
        if let Some(bg) = node.lookup("background-color").and_then(|v| v.to_color())
            .or_else(|| node.lookup("background").and_then(|v| v.to_color()))
        {
            let r = lb.dimensions.padding_box();
            fill_rect(pm, r.x, r.y, r.w, r.h, bg);
        }
    }

    // Paint inline lines
    for line in &lb.lines {
        for item in &line.items {
            draw_text(pm, font, &item.text, item.x, item.y, item.font_size, item.color);
        }
    }

    // Paint children
    for c in &lb.children {
        paint_box(c, pm, font);
    }
}

fn fill_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: Color) {
    if w <= 0.0 || h <= 0.0 { return; }
    let mut paint = Paint::default();
    paint.set_color_rgba8(c.r, c.g, c.b, c.a);
    paint.anti_alias = true;
    if let Some(r) = SkRect::from_xywh(x, y, w, h) {
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }
}

fn draw_text(pm: &mut Pixmap, font: &fontdue::Font, text: &str, x: f32, baseline_y: f32, size: f32, color: Color) {
    let mut pen_x = x;
    for ch in text.chars() {
        let (metrics, bitmap) = font.rasterize(ch, size);
        if metrics.width == 0 || metrics.height == 0 {
            pen_x += metrics.advance_width;
            continue;
        }
        let glyph_x = pen_x + metrics.xmin as f32;
        let glyph_y = baseline_y - (metrics.height as f32 + metrics.ymin as f32);
        blit_coverage(pm, &bitmap, metrics.width, metrics.height, glyph_x, glyph_y, color);
        pen_x += metrics.advance_width;
    }
}

fn blit_coverage(pm: &mut Pixmap, cov: &[u8], w: usize, h: usize, ox: f32, oy: f32, color: Color) {
    let pm_w = pm.width() as i32;
    let pm_h = pm.height() as i32;
    let data = pm.data_mut();
    for j in 0..h {
        for i in 0..w {
            let c = cov[j * w + i];
            if c == 0 { continue; }
            let px = (ox + i as f32).round() as i32;
            let py = (oy + j as f32).round() as i32;
            if px < 0 || py < 0 || px >= pm_w || py >= pm_h { continue; }
            let idx = ((py * pm_w + px) * 4) as usize;
            // source-over alpha blending. Premultiplied in tiny-skia Pixmap.
            let a = (c as u32 * color.a as u32 / 255) as u8;
            if a == 0 { continue; }
            let sr = (color.r as u32 * a as u32 / 255) as u8;
            let sg = (color.g as u32 * a as u32 / 255) as u8;
            let sb = (color.b as u32 * a as u32 / 255) as u8;
            let dr = data[idx];
            let dg = data[idx + 1];
            let db = data[idx + 2];
            let da = data[idx + 3];
            let inv = 255 - a as u32;
            data[idx]     = (sr as u32 + dr as u32 * inv / 255) as u8;
            data[idx + 1] = (sg as u32 + dg as u32 * inv / 255) as u8;
            data[idx + 2] = (sb as u32 + db as u32 * inv / 255) as u8;
            data[idx + 3] = (a as u32 + da as u32 * inv / 255) as u8;
        }
    }
}
