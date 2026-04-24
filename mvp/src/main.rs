//! MVP browser engine CLI: input.html -> output.png
//!
//! Pipeline: HTML parse -> DOM -> JS pass (QuickJS + DOM bindings) ->
//!           CSS parse (UA + <style>) -> styled tree ->
//!           block/inline/flex layout with positioning -> paint via tiny-skia -> PNG.
//!
//! M4 adds the JS pass: `<script>` blocks run through an embedded QuickJS
//! interpreter with a minimal browser-style DOM exposed. After scripts run
//! (and any `setTimeout`s drain) we render the mutated DOM. If the
//! `BROWSER_CLICK=<id>` env var is set, we also dispatch a synthetic click
//! on that element and emit a second `-after.png` showing the post-event
//! DOM — this is how the counter demo's "before / after" renders are made.

mod html;
mod css;
mod style;
mod layout;
mod paint;
mod text;
mod js;

use std::path::{Path, PathBuf};

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

    // Load fonts once — re-used for every render pass.
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

    // 1. Parse HTML
    let parsed = html::parse(&html_text);

    // 1a. JS pass.
    let working = js::Dom::from_html(&parsed);
    let has_scripts = !working.scripts.is_empty();

    if has_scripts {
        let scripts = working.scripts.clone();
        let runtime = js::JsRuntime::new(working).expect("create js runtime");
        for src in &scripts {
            if let Err(e) = runtime.eval(src) {
                eprintln!("[js] script error: {:?}", e);
            }
        }
        if let Err(e) = runtime.drain_timers() {
            eprintln!("[js] timer error: {:?}", e);
        }

        let initial = runtime.dom.borrow().to_html();
        render(&initial, &output_path, width, height, &fonts, &input_path);

        if let Ok(id) = std::env::var("BROWSER_CLICK") {
            let nid_opt = runtime.dom.borrow().get_by_id(&id);
            if let Some(nid) = nid_opt {
                match runtime.dispatch_event(nid, "click") {
                    Ok(n) => println!("[js] dispatched click -> #{} ({} listener(s))", id, n),
                    Err(e) => eprintln!("[js] dispatch error: {:?}", e),
                }
                if let Err(e) = runtime.drain_timers() {
                    eprintln!("[js] post-click timer error: {:?}", e);
                }
                let after = runtime.dom.borrow().to_html();
                let after_path = append_suffix(&output_path, "-after");
                render(&after, &after_path, width, height, &fonts, &input_path);
            } else {
                eprintln!("[js] BROWSER_CLICK target '#{}' not found", id);
            }
        }
        for line in &runtime.console.borrow().lines {
            println!("[console] {}", line);
        }
        return;
    }

    render(&parsed, &output_path, width, height, &fonts, &input_path);
}

fn render(
    dom: &html::Node,
    output_path: &Path,
    width: u32,
    height: u32,
    fonts: &FontSet,
    input_path: &Path,
) {
    let mut css_text = String::from(style::user_agent_css());
    css_text.push_str(&html::extract_styles(dom));
    let stylesheet = css::parse(&css_text);
    let styled = style::style_tree(dom, &stylesheet);
    let engine = LayoutEngine {
        viewport: Rect { x: 0.0, y: 0.0, w: width as f32, h: height as f32 },
        fonts,
    };
    let layout_root = engine.layout(&styled);
    let pm = paint::paint(&layout_root, width, height, fonts);
    pm.save_png(output_path).expect("save png");
    println!(
        "rendered {} -> {} ({}x{})",
        input_path.display(),
        output_path.display(),
        width,
        height
    );
}

fn append_suffix(p: &Path, suffix: &str) -> PathBuf {
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("png");
    let parent = p.parent().unwrap_or_else(|| Path::new(""));
    parent.join(format!("{}{}.{}", stem, suffix, ext))
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
