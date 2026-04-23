//! MVP browser engine CLI: input.html -> output.png
//!
//! Pipeline: HTML parse -> DOM -> CSS parse (UA + <style>) -> styled tree ->
//!           block layout -> paint via tiny-skia -> PNG.

mod html;
mod css;
mod style;
mod layout;
mod paint;

use std::path::PathBuf;

use layout::{LayoutEngine, Rect};

// Bundled font: DejaVu-like TTF. We use a minimal embedded font to avoid system deps.
// For simplicity, load a font at runtime if provided via FONT env var; otherwise fallback to bundled.
static DEFAULT_FONT: &[u8] = include_bytes!("../assets/font.ttf");

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: {} input.html output.png [width] [height]", args[0]);
        std::process::exit(1);
    }
    let input_path = PathBuf::from(&args[1]);
    let output_path = PathBuf::from(&args[2]);
    let width: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(800);
    let height: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1000);

    let html_text = std::fs::read_to_string(&input_path).expect("read input html");

    // 1. Parse HTML
    let dom = html::parse(&html_text);
    // 2. Collect CSS: user-agent + any <style> blocks
    let mut css_text = String::from(style::user_agent_css());
    css_text.push_str(&html::extract_styles(&dom));
    let stylesheet = css::parse(&css_text);
    // 3. Build styled tree
    let styled = style::style_tree(&dom, &stylesheet);

    // 4. Load font
    let font_bytes = std::env::var("BROWSER_FONT")
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .unwrap_or_else(|| DEFAULT_FONT.to_vec());
    let font = fontdue::Font::from_bytes(font_bytes.clone(), fontdue::FontSettings::default())
        .expect("font parse");
    // Bold font: use $BROWSER_BOLD_FONT if set, else reuse regular. Paint applies
    // a synthetic-bold second pass when the font lacks a real bold weight.
    let bold_font_bytes = std::env::var("BROWSER_BOLD_FONT")
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .unwrap_or(font_bytes);
    let bold_font =
        fontdue::Font::from_bytes(bold_font_bytes, fontdue::FontSettings::default())
            .expect("bold font parse");

    // 5. Layout
    let engine = LayoutEngine {
        viewport: Rect { x: 0.0, y: 0.0, w: width as f32, h: height as f32 },
        font: &font,
        bold_font: &bold_font,
    };
    let layout_root = engine.layout(&styled);

    // 6. Paint to PNG
    let pm = paint::paint(&layout_root, width, height, &font, &bold_font);
    pm.save_png(&output_path).expect("save png");
    println!("rendered {} -> {} ({}x{})", input_path.display(), output_path.display(), width, height);
}
