//! Layout engine. M3 adds CSS positioning (`relative`, `absolute`, `fixed`),
//! `top/left/right/bottom`, `z-index`, `opacity`, `box-shadow`, real
//! `font-family` fallback lists, `font-style: italic`, and rustybuzz text
//! shaping for proper kerning.
//!
//! Earlier milestones added borders, `border-radius`, flexbox-lite, inline
//! layout with line boxes, `text-align`, `line-height`, and `font-weight`.

use crate::css::{Color, Value};
use crate::style::{Display, StyledNode};
use crate::text;

#[derive(Debug, Clone, Copy, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EdgeSizes {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// Per-side border: width + color + style keyword (only `solid` is painted;
/// anything else falls back to solid if width > 0).
#[derive(Debug, Clone, Copy, Default)]
pub struct BorderSide {
    pub width: f32,
    pub color: Color,
    pub solid: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Borders {
    pub top: BorderSide,
    pub right: BorderSide,
    pub bottom: BorderSide,
    pub left: BorderSide,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Radii {
    pub tl: f32,
    pub tr: f32,
    pub br: f32,
    pub bl: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Dimensions {
    pub content: Rect,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub margin: EdgeSizes,
}

impl Dimensions {
    pub fn padding_box(&self) -> Rect {
        expand(self.content, self.padding)
    }
    pub fn border_box(&self) -> Rect {
        expand(self.padding_box(), self.border)
    }
    pub fn margin_box(&self) -> Rect {
        expand(self.border_box(), self.margin)
    }
}

fn expand(r: Rect, e: EdgeSizes) -> Rect {
    Rect {
        x: r.x - e.left,
        y: r.y - e.top,
        w: r.w + e.left + e.right,
        h: r.h + e.top + e.bottom,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
}

impl Default for Position {
    fn default() -> Self {
        Position::Static
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Offsets {
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
}

/// Drop-shadow specification resolved from the `box-shadow` property.
#[derive(Debug, Clone, Copy)]
pub struct Shadow {
    pub ox: f32,
    pub oy: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub dimensions: Dimensions,
    pub box_type: BoxType,
    pub children: Vec<LayoutBox>,
    /// Inline lines: for a block that has inline children, we build line boxes here.
    pub lines: Vec<LineBox>,
    pub borders: Borders,
    pub radii: Radii,
    pub bg: Option<Color>,
    pub position: Position,
    pub offsets: Offsets,
    pub z_index: i32,
    pub opacity: f32,
    pub shadow: Option<Shadow>,
}

#[derive(Debug, Clone)]
pub enum BoxType {
    Block(StyledNode),
    Anonymous,
}

#[derive(Debug, Clone, Default)]
pub struct LineBox {
    pub y: f32,
    pub height: f32,
    pub baseline: f32,
    pub items: Vec<InlineItem>,
}

#[derive(Debug, Clone)]
pub struct InlineItem {
    pub text: String,
    pub x: f32,
    /// Baseline Y (absolute).
    pub y: f32,
    pub width: f32,
    pub font_size: f32,
    pub color: Color,
    pub bold: bool,
    pub italic: bool,
    /// Which bundled font family was resolved for this run. Paint uses this
    /// to pick the right fontdue face — lets `font-family: serif` and
    /// `font-family: monospace` render with visually distinct glyphs.
    pub family: FontFamily,
    /// Pre-shaped glyphs: indices + per-glyph x positions in pixels,
    /// relative to `x`. Populated during layout via rustybuzz so paint
    /// gets proper kerning for free.
    pub glyphs: Vec<text::ShapedGlyph>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFamily {
    Sans,
    Serif,
    Monospace,
}

impl Default for FontFamily {
    fn default() -> Self {
        FontFamily::Sans
    }
}

fn px(node: &StyledNode, key: &str, default: f32) -> f32 {
    node.lookup(key).map(|v| v.to_px()).unwrap_or(default)
}

fn color(node: &StyledNode, key: &str, default: Color) -> Color {
    node.lookup(key).and_then(|v| v.to_color()).unwrap_or(default)
}

fn keyword<'a>(node: &'a StyledNode, key: &str) -> Option<String> {
    node.lookup(key).and_then(|v| v.to_keyword().map(|s| s.to_string()))
}

fn number(node: &StyledNode, key: &str, default: f32) -> f32 {
    node.lookup(key)
        .and_then(|v| v.to_number())
        .unwrap_or(default)
}

fn is_bold(node: &StyledNode) -> bool {
    // font-weight keyword or number
    if let Some(v) = node.lookup("font-weight") {
        if let Some(n) = v.to_number() {
            return n >= 600.0;
        }
        if let Some(k) = v.to_keyword() {
            return matches!(k, "bold" | "bolder");
        }
    }
    // default bold for <b>, <strong>, <h1..h6>
    if let StyledNode::Element { tag, .. } = node {
        return matches!(
            tag.as_str(),
            "b" | "strong" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        );
    }
    false
}

fn resolve_borders(node: &StyledNode) -> Borders {
    let side = |side: &str| -> BorderSide {
        let w = node
            .lookup(&format!("border-{}-width", side))
            .map(|v| v.to_px())
            .unwrap_or(0.0);
        let c = node
            .lookup(&format!("border-{}-color", side))
            .and_then(|v| v.to_color())
            .unwrap_or(Color {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            });
        let style = node
            .lookup(&format!("border-{}-style", side))
            .and_then(|v| v.to_keyword().map(|s| s.to_string()))
            .unwrap_or_else(|| "solid".to_string());
        let solid = style != "none" && style != "hidden" && w > 0.0;
        BorderSide {
            width: if solid { w } else { 0.0 },
            color: c,
            solid,
        }
    };
    Borders {
        top: side("top"),
        right: side("right"),
        bottom: side("bottom"),
        left: side("left"),
    }
}

fn resolve_position(node: &StyledNode) -> (Position, Offsets) {
    let pos = match keyword(node, "position").as_deref() {
        Some("relative") => Position::Relative,
        Some("absolute") => Position::Absolute,
        Some("fixed") => Position::Fixed,
        _ => Position::Static,
    };
    let side = |name: &str| -> Option<f32> {
        node.lookup(name).map(|v| v.to_px())
    };
    let offsets = Offsets {
        top: side("top"),
        right: side("right"),
        bottom: side("bottom"),
        left: side("left"),
    };
    (pos, offsets)
}

fn resolve_z_index(node: &StyledNode) -> i32 {
    node.lookup("z-index")
        .and_then(|v| v.to_number())
        .map(|n| n as i32)
        .unwrap_or(0)
}

fn resolve_opacity(node: &StyledNode) -> f32 {
    node.lookup("opacity")
        .and_then(|v| v.to_number())
        .map(|n| n.clamp(0.0, 1.0))
        .unwrap_or(1.0)
}

fn resolve_shadow(node: &StyledNode) -> Option<Shadow> {
    let v = node.lookup("box-shadow")?;
    let (ox, oy, blur, spread, color) = v.to_shadow()?;
    Some(Shadow {
        ox,
        oy,
        blur,
        spread,
        color,
    })
}

/// Inherit the resolved `font-family` from the CSS keyword, walking the
/// comma-separated fallback list. Recognises `serif`, `monospace`,
/// `sans-serif`; anything else falls through to sans (we don't ship named
/// fonts like "Georgia").
pub fn resolve_family(node: &StyledNode) -> FontFamily {
    if let Some(v) = node.lookup("font-family") {
        for item in v.as_list() {
            if let Some(k) = item.to_keyword() {
                match k {
                    "serif" | "georgia" | "times" | "cambria" => return FontFamily::Serif,
                    "monospace" | "mono" | "courier" | "menlo" | "consolas" => {
                        return FontFamily::Monospace
                    }
                    "sans-serif" | "sans" | "arial" | "helvetica" => return FontFamily::Sans,
                    _ => {}
                }
            }
        }
    }
    FontFamily::Sans
}

fn is_italic(node: &StyledNode) -> bool {
    if let Some(k) = keyword(node, "font-style") {
        return k == "italic" || k == "oblique";
    }
    if let StyledNode::Element { tag, .. } = node {
        return matches!(tag.as_str(), "i" | "em" | "cite" | "var");
    }
    false
}

fn resolve_radii(node: &StyledNode) -> Radii {
    Radii {
        tl: px(node, "border-top-left-radius", 0.0),
        tr: px(node, "border-top-right-radius", 0.0),
        br: px(node, "border-bottom-right-radius", 0.0),
        bl: px(node, "border-bottom-left-radius", 0.0),
    }
}

/// Bundle of fonts used by the engine. `FontSet` holds both the fontdue
/// face (used for rasterization) and the rustybuzz face (used for shaping)
/// for every (family, weight, style) combination we support.
pub struct FontSet<'a> {
    pub sans: FontFace<'a>,
    pub sans_bold: FontFace<'a>,
    pub sans_italic: FontFace<'a>,
    pub serif: FontFace<'a>,
    pub mono: FontFace<'a>,
}

pub struct FontFace<'a> {
    pub fontdue: &'a fontdue::Font,
    pub buzz: rustybuzz::Face<'a>,
}

impl<'a> FontSet<'a> {
    /// Pick the face that best matches the requested family/weight/style.
    /// We fall back through sans regular for anything we don't bundle —
    /// bold italic, for example, degrades to plain italic.
    pub fn pick(&self, family: FontFamily, bold: bool, italic: bool) -> &FontFace<'a> {
        match family {
            FontFamily::Serif => &self.serif,
            FontFamily::Monospace => &self.mono,
            FontFamily::Sans => {
                if italic {
                    &self.sans_italic
                } else if bold {
                    &self.sans_bold
                } else {
                    &self.sans
                }
            }
        }
    }
}

pub struct LayoutEngine<'a> {
    pub viewport: Rect,
    pub fonts: &'a FontSet<'a>,
}

impl<'a> LayoutEngine<'a> {
    pub fn layout(&self, root: &StyledNode) -> LayoutBox {
        let mut lb = self.build_box(root);
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        lb.dimensions.content.w = self.viewport.w;
        self.layout_block(
            &mut lb,
            &Dimensions {
                content: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: self.viewport.w,
                    h: 0.0,
                },
                ..Default::default()
            },
        );
        // Post-process: resolve `position: relative` offsets (shift in place)
        // and `position: absolute/fixed` boxes (reparent to nearest positioned
        // ancestor or the viewport).
        self.resolve_positioning(&mut lb);
        lb
    }

    /// Walk the layout tree applying positioning adjustments. Relative boxes
    /// are translated by their top/left offsets. Absolute/fixed boxes are
    /// already laid out in their original document position as a flow box —
    /// we now detach them from their current parent's contribution and
    /// translate to the positioning container.
    fn resolve_positioning(&self, root: &mut LayoutBox) {
        // Collect positioning containers: root (viewport) acts as initial
        // containing block.
        let vp = self.viewport;
        Self::apply_positioning(root, vp, vp);
    }

    fn apply_positioning(lb: &mut LayoutBox, initial_cb: Rect, viewport: Rect) {
        // Determine containing block for this box's children:
        // - If this box is positioned (relative/absolute/fixed) it becomes
        //   the containing block for descendants using its padding box.
        let my_pb = lb.dimensions.padding_box();
        let child_cb = if lb.position != Position::Static {
            my_pb
        } else {
            initial_cb
        };

        // Recurse first so inner abs children are repositioned relative to
        // their nearest positioned ancestor.
        for c in &mut lb.children {
            Self::apply_positioning(c, child_cb, viewport);
        }

        // Apply our own position adjustments using `initial_cb` (the
        // containing block our parent passed down).
        match lb.position {
            Position::Static => {}
            Position::Relative => {
                let dx = lb.offsets.left.unwrap_or_else(|| {
                    lb.offsets.right.map(|r| -r).unwrap_or(0.0)
                });
                let dy = lb.offsets.top.unwrap_or_else(|| {
                    lb.offsets.bottom.map(|b| -b).unwrap_or(0.0)
                });
                if dx != 0.0 || dy != 0.0 {
                    translate_box(lb, dx, dy);
                }
            }
            Position::Absolute | Position::Fixed => {
                let cb = if lb.position == Position::Fixed {
                    viewport
                } else {
                    initial_cb
                };
                // Current margin-box position:
                let mb = lb.dimensions.margin_box();
                let target_x = if let Some(l) = lb.offsets.left {
                    cb.x + l
                } else if let Some(r) = lb.offsets.right {
                    cb.x + cb.w - r - mb.w
                } else {
                    mb.x
                };
                let target_y = if let Some(t) = lb.offsets.top {
                    cb.y + t
                } else if let Some(b) = lb.offsets.bottom {
                    cb.y + cb.h - b - mb.h
                } else {
                    mb.y
                };
                let dx = target_x - mb.x;
                let dy = target_y - mb.y;
                if dx != 0.0 || dy != 0.0 {
                    translate_box(lb, dx, dy);
                }
            }
        }
    }

    fn build_box(&self, node: &StyledNode) -> LayoutBox {
        let (borders, radii, bg, position, offsets, z, opacity, shadow) = match node {
            StyledNode::Element { .. } => {
                let bg = node
                    .lookup("background-color")
                    .and_then(|v| v.to_color())
                    .or_else(|| node.lookup("background").and_then(|v| v.to_color()));
                let (position, offsets) = resolve_position(node);
                (
                    resolve_borders(node),
                    resolve_radii(node),
                    bg,
                    position,
                    offsets,
                    resolve_z_index(node),
                    resolve_opacity(node),
                    resolve_shadow(node),
                )
            }
            _ => (
                Borders::default(),
                Radii::default(),
                None,
                Position::Static,
                Offsets::default(),
                0,
                1.0,
                None,
            ),
        };
        LayoutBox {
            dimensions: Dimensions::default(),
            box_type: BoxType::Block(node.clone()),
            children: Vec::new(),
            lines: Vec::new(),
            borders,
            radii,
            bg,
            position,
            offsets,
            z_index: z,
            opacity,
            shadow,
        }
    }

    /// Core block/flex layout.
    fn layout_block(&self, lb: &mut LayoutBox, containing: &Dimensions) {
        let node = match &lb.box_type {
            BoxType::Block(n) => n.clone(),
            BoxType::Anonymous => return,
        };

        let ml = px(&node, "margin-left", 0.0);
        let mr = px(&node, "margin-right", 0.0);
        let mt = px(&node, "margin-top", 0.0);
        let mb = px(&node, "margin-bottom", 0.0);
        let pl = px(&node, "padding-left", 0.0);
        let pr = px(&node, "padding-right", 0.0);
        let pt = px(&node, "padding-top", 0.0);
        let pb = px(&node, "padding-bottom", 0.0);

        let bl = lb.borders.left.width;
        let br_ = lb.borders.right.width;
        let bt = lb.borders.top.width;
        let bb = lb.borders.bottom.width;

        lb.dimensions.margin = EdgeSizes {
            left: ml,
            right: mr,
            top: mt,
            bottom: mb,
        };
        lb.dimensions.border = EdgeSizes {
            left: bl,
            right: br_,
            top: bt,
            bottom: bb,
        };
        lb.dimensions.padding = EdgeSizes {
            left: pl,
            right: pr,
            top: pt,
            bottom: pb,
        };

        // Width: fill container minus all horizontal edges (or explicit width).
        let width_override = node.lookup("width").map(|v| v.to_px());
        let content_w = width_override.unwrap_or(
            (containing.content.w - ml - mr - bl - br_ - pl - pr).max(0.0),
        );
        lb.dimensions.content.w = content_w;
        lb.dimensions.content.x = containing.content.x + ml + bl + pl;
        lb.dimensions.content.y = containing.content.y + mt + bt + pt;

        // Separate children into runs.
        let raw_children: Vec<StyledNode> = if let StyledNode::Element { children, .. } = &node {
            children
                .iter()
                .filter(|c| c.display() != Display::None)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        // If any sibling is block-level, drop pure-whitespace text siblings —
        // they're just source-code indentation, not meaningful content.
        let any_block = raw_children
            .iter()
            .any(|c| c.display() == Display::Block || c.display() == Display::Flex);
        let children_nodes: Vec<StyledNode> = if any_block {
            raw_children
                .into_iter()
                .filter(|c| {
                    !matches!(c, StyledNode::Text(t) if t.trim().is_empty())
                })
                .collect()
        } else {
            raw_children
        };

        // Decide layout mode:
        let mode = node.display();

        if mode == Display::Flex {
            self.layout_flex(lb, &node, &children_nodes);
            // height override
            let height_override = node.lookup("height").map(|v| v.to_px());
            if let Some(h) = height_override {
                lb.dimensions.content.h = h;
            }
            return;
        }

        // Partition: absolutely-positioned (including fixed) children don't
        // participate in the normal flow, but they still need to be laid out
        // so we can size them. We layout them separately and attach as
        // children — the positioning pass will place them.
        let (flow_children, out_of_flow): (Vec<StyledNode>, Vec<StyledNode>) = children_nodes
            .into_iter()
            .partition(|c| {
                !matches!(
                    keyword(c, "position").as_deref(),
                    Some("absolute") | Some("fixed")
                )
            });
        let children_nodes = flow_children;

        let all_inline = !children_nodes.is_empty()
            && children_nodes.iter().all(|c| c.display() == Display::Inline);

        let mut cursor_y = lb.dimensions.content.y;

        if all_inline {
            self.layout_inline(lb, &node, &children_nodes);
        } else {
            for child in &children_nodes {
                if child.display() == Display::None {
                    continue;
                }
                if matches!(child, StyledNode::Text(_)) {
                    let t = if let StyledNode::Text(s) = child {
                        s.clone()
                    } else {
                        String::new()
                    };
                    if t.trim().is_empty() {
                        continue;
                    }
                    // Wrap stray text in a synthetic inline-only block that inherits node's style.
                    let fs = px(&node, "font-size", 16.0);
                    let col = color(
                        &node,
                        "color",
                        Color {
                            r: 17,
                            g: 17,
                            b: 17,
                            a: 255,
                        },
                    );
                    let mut specified = std::collections::HashMap::new();
                    specified.insert(
                        "font-size".to_string(),
                        Value::Length(fs, crate::css::Unit::Px),
                    );
                    specified.insert("color".to_string(), Value::Color(col));
                    if let Some(ta) = keyword(&node, "text-align") {
                        specified.insert("text-align".to_string(), Value::Keyword(ta));
                    }
                    if let Some(lh) = node.lookup("line-height") {
                        specified.insert("line-height".to_string(), lh.clone());
                    }
                    if is_bold(&node) {
                        specified.insert(
                            "font-weight".to_string(),
                            Value::Keyword("bold".to_string()),
                        );
                    }
                    let wrap = StyledNode::Element {
                        tag: "anon".to_string(),
                        specified,
                        children: vec![StyledNode::Text(t)],
                    };
                    let mut cb = self.build_box(&wrap);
                    let cont = Dimensions {
                        content: Rect {
                            x: lb.dimensions.content.x,
                            y: cursor_y,
                            w: lb.dimensions.content.w,
                            h: 0.0,
                        },
                        ..Default::default()
                    };
                    self.layout_block(&mut cb, &cont);
                    cursor_y = cb.dimensions.margin_box().y + cb.dimensions.margin_box().h;
                    lb.children.push(cb);
                    continue;
                }
                let mut cb = self.build_box(child);
                let cont = Dimensions {
                    content: Rect {
                        x: lb.dimensions.content.x,
                        y: cursor_y,
                        w: lb.dimensions.content.w,
                        h: 0.0,
                    },
                    ..Default::default()
                };
                self.layout_block(&mut cb, &cont);
                cursor_y = cb.dimensions.margin_box().y + cb.dimensions.margin_box().h;
                lb.children.push(cb);
            }
        }

        // Content height: either override, or based on line boxes / child stacks.
        let height_override = node.lookup("height").map(|v| v.to_px());
        if let Some(h) = height_override {
            lb.dimensions.content.h = h;
        } else if !lb.lines.is_empty() {
            // computed inside layout_inline already
        } else if !all_inline {
            lb.dimensions.content.h = (cursor_y - lb.dimensions.content.y).max(0.0);
        }

        // Layout out-of-flow children (absolute/fixed). They sit at their
        // source-order "current" cursor position; the positioning pass will
        // later move them to their actual offset. Sizing uses the current
        // box as a starting containing block.
        for child in &out_of_flow {
            let mut cb = self.build_box(child);
            let cont = Dimensions {
                content: Rect {
                    x: lb.dimensions.content.x,
                    y: lb.dimensions.content.y,
                    w: lb.dimensions.content.w,
                    h: 0.0,
                },
                ..Default::default()
            };
            self.layout_block(&mut cb, &cont);
            lb.children.push(cb);
        }
    }

    /// Build line boxes from inline children.
    fn layout_inline(&self, lb: &mut LayoutBox, node: &StyledNode, children: &[StyledNode]) {
        let font_size = px(node, "font-size", 16.0);
        let text_color = color(
            node,
            "color",
            Color {
                r: 17,
                g: 17,
                b: 17,
                a: 255,
            },
        );
        let parent_family = resolve_family(node);
        let parent_italic = is_italic(node);
        // line-height: unitless multiplier OR length in px
        let line_height = match node.lookup("line-height") {
            Some(Value::Number(n)) => font_size * n,
            Some(v) => {
                let px = v.to_px();
                if px > 0.0 {
                    px
                } else {
                    font_size * 1.3
                }
            }
            None => font_size * 1.3,
        };
        let text_align = keyword(node, "text-align").unwrap_or_else(|| "left".to_string());
        let parent_bold = is_bold(node);

        // Flatten inline children into a token stream: Word { text, fs, col, bold }
        struct Tok {
            text: String,
            fs: f32,
            col: Color,
            bold: bool,
            italic: bool,
            family: FontFamily,
            // true if this token is just whitespace (a separator that may collapse at line breaks)
            ws: bool,
        }
        fn flatten(
            child: &StyledNode,
            parent_fs: f32,
            parent_col: Color,
            parent_bold: bool,
            parent_italic: bool,
            parent_family: FontFamily,
            out: &mut Vec<Tok>,
        ) {
            match child {
                StyledNode::Text(t) => {
                    // Per-CSS whitespace collapsing: runs of whitespace -> single space.
                    // Emit alternating word/whitespace tokens so we can collapse at line
                    // breaks. We preserve a leading/trailing whitespace separator token
                    // so that inter-element spaces (e.g. the " " between </strong> and
                    // the next text node) survive.
                    let mut buf = String::new();
                    let mut in_ws = false;
                    let starts_ws = t.chars().next().map(|c| c.is_whitespace()).unwrap_or(false);
                    if starts_ws {
                        out.push(Tok {
                            text: " ".to_string(),
                            fs: parent_fs,
                            col: parent_col,
                            bold: parent_bold,
                            italic: parent_italic,
                            family: parent_family,
                            ws: true,
                        });
                    }
                    for c in t.chars() {
                        if c.is_whitespace() {
                            if !in_ws {
                                if !buf.is_empty() {
                                    out.push(Tok {
                                        text: std::mem::take(&mut buf),
                                        fs: parent_fs,
                                        col: parent_col,
                                        bold: parent_bold,
                                        italic: parent_italic,
                                        family: parent_family,
                                        ws: false,
                                    });
                                }
                                in_ws = true;
                            }
                        } else {
                            if in_ws && !buf.is_empty() {
                                // already handled by flush above
                            }
                            if in_ws {
                                // mid-text whitespace run -> emit a space separator
                                // only if we've already emitted a word (so we don't
                                // duplicate the leading-ws token).
                                if !out
                                    .last()
                                    .map(|t| t.ws)
                                    .unwrap_or(false)
                                    && out
                                        .last()
                                        .map(|_| true)
                                        .unwrap_or(false)
                                {
                                    // previous token was a word — add separator
                                    let prev_was_word = out.last().map(|t| !t.ws).unwrap_or(false);
                                    if prev_was_word {
                                        out.push(Tok {
                                            text: " ".to_string(),
                                            fs: parent_fs,
                                            col: parent_col,
                                            bold: parent_bold,
                                            italic: parent_italic,
                                            family: parent_family,
                                            ws: true,
                                        });
                                    }
                                }
                            }
                            in_ws = false;
                            buf.push(c);
                        }
                    }
                    if !buf.is_empty() {
                        out.push(Tok {
                            text: buf,
                            fs: parent_fs,
                            col: parent_col,
                            bold: parent_bold,
                            italic: parent_italic,
                            family: parent_family,
                            ws: false,
                        });
                    }
                    let ends_ws = t.chars().last().map(|c| c.is_whitespace()).unwrap_or(false);
                    if ends_ws {
                        // append trailing space separator (may be collapsed later)
                        if !out.last().map(|t| t.ws).unwrap_or(false) {
                            out.push(Tok {
                                text: " ".to_string(),
                                fs: parent_fs,
                                col: parent_col,
                                bold: parent_bold,
                                italic: parent_italic,
                                family: parent_family,
                                ws: true,
                            });
                        }
                    }
                }
                StyledNode::Element { children, .. } => {
                    let fs = child
                        .lookup("font-size")
                        .map(|v| v.to_px())
                        .unwrap_or(parent_fs);
                    let col = child
                        .lookup("color")
                        .and_then(|v| v.to_color())
                        .unwrap_or(parent_col);
                    let bold = is_bold(child) || parent_bold;
                    let italic = is_italic(child) || parent_italic;
                    // Inherit family from child if set, else parent.
                    let family = if child.lookup("font-family").is_some() {
                        resolve_family(child)
                    } else {
                        parent_family
                    };
                    for gc in children {
                        flatten(gc, fs, col, bold, italic, family, out);
                    }
                }
            }
        }

        let mut toks: Vec<Tok> = Vec::new();
        for c in children {
            flatten(
                c,
                font_size,
                text_color,
                parent_bold,
                parent_italic,
                parent_family,
                &mut toks,
            );
        }

        // Line break: iterate tokens, measure, wrap when exceeding content width.
        let content_left = lb.dimensions.content.x;
        let content_right = content_left + lb.dimensions.content.w;
        let mut cursor_y = lb.dimensions.content.y;
        let mut cur = LineBox {
            y: cursor_y,
            height: line_height,
            baseline: cursor_y + font_size, // default
            items: Vec::new(),
        };
        let mut cur_x = content_left;
        let mut cur_max_fs: f32 = font_size;

        // Close the current line and start a new one.
        let mut lines: Vec<LineBox> = Vec::new();

        let flush_line =
            |lines: &mut Vec<LineBox>, cur: &mut LineBox, cur_max_fs: &mut f32, cursor_y: &mut f32| {
                // trim trailing whitespace items from this line
                while cur
                    .items
                    .last()
                    .map(|it| it.text.trim().is_empty())
                    .unwrap_or(false)
                {
                    cur.items.pop();
                }
                if !cur.items.is_empty() {
                    // Set height to max(line_height, max font-size * 1.2)
                    let h = line_height.max(*cur_max_fs * 1.3);
                    cur.height = h;
                    cur.baseline = cur.y + (*cur_max_fs).max(font_size); // place baseline just below font size
                    // adjust each item's y to baseline
                    let baseline = cur.baseline;
                    for it in &mut cur.items {
                        it.y = baseline;
                    }
                    lines.push(std::mem::take(cur));
                    *cursor_y += h;
                }
                cur.y = *cursor_y;
                cur.items.clear();
                *cur_max_fs = 0.0;
            };

        for tok in &toks {
            // Shape via rustybuzz — picks up kerning + ligatures for free.
            let face = self.fonts.pick(tok.family, tok.bold, tok.italic);
            let (glyphs, w) = text::shape(&face.buzz, &tok.text, tok.fs);
            let is_leading_ws = tok.ws && cur.items.is_empty();
            if is_leading_ws {
                continue;
            }
            if cur_x + w > content_right && !cur.items.is_empty() {
                // wrap
                flush_line(&mut lines, &mut cur, &mut cur_max_fs, &mut cursor_y);
                cur_x = content_left;
                if tok.ws {
                    continue;
                }
            }
            cur.items.push(InlineItem {
                text: tok.text.clone(),
                x: cur_x,
                y: cursor_y + tok.fs, // baseline placeholder, fixed in flush
                width: w,
                font_size: tok.fs,
                color: tok.col,
                bold: tok.bold,
                italic: tok.italic,
                family: tok.family,
                glyphs,
            });
            cur_x += w;
            if tok.fs > cur_max_fs {
                cur_max_fs = tok.fs;
            }
        }
        flush_line(&mut lines, &mut cur, &mut cur_max_fs, &mut cursor_y);

        // Apply text-align to each line.
        for line in &mut lines {
            if line.items.is_empty() {
                continue;
            }
            let first_x = line.items.first().unwrap().x;
            let last = line.items.last().unwrap();
            let line_w = (last.x + last.width) - first_x;
            let slack = lb.dimensions.content.w - line_w;
            let dx = match text_align.as_str() {
                "center" => (slack / 2.0).max(0.0),
                "right" => slack.max(0.0),
                _ => 0.0,
            };
            if dx > 0.0 {
                for it in &mut line.items {
                    it.x += dx;
                }
            }
        }

        lb.lines = lines;
        lb.dimensions.content.h = (cursor_y - lb.dimensions.content.y).max(0.0);
    }

    /// Flexbox-lite: single-line, horizontal or vertical, with justify-content,
    /// align-items, and gap. No wrap, no flex-grow/shrink (M2 scope).
    fn layout_flex(&self, lb: &mut LayoutBox, node: &StyledNode, children: &[StyledNode]) {
        let direction = keyword(node, "flex-direction").unwrap_or_else(|| "row".to_string());
        let row = direction != "column" && direction != "column-reverse";
        let justify = keyword(node, "justify-content").unwrap_or_else(|| "flex-start".to_string());
        let align = keyword(node, "align-items").unwrap_or_else(|| "stretch".to_string());
        let gap = px(node, "gap", 0.0);

        // First, lay out each child as a block in a 0-origin container, so we
        // know their intrinsic sizes. Then reposition along the main axis.
        let mut boxes: Vec<LayoutBox> = Vec::new();
        for child in children {
            if matches!(child, StyledNode::Text(_)) {
                continue;
            }
            let has_explicit_width = child.lookup("width").is_some();
            let mut cb = self.build_box(child);
            let cont = Dimensions {
                content: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: lb.dimensions.content.w,
                    h: 0.0,
                },
                ..Default::default()
            };
            self.layout_block(&mut cb, &cont);
            // Flex children without explicit width: shrink-to-fit.
            if !has_explicit_width {
                let intrinsic = intrinsic_content_width(&cb);
                if intrinsic > 0.0 && intrinsic < cb.dimensions.content.w {
                    // Re-layout at the shrunk width so wrapping and children are correct.
                    let new_w = intrinsic;
                    let mut cb2 = self.build_box(child);
                    let cont2 = Dimensions {
                        content: Rect {
                            x: 0.0,
                            y: 0.0,
                            w: new_w
                                + cb.dimensions.padding.left
                                + cb.dimensions.padding.right
                                + cb.dimensions.border.left
                                + cb.dimensions.border.right
                                + cb.dimensions.margin.left
                                + cb.dimensions.margin.right,
                            h: 0.0,
                        },
                        ..Default::default()
                    };
                    self.layout_block(&mut cb2, &cont2);
                    cb = cb2;
                }
            }
            boxes.push(cb);
        }

        let n = boxes.len();
        if n == 0 {
            lb.dimensions.content.h = 0.0;
            return;
        }

        let total_gap = gap * (n as f32 - 1.0).max(0.0);
        let container_w = lb.dimensions.content.w;
        let (main_total, cross_max) = if row {
            let tot: f32 = boxes.iter().map(|b| b.dimensions.margin_box().w).sum();
            let cross: f32 = boxes
                .iter()
                .map(|b| b.dimensions.margin_box().h)
                .fold(0.0_f32, f32::max);
            (tot + total_gap, cross)
        } else {
            let tot: f32 = boxes.iter().map(|b| b.dimensions.margin_box().h).sum();
            let cross: f32 = boxes
                .iter()
                .map(|b| b.dimensions.margin_box().w)
                .fold(0.0_f32, f32::max);
            (tot + total_gap, cross)
        };

        // Container cross size: explicit height for row, width for column; else content-max.
        let container_h = node
            .lookup("height")
            .map(|v| v.to_px())
            .unwrap_or_else(|| if row { cross_max } else { main_total });
        if row {
            lb.dimensions.content.h = container_h;
        } else {
            lb.dimensions.content.h = container_h;
        }

        // Compute starting offset along main axis for justify-content.
        let available = if row {
            container_w - main_total
        } else {
            container_h - main_total
        };
        let (start_offset, between_extra) = match justify.as_str() {
            "flex-end" | "end" => (available.max(0.0), 0.0),
            "center" => ((available / 2.0).max(0.0), 0.0),
            "space-between" if n > 1 => (0.0, available.max(0.0) / (n as f32 - 1.0)),
            "space-around" if n > 0 => {
                let each = available.max(0.0) / n as f32;
                (each / 2.0, each)
            }
            "space-evenly" if n > 0 => {
                let each = available.max(0.0) / (n as f32 + 1.0);
                (each, each)
            }
            _ => (0.0, 0.0),
        };

        // Place each child.
        let origin_x = lb.dimensions.content.x;
        let origin_y = lb.dimensions.content.y;
        let mut main_cursor = if row { origin_x } else { origin_y } + start_offset;

        for mut cb in boxes.into_iter() {
            let mb = cb.dimensions.margin_box();
            let (main_size, cross_size) = if row { (mb.w, mb.h) } else { (mb.h, mb.w) };

            // Cross-axis alignment:
            let cross_start = if row { origin_y } else { origin_x };
            let cross_extent = if row { container_h } else { container_w };
            let cross_offset = match align.as_str() {
                "flex-end" | "end" => (cross_extent - cross_size).max(0.0),
                "center" => ((cross_extent - cross_size) / 2.0).max(0.0),
                "stretch" => 0.0, // size stretching not implemented for M2; top-align
                _ => 0.0,
            };

            // The child was laid out at origin (0,0) with its own margin. We
            // translate it so its margin-box starts at the desired spot.
            let desired_main = main_cursor;
            let desired_cross = cross_start + cross_offset;
            let (dx, dy) = if row {
                (desired_main - mb.x, desired_cross - mb.y)
            } else {
                (desired_cross - mb.x, desired_main - mb.y)
            };
            translate_box(&mut cb, dx, dy);

            main_cursor += main_size + gap + between_extra;
            lb.children.push(cb);
        }
    }
}

/// Recursively translate a laid-out box and everything inside it.
fn translate_box(lb: &mut LayoutBox, dx: f32, dy: f32) {
    lb.dimensions.content.x += dx;
    lb.dimensions.content.y += dy;
    for line in &mut lb.lines {
        line.y += dy;
        line.baseline += dy;
        for it in &mut line.items {
            it.x += dx;
            it.y += dy;
        }
    }
    for c in &mut lb.children {
        translate_box(c, dx, dy);
    }
}

/// Maximum content width used by this laid-out box, measured from the
/// actual inline lines and child margin-boxes. Used by flex to shrink
/// children without an explicit `width`.
fn intrinsic_content_width(lb: &LayoutBox) -> f32 {
    let mut max_w: f32 = 0.0;
    let origin = lb.dimensions.content.x;
    for line in &lb.lines {
        let w = line
            .items
            .iter()
            .map(|it| (it.x + it.width) - origin)
            .fold(0.0_f32, f32::max);
        if w > max_w {
            max_w = w;
        }
    }
    for c in &lb.children {
        // child's margin-box width relative to this content
        let mb = c.dimensions.margin_box();
        let w = (mb.x + mb.w) - origin;
        if w > max_w {
            max_w = w;
        }
        // also consider the child's own intrinsic width (in case its
        // explicit content width is wider than where it currently sits)
        let child_intrinsic = intrinsic_content_width(c);
        if child_intrinsic > max_w {
            max_w = child_intrinsic;
        }
    }
    max_w
}

pub fn measure_text(font: &fontdue::Font, text: &str, size: f32) -> f32 {
    let mut w = 0.0;
    for c in text.chars() {
        let (m, _) = font.rasterize(c, size);
        w += m.advance_width;
    }
    w
}
