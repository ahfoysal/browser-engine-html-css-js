//! Paint the layout tree onto a tiny-skia Pixmap and write PNG. M2 adds
//! border painting, `border-radius` rounded rects (clip + stroke), and a
//! separately-rasterized bold font for `font-weight: bold`.

use tiny_skia::{
    FillRule, Paint, PathBuilder, Pixmap, Rect as SkRect, Stroke, Transform,
};

use crate::css::Color;
use crate::layout::{BoxType, LayoutBox, Radii};

pub fn paint(
    root: &LayoutBox,
    width: u32,
    height: u32,
    font: &fontdue::Font,
    bold_font: &fontdue::Font,
) -> Pixmap {
    let mut pm = Pixmap::new(width, height).expect("pixmap");
    pm.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
    paint_box(root, &mut pm, font, bold_font);
    pm
}

fn paint_box(
    lb: &LayoutBox,
    pm: &mut Pixmap,
    font: &fontdue::Font,
    bold_font: &fontdue::Font,
) {
    // Background fill (padding-box clipped by border-radius).
    if let Some(bg) = lb.bg {
        let r = lb.dimensions.padding_box();
        fill_rounded_rect(pm, r.x, r.y, r.w, r.h, &lb.radii, bg);
    }

    // Borders (drawn along the inside edge of the border-box, per-side).
    if has_border(lb) {
        paint_borders(lb, pm);
    }

    // Inline lines
    if let BoxType::Block(_) = &lb.box_type {
        for line in &lb.lines {
            for item in &line.items {
                let f = if item.bold { bold_font } else { font };
                draw_text(pm, f, &item.text, item.x, item.y, item.font_size, item.color, item.bold);
            }
        }
    }

    // Paint children
    for c in &lb.children {
        paint_box(c, pm, font, bold_font);
    }
}

fn has_border(lb: &LayoutBox) -> bool {
    lb.borders.top.width > 0.0
        || lb.borders.right.width > 0.0
        || lb.borders.bottom.width > 0.0
        || lb.borders.left.width > 0.0
}

fn paint_borders(lb: &LayoutBox, pm: &mut Pixmap) {
    let bb = lb.dimensions.border_box();
    let b = &lb.borders;
    let radii = &lb.radii;
    let any_radius =
        radii.tl > 0.0 || radii.tr > 0.0 || radii.br > 0.0 || radii.bl > 0.0;

    if any_radius {
        // For rounded borders we approximate: stroke the outer rounded rect
        // with the max border width, using the top border's color when all
        // four sides match, otherwise pick the thickest side.
        let all_same_color = colors_equal(b.top.color, b.right.color)
            && colors_equal(b.right.color, b.bottom.color)
            && colors_equal(b.bottom.color, b.left.color);
        let widths = [b.top.width, b.right.width, b.bottom.width, b.left.width];
        let max_w = widths.iter().cloned().fold(0.0_f32, f32::max);
        if max_w <= 0.0 {
            return;
        }
        let stroke_color = if all_same_color {
            b.top.color
        } else {
            // pick the side with the largest width
            let mut idx = 0usize;
            for i in 1..4 {
                if widths[i] > widths[idx] {
                    idx = i;
                }
            }
            match idx {
                0 => b.top.color,
                1 => b.right.color,
                2 => b.bottom.color,
                _ => b.left.color,
            }
        };
        // Stroke centered on the border rect inset by half the width.
        let inset = max_w / 2.0;
        let x = bb.x + inset;
        let y = bb.y + inset;
        let w = (bb.w - max_w).max(0.0);
        let h = (bb.h - max_w).max(0.0);
        let path = rounded_rect_path(x, y, w, h, radii, inset);
        if let Some(p) = path {
            let mut paint = Paint::default();
            paint.set_color_rgba8(
                stroke_color.r,
                stroke_color.g,
                stroke_color.b,
                stroke_color.a,
            );
            paint.anti_alias = true;
            let mut stroke = Stroke::default();
            stroke.width = max_w;
            pm.stroke_path(&p, &paint, &stroke, Transform::identity(), None);
        }
        return;
    }

    // Non-rounded: fill four rectangles (with mitered corners implicit).
    // top
    if b.top.width > 0.0 {
        fill_rect(pm, bb.x, bb.y, bb.w, b.top.width, b.top.color);
    }
    // bottom
    if b.bottom.width > 0.0 {
        fill_rect(
            pm,
            bb.x,
            bb.y + bb.h - b.bottom.width,
            bb.w,
            b.bottom.width,
            b.bottom.color,
        );
    }
    // left
    if b.left.width > 0.0 {
        fill_rect(pm, bb.x, bb.y, b.left.width, bb.h, b.left.color);
    }
    // right
    if b.right.width > 0.0 {
        fill_rect(
            pm,
            bb.x + bb.w - b.right.width,
            bb.y,
            b.right.width,
            bb.h,
            b.right.color,
        );
    }
}

fn colors_equal(a: Color, b: Color) -> bool {
    a.r == b.r && a.g == b.g && a.b == b.b && a.a == b.a
}

/// Build a rounded rect path. `radii` is per-corner; we clamp to half the
/// smallest side. `_inset` is informational — the caller has already
/// inset x/y/w/h when stroking.
fn rounded_rect_path(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radii: &Radii,
    _inset: f32,
) -> Option<tiny_skia::Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let max_r = (w.min(h)) / 2.0;
    let tl = radii.tl.min(max_r).max(0.0);
    let tr = radii.tr.min(max_r).max(0.0);
    let br = radii.br.min(max_r).max(0.0);
    let bl = radii.bl.min(max_r).max(0.0);

    // Approximation constant for a quarter circle with cubic Bezier.
    let k = 0.5522847498;
    let mut pb = PathBuilder::new();
    // Start at top-left corner end
    pb.move_to(x + tl, y);
    // top edge -> top-right
    pb.line_to(x + w - tr, y);
    if tr > 0.0 {
        pb.cubic_to(
            x + w - tr + tr * k,
            y,
            x + w,
            y + tr - tr * k,
            x + w,
            y + tr,
        );
    }
    // right edge -> bottom-right
    pb.line_to(x + w, y + h - br);
    if br > 0.0 {
        pb.cubic_to(
            x + w,
            y + h - br + br * k,
            x + w - br + br * k,
            y + h,
            x + w - br,
            y + h,
        );
    }
    // bottom edge -> bottom-left
    pb.line_to(x + bl, y + h);
    if bl > 0.0 {
        pb.cubic_to(
            x + bl - bl * k,
            y + h,
            x,
            y + h - bl + bl * k,
            x,
            y + h - bl,
        );
    }
    // left edge -> top-left
    pb.line_to(x, y + tl);
    if tl > 0.0 {
        pb.cubic_to(
            x,
            y + tl - tl * k,
            x + tl - tl * k,
            y,
            x + tl,
            y,
        );
    }
    pb.close();
    pb.finish()
}

fn fill_rounded_rect(
    pm: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radii: &Radii,
    c: Color,
) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let any = radii.tl > 0.0 || radii.tr > 0.0 || radii.br > 0.0 || radii.bl > 0.0;
    if !any {
        fill_rect(pm, x, y, w, h, c);
        return;
    }
    if let Some(p) = rounded_rect_path(x, y, w, h, radii, 0.0) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c.r, c.g, c.b, c.a);
        paint.anti_alias = true;
        pm.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn fill_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: Color) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let mut paint = Paint::default();
    paint.set_color_rgba8(c.r, c.g, c.b, c.a);
    paint.anti_alias = true;
    if let Some(r) = SkRect::from_xywh(x, y, w, h) {
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }
}

fn draw_text(
    pm: &mut Pixmap,
    font: &fontdue::Font,
    text: &str,
    x: f32,
    baseline_y: f32,
    size: f32,
    color: Color,
    bold: bool,
) {
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
        // Synthetic bold: draw a second pass at +1px if we don't have a bold font
        // that differs from the regular font. Cheap, legible.
        if bold {
            blit_coverage(
                pm,
                &bitmap,
                metrics.width,
                metrics.height,
                glyph_x + 0.6,
                glyph_y,
                color,
            );
        }
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
            if c == 0 {
                continue;
            }
            let px = (ox + i as f32).round() as i32;
            let py = (oy + j as f32).round() as i32;
            if px < 0 || py < 0 || px >= pm_w || py >= pm_h {
                continue;
            }
            let idx = ((py * pm_w + px) * 4) as usize;
            let a = (c as u32 * color.a as u32 / 255) as u8;
            if a == 0 {
                continue;
            }
            let sr = (color.r as u32 * a as u32 / 255) as u8;
            let sg = (color.g as u32 * a as u32 / 255) as u8;
            let sb = (color.b as u32 * a as u32 / 255) as u8;
            let dr = data[idx];
            let dg = data[idx + 1];
            let db = data[idx + 2];
            let da = data[idx + 3];
            let inv = 255 - a as u32;
            data[idx] = (sr as u32 + dr as u32 * inv / 255) as u8;
            data[idx + 1] = (sg as u32 + dg as u32 * inv / 255) as u8;
            data[idx + 2] = (sb as u32 + db as u32 * inv / 255) as u8;
            data[idx + 3] = (a as u32 + da as u32 * inv / 255) as u8;
        }
    }
}
