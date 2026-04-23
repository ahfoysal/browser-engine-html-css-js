//! Paint the layout tree onto a tiny-skia Pixmap and write PNG. M3 adds
//! `opacity`, `box-shadow` (outer drop shadow with a separable box blur),
//! stacking-context ordering via `z-index`, and glyph rasterization driven
//! by rustybuzz-shaped glyph indices (so kerning + ligatures survive all
//! the way to the pixmap).

use tiny_skia::{
    FillRule, Paint, PathBuilder, Pixmap, Rect as SkRect, Stroke, Transform,
};

use crate::css::Color;
use crate::layout::{BoxType, FontFamily, FontSet, LayoutBox, Radii, Shadow};

pub fn paint(root: &LayoutBox, width: u32, height: u32, fonts: &FontSet) -> Pixmap {
    let mut pm = Pixmap::new(width, height).expect("pixmap");
    pm.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
    paint_box(root, &mut pm, fonts, 1.0);
    pm
}

/// Walks a subtree and paints it with stacking-context ordering. Children
/// are split into three buckets based on `z-index`: negatives (painted
/// behind), zero/auto (flow order), positives (above). Effectively a tiny
/// subset of CSS 2.1 §9.9 stacking.
fn paint_box(lb: &LayoutBox, pm: &mut Pixmap, fonts: &FontSet, parent_opacity: f32) {
    let effective_opacity = parent_opacity * lb.opacity;
    if effective_opacity <= 0.0 {
        return;
    }

    // Drop shadow first (painted under background).
    if let Some(sh) = lb.shadow {
        paint_shadow(pm, lb, sh, effective_opacity);
    }

    // Background fill (padding-box clipped by border-radius).
    if let Some(bg) = lb.bg {
        let r = lb.dimensions.padding_box();
        let bg = apply_opacity(bg, effective_opacity);
        fill_rounded_rect(pm, r.x, r.y, r.w, r.h, &lb.radii, bg);
    }

    // Borders (drawn along the inside edge of the border-box, per-side).
    if has_border(lb) {
        paint_borders(lb, pm, effective_opacity);
    }

    // Inline lines
    if let BoxType::Block(_) = &lb.box_type {
        for line in &lb.lines {
            for item in &line.items {
                let face = fonts.pick(item.family, item.bold, item.italic);
                draw_shaped_text(
                    pm,
                    face.fontdue,
                    &item.glyphs,
                    item.x,
                    item.y,
                    item.font_size,
                    apply_opacity(item.color, effective_opacity),
                    item.bold && item.family == FontFamily::Sans && !item.italic
                        && std::ptr::eq(face.fontdue, fonts.sans.fontdue),
                );
            }
        }
    }

    // Paint children in z-order. `0` / default keeps tree order.
    let mut ordered: Vec<&LayoutBox> = lb.children.iter().collect();
    ordered.sort_by_key(|c| c.z_index);
    for c in ordered {
        paint_box(c, pm, fonts, effective_opacity);
    }
}

fn apply_opacity(c: Color, opacity: f32) -> Color {
    let a = (c.a as f32 * opacity).clamp(0.0, 255.0) as u8;
    Color {
        r: c.r,
        g: c.g,
        b: c.b,
        a,
    }
}

fn has_border(lb: &LayoutBox) -> bool {
    lb.borders.top.width > 0.0
        || lb.borders.right.width > 0.0
        || lb.borders.bottom.width > 0.0
        || lb.borders.left.width > 0.0
}

fn paint_borders(lb: &LayoutBox, pm: &mut Pixmap, opacity: f32) {
    let bb = lb.dimensions.border_box();
    let b = &lb.borders;
    let radii = &lb.radii;
    let any_radius =
        radii.tl > 0.0 || radii.tr > 0.0 || radii.br > 0.0 || radii.bl > 0.0;

    if any_radius {
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
        let inset = max_w / 2.0;
        let x = bb.x + inset;
        let y = bb.y + inset;
        let w = (bb.w - max_w).max(0.0);
        let h = (bb.h - max_w).max(0.0);
        let path = rounded_rect_path(x, y, w, h, radii, inset);
        if let Some(p) = path {
            let mut paint = Paint::default();
            let c = apply_opacity(stroke_color, opacity);
            paint.set_color_rgba8(c.r, c.g, c.b, c.a);
            paint.anti_alias = true;
            let mut stroke = Stroke::default();
            stroke.width = max_w;
            pm.stroke_path(&p, &paint, &stroke, Transform::identity(), None);
        }
        return;
    }

    if b.top.width > 0.0 {
        fill_rect(pm, bb.x, bb.y, bb.w, b.top.width, apply_opacity(b.top.color, opacity));
    }
    if b.bottom.width > 0.0 {
        fill_rect(
            pm,
            bb.x,
            bb.y + bb.h - b.bottom.width,
            bb.w,
            b.bottom.width,
            apply_opacity(b.bottom.color, opacity),
        );
    }
    if b.left.width > 0.0 {
        fill_rect(pm, bb.x, bb.y, b.left.width, bb.h, apply_opacity(b.left.color, opacity));
    }
    if b.right.width > 0.0 {
        fill_rect(
            pm,
            bb.x + bb.w - b.right.width,
            bb.y,
            b.right.width,
            bb.h,
            apply_opacity(b.right.color, opacity),
        );
    }
}

/// Render a drop shadow: expand the border-box by `spread`, offset by
/// (ox, oy), then blur the mask with a separable box blur repeated three
/// times (≈ Gaussian) and blit the tinted result onto the pixmap.
fn paint_shadow(pm: &mut Pixmap, lb: &LayoutBox, sh: Shadow, opacity: f32) {
    if sh.color.a == 0 {
        return;
    }
    let bb = lb.dimensions.border_box();
    let rect_x = bb.x + sh.ox - sh.spread;
    let rect_y = bb.y + sh.oy - sh.spread;
    let rect_w = bb.w + 2.0 * sh.spread;
    let rect_h = bb.h + 2.0 * sh.spread;
    if rect_w <= 0.0 || rect_h <= 0.0 {
        return;
    }

    let blur = sh.blur.max(0.0);
    let pad = (blur * 1.5).ceil() as i32 + 1;
    // Rasterize shadow mask at px-origin (gx0, gy0) with size (gw, gh).
    let gw = (rect_w as i32) + pad * 2;
    let gh = (rect_h as i32) + pad * 2;
    if gw <= 0 || gh <= 0 {
        return;
    }
    let gx0 = rect_x.floor() as i32 - pad;
    let gy0 = rect_y.floor() as i32 - pad;
    let mut mask: Vec<u8> = vec![0u8; (gw * gh) as usize];

    // Fill the inner rectangle (accounting for radii via a simple rounded mask).
    let rx = lb.radii.tl.max(lb.radii.tr).max(lb.radii.br).max(lb.radii.bl) + sh.spread.max(0.0);
    for j in 0..gh {
        for i in 0..gw {
            let px = (gx0 + i) as f32 + 0.5;
            let py = (gy0 + j) as f32 + 0.5;
            if px < rect_x || px > rect_x + rect_w || py < rect_y || py > rect_y + rect_h {
                continue;
            }
            // Simple rounded-corner test: distance to nearest corner.
            if rx > 0.0 {
                let dx_l = rect_x + rx - px;
                let dx_r = px - (rect_x + rect_w - rx);
                let dy_t = rect_y + rx - py;
                let dy_b = py - (rect_y + rect_h - rx);
                let dx = dx_l.max(dx_r).max(0.0);
                let dy = dy_t.max(dy_b).max(0.0);
                if dx > 0.0 && dy > 0.0 {
                    let d = (dx * dx + dy * dy).sqrt();
                    if d > rx {
                        continue;
                    }
                }
            }
            mask[(j * gw + i) as usize] = 255;
        }
    }

    if blur > 0.5 {
        // Separable box blur — 3 passes approximates a Gaussian.
        let radius = (blur / 2.0).max(1.0) as i32;
        for _ in 0..3 {
            mask = box_blur_h(&mask, gw, gh, radius);
            mask = box_blur_v(&mask, gw, gh, radius);
        }
    }

    // Blit mask tinted by shadow color.
    let color = apply_opacity(sh.color, opacity);
    let pm_w = pm.width() as i32;
    let pm_h = pm.height() as i32;
    let data = pm.data_mut();
    for j in 0..gh {
        for i in 0..gw {
            let m = mask[(j * gw + i) as usize];
            if m == 0 {
                continue;
            }
            let px = gx0 + i;
            let py = gy0 + j;
            if px < 0 || py < 0 || px >= pm_w || py >= pm_h {
                continue;
            }
            let a = (m as u32 * color.a as u32 / 255) as u8;
            if a == 0 {
                continue;
            }
            let idx = ((py * pm_w + px) * 4) as usize;
            blend(data, idx, color, a);
        }
    }
}

fn box_blur_h(src: &[u8], w: i32, h: i32, r: i32) -> Vec<u8> {
    let mut out = vec![0u8; src.len()];
    let win = (2 * r + 1) as u32;
    for j in 0..h {
        let row = (j * w) as usize;
        let mut sum: u32 = 0;
        // Prime initial window: 0..=r on left (treat out-of-bounds as 0).
        for k in 0..=r {
            if k < w {
                sum += src[row + k as usize] as u32;
            }
        }
        for i in 0..w {
            out[row + i as usize] = (sum / win) as u8;
            let add = i + r + 1;
            let sub = i - r;
            if add < w {
                sum += src[row + add as usize] as u32;
            }
            if sub >= 0 {
                sum = sum.saturating_sub(src[row + sub as usize] as u32);
            }
        }
    }
    out
}

fn box_blur_v(src: &[u8], w: i32, h: i32, r: i32) -> Vec<u8> {
    let mut out = vec![0u8; src.len()];
    let win = (2 * r + 1) as u32;
    for i in 0..w {
        let mut sum: u32 = 0;
        for k in 0..=r {
            if k < h {
                sum += src[(k * w + i) as usize] as u32;
            }
        }
        for j in 0..h {
            out[(j * w + i) as usize] = (sum / win) as u8;
            let add = j + r + 1;
            let sub = j - r;
            if add < h {
                sum += src[(add * w + i) as usize] as u32;
            }
            if sub >= 0 {
                sum = sum.saturating_sub(src[(sub * w + i) as usize] as u32);
            }
        }
    }
    out
}

fn colors_equal(a: Color, b: Color) -> bool {
    a.r == b.r && a.g == b.g && a.b == b.b && a.a == b.a
}

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

    let k = 0.5522847498;
    let mut pb = PathBuilder::new();
    pb.move_to(x + tl, y);
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

/// Rasterize a pre-shaped run. `glyphs` come from rustybuzz and carry the
/// correct per-glyph x offsets including kerning. We use
/// `fontdue::rasterize_indexed` so the glyph IDs agree with the shaper.
fn draw_shaped_text(
    pm: &mut Pixmap,
    font: &fontdue::Font,
    glyphs: &[crate::text::ShapedGlyph],
    x: f32,
    baseline_y: f32,
    size: f32,
    color: Color,
    synthetic_bold: bool,
) {
    for g in glyphs {
        let (metrics, bitmap) = font.rasterize_indexed(g.glyph_id, size);
        if metrics.width == 0 || metrics.height == 0 {
            continue;
        }
        let glyph_x = x + g.x + metrics.xmin as f32;
        let glyph_y = baseline_y + g.y_offset - (metrics.height as f32 + metrics.ymin as f32);
        blit_coverage(pm, &bitmap, metrics.width, metrics.height, glyph_x, glyph_y, color);
        if synthetic_bold {
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
            blend(data, idx, color, a);
        }
    }
}

fn blend(data: &mut [u8], idx: usize, color: Color, a: u8) {
    if a == 0 {
        return;
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
