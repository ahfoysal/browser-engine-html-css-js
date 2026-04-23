//! MVP browser engine CLI: input.html -> output.png
//!
//! Pipeline: HTML parse -> DOM -> CSS parse (UA + <style>) -> styled tree ->
//!           block/inline/flex layout with positioning -> paint via tiny-skia -> PNG.

mod html;
mod css;
mod style;
mod layout;
mod paint;
mod text;

use std::path::PathBuf;

use layout::{FontFace, FontSet, LayoutEngine, Rect};

// Bundled fonts. We ship a Sans family (regular + bold + italic), a Serif,
// and a Monospace so `font-family` fallback actually changes the render.
static FONT_SANS: &[u8] = include_bytes!("../assets/font.ttf");
static FONT_SANS_BOLD: &[u8] = include_bytes!("../assets/font-bold.ttf");
static FONT_SANS_ITALIC: &[u8] = include_bytes!("../assets/font-italic.ttf");
static FONT_SERIF: &[u8] = include_bytes!("../assets/font-serif.ttf");
static FONT_MONO: &[u8] = include_bytes!("../assets/font-mono.ttf");

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

    // 4. Load fonts. Each face carries both a fontdue::Font (rasterizer)
    //    and a rustybuzz::Face (shaper) built from the same bytes so the
    //    glyph indices agree. An override env var `BROWSER_FONT` lets
    //    callers swap the regular sans face with any TTF.
    let sans_bytes = load_bytes("BROWSER_FONT", FONT_SANS);
    let sans_bold_bytes = load_bytes("BROWSER_BOLD_FONT", FONT_SANS_BOLD);
    let sans_italic_bytes = load_bytes("BROWSER_ITALIC_FONT", FONT_SANS_ITALIC);
    let serif_bytes = load_bytes("BROWSER_SERIF_FONT", FONT_SERIF);
    let mono_bytes = load_bytes("BROWSER_MONO_FONT", FONT_MONO);

    let sans = make_fontdue(&sans_bytes, "sans");
    let sans_bold = make_fontdue(&sans_bold_bytes, "sans-bold");
    let sans_italic = make_fontdue(&sans_italic_bytes, "sans-italic");
    let serif = make_fontdue(&serif_bytes, "serif");
    let mono = make_fontdue(&mono_bytes, "mono");

    let fonts = FontSet {
        sans: FontFace {
            fontdue: &sans,
            buzz: rustybuzz::Face::from_slice(&sans_bytes, 0).expect("sans buzz"),
        },
        sans_bold: FontFace {
            fontdue: &sans_bold,
            buzz: rustybuzz::Face::from_slice(&sans_bold_bytes, 0).expect("sans-bold buzz"),
        },
        sans_italic: FontFace {
            fontdue: &sans_italic,
            buzz: rustybuzz::Face::from_slice(&sans_italic_bytes, 0).expect("sans-italic buzz"),
        },
        serif: FontFace {
            fontdue: &serif,
            buzz: rustybuzz::Face::from_slice(&serif_bytes, 0).expect("serif buzz"),
        },
        mono: FontFace {
            fontdue: &mono,
            buzz: rustybuzz::Face::from_slice(&mono_bytes, 0).expect("mono buzz"),
        },
    };

    // 5. Layout
    let engine = LayoutEngine {
        viewport: Rect { x: 0.0, y: 0.0, w: width as f32, h: height as f32 },
        fonts: &fonts,
    };
    let layout_root = engine.layout(&styled);

    // 6. Paint to PNG
    let pm = paint::paint(&layout_root, width, height, &fonts);
    pm.save_png(&output_path).expect("save png");
    println!("rendered {} -> {} ({}x{})", input_path.display(), output_path.display(), width, height);
}

fn load_bytes(env_var: &str, fallback: &[u8]) -> Vec<u8> {
    std::env::var(env_var)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .unwrap_or_else(|| fallback.to_vec())
}

fn make_fontdue(bytes: &[u8], label: &str) -> fontdue::Font {
    fontdue::Font::from_bytes(bytes.to_vec(), fontdue::FontSettings::default())
        .unwrap_or_else(|e| panic!("font parse ({}): {:?}", label, e))
}
