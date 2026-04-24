//! DevTools bottom-panel overlay.
//!
//! Composes the rendered page pixmap with an extra panel drawn underneath
//! containing three tabs' worth of info, side by side:
//!
//! 1. **Elements** — condensed DOM tree (tag + id/class) indented by depth.
//! 2. **Network** — one row per request: status, bytes, duration, URL tail.
//! 3. **Console** — the captured `console.log` output.
//!
//! The panel is rasterized directly onto a tiny-skia `Pixmap` using the
//! engine's bundled sans font via `fontdue` — no layout engine involved,
//! just fixed rows of text.

use std::path::Path;

use fontdue::Font;
use tiny_skia::{Paint, Pixmap, Rect as SkRect, Transform};

use crate::css::Color as CssColor;
use crate::html::Node;
use crate::net::NetEntry;

const PANEL_HEIGHT: u32 = 280;
const TAB_WIDTH: u32 = 360;
const GUTTER: u32 = 12;
const HEADER_H: u32 = 28;
const ROW_H: f32 = 14.0;

pub struct DevToolsData<'a> {
    pub dom: &'a Node,
    pub network: &'a [NetEntry],
    pub console: &'a [String],
}

/// Build a new pixmap that stacks the rendered page on top and a devtools
/// panel on the bottom. Returns the composed pixmap.
pub fn compose(page: &Pixmap, data: &DevToolsData<'_>, font: &Font) -> Pixmap {
    let w = page.width();
    let h = page.height() + PANEL_HEIGHT;
    let mut out = Pixmap::new(w, h).expect("devtools pixmap");
    out.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 255));

    // Blit the page unchanged.
    if let Some(page_paint) = tiny_skia::PixmapPaint::default().into() {
        let _ = page_paint;
    }
    out.draw_pixmap(
        0,
        0,
        page.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::identity(),
        None,
    );

    draw_panel(&mut out, page.height(), data, font);
    out
}

/// Save composed pixmap + devtools directly.
pub fn save_with_panel<P: AsRef<Path>>(
    page: &Pixmap,
    data: &DevToolsData<'_>,
    font: &Font,
    path: P,
) -> std::io::Result<()> {
    let composed = compose(page, data, font);
    composed
        .save_png(path.as_ref())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
}

fn draw_panel(pm: &mut Pixmap, y0: u32, data: &DevToolsData<'_>, font: &Font) {
    let w = pm.width();

    // Panel background + top separator.
    fill_rect_u(pm, 0, y0, w, PANEL_HEIGHT, rgb(32, 34, 40));
    fill_rect_u(pm, 0, y0, w, 2, rgb(80, 84, 92));

    // Header strip: tabs with labels.
    fill_rect_u(pm, 0, y0 + 2, w, HEADER_H, rgb(24, 26, 31));
    let labels = ["Elements", "Network", "Console"];
    for (i, label) in labels.iter().enumerate() {
        let x = (GUTTER + i as u32 * TAB_WIDTH) as f32;
        draw_text(pm, font, label, x, y0 as f32 + 22.0, 13.0, rgb(230, 230, 235));
    }

    // Column dividers.
    for i in 1..labels.len() {
        let x = (i as u32 * TAB_WIDTH) as i32;
        if x < w as i32 {
            fill_rect_u(pm, x as u32, y0 + 2, 1, PANEL_HEIGHT - 2, rgb(50, 52, 60));
        }
    }

    let body_y = y0 + HEADER_H + 8;
    let body_h = PANEL_HEIGHT - HEADER_H - 10;
    let max_rows = (body_h as f32 / ROW_H) as usize;

    // 1. Elements tab — walk DOM
    {
        let mut rows = Vec::new();
        collect_dom_rows(data.dom, 0, &mut rows, max_rows);
        let x0 = GUTTER as f32;
        for (i, (depth, text)) in rows.iter().take(max_rows).enumerate() {
            let x = x0 + (*depth as f32) * 10.0;
            let y = body_y as f32 + 10.0 + i as f32 * ROW_H;
            draw_text(pm, font, text, x, y, 11.0, rgb(200, 210, 220));
        }
    }

    // 2. Network tab
    {
        let x0 = TAB_WIDTH as f32 + GUTTER as f32;
        if data.network.is_empty() {
            draw_text(pm, font, "(no requests)", x0, body_y as f32 + 10.0, 11.0, rgb(140, 140, 150));
        } else {
            for (i, e) in data.network.iter().take(max_rows).enumerate() {
                let y = body_y as f32 + 10.0 + i as f32 * ROW_H;
                let color = if e.from_cache {
                    rgb(170, 170, 180)
                } else if (200..300).contains(&e.status) {
                    rgb(140, 220, 150)
                } else {
                    rgb(230, 120, 120)
                };
                let tag = if e.from_cache { "CACHE" } else { &e.http_version };
                let short = short_url(&e.url, 34);
                let line = format!(
                    "{:<5} {:>3} {:>6}B {:>4}ms  {}",
                    tag, e.status, e.bytes, e.duration_ms, short
                );
                draw_text(pm, font, &line, x0, y, 10.5, color);
            }
        }
    }

    // 3. Console tab
    {
        let x0 = (TAB_WIDTH * 2) as f32 + GUTTER as f32;
        if data.console.is_empty() {
            draw_text(pm, font, "(no output)", x0, body_y as f32 + 10.0, 11.0, rgb(140, 140, 150));
        } else {
            for (i, line) in data.console.iter().rev().take(max_rows).enumerate() {
                let y = body_y as f32 + 10.0 + i as f32 * ROW_H;
                draw_text(pm, font, &truncate(line, 42), x0, y, 11.0, rgb(220, 225, 230));
            }
        }
    }

    // Footer hint.
    let footer_y = y0 + PANEL_HEIGHT - 14;
    fill_rect_u(pm, 0, footer_y, w, 14, rgb(20, 22, 26));
    draw_text(
        pm,
        font,
        "browser-engine devtools — M6",
        (GUTTER) as f32,
        (footer_y + 11) as f32,
        10.0,
        rgb(130, 130, 140),
    );
}

fn collect_dom_rows(
    node: &Node,
    depth: usize,
    rows: &mut Vec<(usize, String)>,
    limit: usize,
) {
    if rows.len() >= limit {
        return;
    }
    match node {
        Node::Text(t) => {
            let clean: String = t
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            let trimmed = clean.trim();
            if !trimmed.is_empty() {
                let show = truncate(trimmed, 48);
                rows.push((depth, format!("\"{}\"", show)));
            }
        }
        Node::Element(e) => {
            let mut label = format!("<{}>", e.tag);
            if let Some(id) = e.attrs.get("id") {
                label = format!("<{} #{}>", e.tag, id);
            } else if let Some(cls) = e.attrs.get("class") {
                let first = cls.split_whitespace().next().unwrap_or("");
                if !first.is_empty() {
                    label = format!("<{}.{}>", e.tag, first);
                }
            }
            rows.push((depth, label));
            for c in &e.children {
                if rows.len() >= limit {
                    break;
                }
                collect_dom_rows(c, depth + 1, rows, limit);
            }
        }
    }
}

fn short_url(u: &str, max: usize) -> String {
    // Drop scheme, keep host + path tail.
    let no_scheme = u
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    truncate(no_scheme, max)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn rgb(r: u8, g: u8, b: u8) -> CssColor {
    CssColor { r, g, b, a: 255 }
}

fn fill_rect_u(pm: &mut Pixmap, x: u32, y: u32, w: u32, h: u32, c: CssColor) {
    if w == 0 || h == 0 {
        return;
    }
    let mut paint = Paint::default();
    paint.set_color_rgba8(c.r, c.g, c.b, c.a);
    paint.anti_alias = false;
    if let Some(r) = SkRect::from_xywh(x as f32, y as f32, w as f32, h as f32) {
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }
}

/// Minimal left-to-right text blit using fontdue coverage bitmaps.
fn draw_text(pm: &mut Pixmap, font: &Font, text: &str, x: f32, baseline_y: f32, size: f32, c: CssColor) {
    let mut pen = x;
    for ch in text.chars() {
        let (metrics, bitmap) = font.rasterize(ch, size);
        if metrics.width > 0 && metrics.height > 0 {
            let gx = pen + metrics.xmin as f32;
            let gy = baseline_y - (metrics.height as f32 + metrics.ymin as f32);
            blit(pm, &bitmap, metrics.width, metrics.height, gx, gy, c);
        }
        pen += metrics.advance_width;
    }
}

fn blit(pm: &mut Pixmap, cov: &[u8], w: usize, h: usize, ox: f32, oy: f32, color: CssColor) {
    let pm_w = pm.width() as i32;
    let pm_h = pm.height() as i32;
    let data = pm.data_mut();
    for j in 0..h {
        for i in 0..w {
            let cv = cov[j * w + i];
            if cv == 0 {
                continue;
            }
            let px = (ox + i as f32).round() as i32;
            let py = (oy + j as f32).round() as i32;
            if px < 0 || py < 0 || px >= pm_w || py >= pm_h {
                continue;
            }
            let idx = ((py * pm_w + px) * 4) as usize;
            let a = (cv as u32 * color.a as u32 / 255) as u8;
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
