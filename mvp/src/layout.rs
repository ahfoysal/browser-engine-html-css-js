//! Block-flow layout. Each block has margin/padding, stacks vertically.
//! Inline content within a block is laid out as lines (simple wrapping).

use crate::style::{Display, StyledNode};

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

#[derive(Debug, Clone, Default)]
pub struct Dimensions {
    pub content: Rect,
    pub padding: EdgeSizes,
    pub margin: EdgeSizes,
}

impl Dimensions {
    pub fn padding_box(&self) -> Rect {
        expand(self.content, self.padding)
    }
    pub fn margin_box(&self) -> Rect {
        expand(self.padding_box(), self.margin)
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

#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub dimensions: Dimensions,
    pub box_type: BoxType,
    pub children: Vec<LayoutBox>,
    /// Inline lines: for a block that has inline children, we build line boxes here.
    pub lines: Vec<LineBox>,
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
    pub items: Vec<InlineItem>,
}

#[derive(Debug, Clone)]
pub struct InlineItem {
    pub text: String,
    pub x: f32,
    pub y: f32,        // baseline Y
    pub width: f32,
    pub font_size: f32,
    pub color: crate::css::Color,
}

fn px(node: &StyledNode, key: &str, default: f32) -> f32 {
    node.lookup(key).map(|v| v.to_px()).unwrap_or(default)
}

fn color(node: &StyledNode, key: &str, default: crate::css::Color) -> crate::css::Color {
    node.lookup(key)
        .and_then(|v| v.to_color())
        .unwrap_or(default)
}

pub struct LayoutEngine<'a> {
    pub viewport: Rect,
    pub font: &'a fontdue::Font,
}

impl<'a> LayoutEngine<'a> {
    pub fn layout(&self, root: &StyledNode) -> LayoutBox {
        let mut lb = self.build_box(root);
        // Set root box content width to viewport
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        lb.dimensions.content.w = self.viewport.w;
        self.layout_block(&mut lb, &Dimensions {
            content: Rect { x: 0.0, y: 0.0, w: self.viewport.w, h: 0.0 },
            ..Default::default()
        });
        lb
    }

    fn build_box(&self, node: &StyledNode) -> LayoutBox {
        LayoutBox {
            dimensions: Dimensions::default(),
            box_type: BoxType::Block(node.clone()),
            children: Vec::new(),
            lines: Vec::new(),
        }
    }

    fn layout_block(&self, lb: &mut LayoutBox, containing: &Dimensions) {
        // Compute margins/padding & width from node.
        let node = match &lb.box_type {
            BoxType::Block(n) => n.clone(),
            BoxType::Anonymous => return,
        };

        let ml = px(&node, "margin-left", px(&node, "margin", 0.0));
        let mr = px(&node, "margin-right", px(&node, "margin", 0.0));
        let mt = px(&node, "margin-top", px(&node, "margin", 0.0));
        let mb = px(&node, "margin-bottom", px(&node, "margin", 0.0));
        let pl = px(&node, "padding-left", px(&node, "padding", 0.0));
        let pr = px(&node, "padding-right", px(&node, "padding", 0.0));
        let pt = px(&node, "padding-top", px(&node, "padding", 0.0));
        let pb = px(&node, "padding-bottom", px(&node, "padding", 0.0));

        lb.dimensions.margin = EdgeSizes { left: ml, right: mr, top: mt, bottom: mb };
        lb.dimensions.padding = EdgeSizes { left: pl, right: pr, top: pt, bottom: pb };

        // Width: fill container minus margins & padding.
        let width_override = node.lookup("width").map(|v| v.to_px());
        let content_w = width_override.unwrap_or(
            (containing.content.w - ml - mr - pl - pr).max(0.0),
        );
        lb.dimensions.content.w = content_w;
        lb.dimensions.content.x = containing.content.x + ml + pl;
        lb.dimensions.content.y = containing.content.y + mt + pt;

        // Separate children into runs: if a block contains all inline children,
        // treat them as lines within this block. Otherwise each child is a block.
        let children_nodes: Vec<StyledNode> = if let StyledNode::Element { children, .. } = &node {
            children.iter().filter(|c| !matches!(c, StyledNode::Element { .. } if c.display() == Display::None)).cloned().collect()
        } else {
            Vec::new()
        };

        let all_inline = !children_nodes.is_empty()
            && children_nodes.iter().all(|c| c.display() != Display::Block);

        let mut cursor_y = lb.dimensions.content.y;

        if all_inline {
            // Build line boxes.
            let font_size = px(&node, "font-size", 16.0);
            let text_color = color(&node, "color", crate::css::Color { r: 17, g: 17, b: 17, a: 255 });
            let line_height = font_size * 1.3;
            let max_x = lb.dimensions.content.x + lb.dimensions.content.w;
            let mut x = lb.dimensions.content.x;
            let mut current = LineBox {
                y: cursor_y,
                height: line_height,
                items: Vec::new(),
            };

            let mut push_word = |current: &mut LineBox,
                                  cursor_y: &mut f32,
                                  x: &mut f32,
                                  lb: &mut LayoutBox,
                                  word: &str,
                                  fs: f32,
                                  col: crate::css::Color| {
                if word.is_empty() {
                    return;
                }
                let w = measure_text(self.font, word, fs);
                if *x + w > max_x && !current.items.is_empty() {
                    // wrap
                    lb.lines.push(std::mem::take(current));
                    *cursor_y += line_height;
                    current.y = *cursor_y;
                    current.height = line_height;
                    *x = lb.dimensions.content.x;
                }
                current.items.push(InlineItem {
                    text: word.to_string(),
                    x: *x,
                    y: *cursor_y + fs, // baseline approximation
                    width: w,
                    font_size: fs,
                    color: col,
                });
                *x += w;
            };

            for child in &children_nodes {
                match child {
                    StyledNode::Text(t) => {
                        let mut first = true;
                        for word in t.split_whitespace() {
                            let w = if first { word.to_string() } else { format!(" {}", word) };
                            first = false;
                            push_word(&mut current, &mut cursor_y, &mut x, lb, &w, font_size, text_color);
                        }
                    }
                    StyledNode::Element { .. } => {
                        // inline element: pull text, use own color/font-size
                        let fs = child.lookup("font-size").map(|v| v.to_px()).unwrap_or(font_size);
                        let col = child.lookup("color").and_then(|v| v.to_color()).unwrap_or(text_color);
                        let text = collect_text(child);
                        let mut first = true;
                        for word in text.split_whitespace() {
                            let w = if first { word.to_string() } else { format!(" {}", word) };
                            first = false;
                            push_word(&mut current, &mut cursor_y, &mut x, lb, &w, fs, col);
                        }
                    }
                }
            }
            if !current.items.is_empty() {
                lb.lines.push(current);
                cursor_y += line_height;
            }

            lb.dimensions.content.h = (cursor_y - lb.dimensions.content.y).max(0.0);
        } else {
            // Block children
            for child in &children_nodes {
                if child.display() == Display::None {
                    continue;
                }
                if matches!(child, StyledNode::Text(_)) {
                    // anonymous block for stray text
                    let t = if let StyledNode::Text(s) = child { s.clone() } else { String::new() };
                    if t.trim().is_empty() { continue; }
                    // Wrap text in a synthetic styled element with node's font-size/color
                    let fs = px(&node, "font-size", 16.0);
                    let col = color(&node, "color", crate::css::Color { r: 17, g: 17, b: 17, a: 255 });
                    let mut specified = std::collections::HashMap::new();
                    specified.insert("font-size".to_string(), crate::css::Value::Length(fs, crate::css::Unit::Px));
                    specified.insert("color".to_string(), crate::css::Value::Color(col));
                    let wrap = StyledNode::Element {
                        tag: "anon".to_string(),
                        specified,
                        children: vec![StyledNode::Text(t)],
                    };
                    let mut cb = self.build_box(&wrap);
                    let cont = Dimensions {
                        content: Rect { x: lb.dimensions.content.x, y: cursor_y, w: lb.dimensions.content.w, h: 0.0 },
                        ..Default::default()
                    };
                    self.layout_block(&mut cb, &cont);
                    cursor_y = cb.dimensions.margin_box().y + cb.dimensions.margin_box().h;
                    lb.children.push(cb);
                    continue;
                }
                let mut cb = self.build_box(child);
                let cont = Dimensions {
                    content: Rect { x: lb.dimensions.content.x, y: cursor_y, w: lb.dimensions.content.w, h: 0.0 },
                    ..Default::default()
                };
                self.layout_block(&mut cb, &cont);
                cursor_y = cb.dimensions.margin_box().y + cb.dimensions.margin_box().h;
                lb.children.push(cb);
            }

            let height_override = node.lookup("height").map(|v| v.to_px());
            lb.dimensions.content.h = height_override.unwrap_or(
                (cursor_y - lb.dimensions.content.y).max(0.0),
            );
        }
    }
}

fn collect_text(n: &StyledNode) -> String {
    let mut s = String::new();
    match n {
        StyledNode::Text(t) => s.push_str(t),
        StyledNode::Element { children, .. } => {
            for c in children {
                s.push_str(&collect_text(c));
            }
        }
    }
    s
}

pub fn measure_text(font: &fontdue::Font, text: &str, size: f32) -> f32 {
    let mut w = 0.0;
    for c in text.chars() {
        let (m, _) = font.rasterize(c, size);
        w += m.advance_width;
    }
    w
}
