//! Selector matching: build a styled tree from DOM + stylesheet.

use std::collections::HashMap;

use crate::css::{Rule, Selector, Stylesheet, Value};
use crate::html::{Element, Node};

pub type PropertyMap = HashMap<String, Value>;

#[derive(Debug, Clone)]
pub enum StyledNode {
    Element {
        tag: String,
        specified: PropertyMap,
        children: Vec<StyledNode>,
    },
    Text(String),
}

impl StyledNode {
    pub fn display(&self) -> Display {
        match self {
            StyledNode::Text(_) => Display::Inline,
            StyledNode::Element { specified, tag, .. } => {
                if let Some(v) = specified.get("display") {
                    if let Some(k) = v.to_keyword() {
                        return match k {
                            "none" => Display::None,
                            "inline" => Display::Inline,
                            "flex" => Display::Flex,
                            _ => Display::Block,
                        };
                    }
                }
                // default per common tags
                match tag.as_str() {
                    "span" | "a" | "b" | "i" | "em" | "strong" | "code" => Display::Inline,
                    _ => Display::Block,
                }
            }
        }
    }

    pub fn lookup(&self, key: &str) -> Option<&Value> {
        if let StyledNode::Element { specified, .. } = self {
            specified.get(key)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Display {
    Block,
    Inline,
    Flex,
    None,
}

fn selector_matches(sel: &Selector, el: &Element) -> bool {
    if let Some(t) = &sel.tag {
        if t != "*" && t != &el.tag {
            return false;
        }
    }
    if let Some(id) = &sel.id {
        if el.id() != Some(id.as_str()) {
            return false;
        }
    }
    let el_classes = el.classes();
    for c in &sel.classes {
        if !el_classes.contains(&c.as_str()) {
            return false;
        }
    }
    true
}

fn matching_rules<'a>(
    el: &Element,
    stylesheet: &'a Stylesheet,
) -> Vec<((usize, usize, usize), &'a Rule)> {
    let mut out = Vec::new();
    for rule in &stylesheet.rules {
        for sel in &rule.selectors {
            if selector_matches(sel, el) {
                out.push((sel.specificity(), rule));
                break;
            }
        }
    }
    out.sort_by_key(|(s, _)| *s);
    out
}

fn specified_values(el: &Element, stylesheet: &Stylesheet) -> PropertyMap {
    let mut map = PropertyMap::new();
    for (_, rule) in matching_rules(el, stylesheet) {
        for d in &rule.declarations {
            insert_expanded(&mut map, &d.name, &d.value);
        }
    }
    // inline style="..." attribute
    if let Some(inline) = el.attrs.get("style") {
        let wrapped = format!("x {{ {} }}", inline);
        let ss = crate::css::parse(&wrapped);
        if let Some(rule) = ss.rules.first() {
            for d in &rule.declarations {
                insert_expanded(&mut map, &d.name, &d.value);
            }
        }
    }
    map
}

/// Expand common shorthand properties into longhand entries. Also stores the
/// shorthand itself so callers can fall back to it.
fn insert_expanded(map: &mut PropertyMap, name: &str, value: &Value) {
    map.insert(name.to_string(), value.clone());
    match name {
        "margin" | "padding" => {
            let parts = value.as_list();
            let (t, r, b, l) = four_sides(&parts);
            map.insert(format!("{}-top", name), t);
            map.insert(format!("{}-right", name), r);
            map.insert(format!("{}-bottom", name), b);
            map.insert(format!("{}-left", name), l);
        }
        "border" => {
            // border: <width> <style> <color>  (any order, any missing)
            let parts = value.as_list();
            let mut width: Option<Value> = None;
            let mut style: Option<Value> = None;
            let mut color: Option<Value> = None;
            for p in &parts {
                match p {
                    Value::Length(_, _) | Value::Number(_) => width = Some(p.clone()),
                    Value::Color(_) => color = Some(p.clone()),
                    Value::Keyword(k) => {
                        if is_border_style(k) {
                            style = Some(p.clone());
                        } else if width.is_none() && (k == "thin" || k == "medium" || k == "thick")
                        {
                            // map to px
                            let px = match k.as_str() {
                                "thin" => 1.0,
                                "medium" => 3.0,
                                "thick" => 5.0,
                                _ => 1.0,
                            };
                            width = Some(Value::Length(px, crate::css::Unit::Px));
                        } else if color.is_none() {
                            // might be a named color that fell through — unlikely since css parses it
                            color = Some(p.clone());
                        }
                    }
                    _ => {}
                }
            }
            let width = width.unwrap_or(Value::Length(0.0, crate::css::Unit::Px));
            let style = style.unwrap_or(Value::Keyword("solid".to_string()));
            let color = color.unwrap_or(Value::Color(crate::css::Color {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            }));
            for side in &["top", "right", "bottom", "left"] {
                map.insert(format!("border-{}-width", side), width.clone());
                map.insert(format!("border-{}-style", side), style.clone());
                map.insert(format!("border-{}-color", side), color.clone());
            }
        }
        "border-width" => {
            let parts = value.as_list();
            let (t, r, b, l) = four_sides(&parts);
            map.insert("border-top-width".to_string(), t);
            map.insert("border-right-width".to_string(), r);
            map.insert("border-bottom-width".to_string(), b);
            map.insert("border-left-width".to_string(), l);
        }
        "border-color" => {
            let parts = value.as_list();
            let (t, r, b, l) = four_sides(&parts);
            map.insert("border-top-color".to_string(), t);
            map.insert("border-right-color".to_string(), r);
            map.insert("border-bottom-color".to_string(), b);
            map.insert("border-left-color".to_string(), l);
        }
        "border-style" => {
            let parts = value.as_list();
            let (t, r, b, l) = four_sides(&parts);
            map.insert("border-top-style".to_string(), t);
            map.insert("border-right-style".to_string(), r);
            map.insert("border-bottom-style".to_string(), b);
            map.insert("border-left-style".to_string(), l);
        }
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            // Same logic as `border` but for one side only.
            let parts = value.as_list();
            let mut width: Option<Value> = None;
            let mut style: Option<Value> = None;
            let mut color: Option<Value> = None;
            for p in &parts {
                match p {
                    Value::Length(_, _) | Value::Number(_) => width = Some(p.clone()),
                    Value::Color(_) => color = Some(p.clone()),
                    Value::Keyword(k) if is_border_style(k) => style = Some(p.clone()),
                    _ => {}
                }
            }
            let side = name.trim_start_matches("border-");
            if let Some(w) = width {
                map.insert(format!("border-{}-width", side), w);
            }
            if let Some(s) = style {
                map.insert(format!("border-{}-style", side), s);
            }
            if let Some(c) = color {
                map.insert(format!("border-{}-color", side), c);
            }
        }
        "border-radius" => {
            // 1-4 values
            let parts = value.as_list();
            let (tl, tr, br, bl) = four_sides(&parts);
            // CSS ordering for border-radius is tl tr br bl
            map.insert("border-top-left-radius".to_string(), tl);
            map.insert("border-top-right-radius".to_string(), tr);
            map.insert("border-bottom-right-radius".to_string(), br);
            map.insert("border-bottom-left-radius".to_string(), bl);
        }
        _ => {}
    }
}

fn is_border_style(k: &str) -> bool {
    matches!(
        k,
        "none" | "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset"
            | "outset" | "hidden"
    )
}

/// Expand 1/2/3/4 value shorthand into (top, right, bottom, left) per CSS spec.
fn four_sides(parts: &[Value]) -> (Value, Value, Value, Value) {
    match parts.len() {
        0 => {
            let z = Value::Length(0.0, crate::css::Unit::Px);
            (z.clone(), z.clone(), z.clone(), z)
        }
        1 => (parts[0].clone(), parts[0].clone(), parts[0].clone(), parts[0].clone()),
        2 => (
            parts[0].clone(),
            parts[1].clone(),
            parts[0].clone(),
            parts[1].clone(),
        ),
        3 => (
            parts[0].clone(),
            parts[1].clone(),
            parts[2].clone(),
            parts[1].clone(),
        ),
        _ => (
            parts[0].clone(),
            parts[1].clone(),
            parts[2].clone(),
            parts[3].clone(),
        ),
    }
}

pub fn style_tree(root: &Node, stylesheet: &Stylesheet) -> StyledNode {
    match root {
        Node::Text(t) => StyledNode::Text(t.clone()),
        Node::Element(e) => {
            // skip <style>, <script>, <head>
            if e.tag == "style" || e.tag == "script" || e.tag == "head" {
                return StyledNode::Element {
                    tag: e.tag.clone(),
                    specified: {
                        let mut m = PropertyMap::new();
                        m.insert("display".to_string(), Value::Keyword("none".to_string()));
                        m
                    },
                    children: Vec::new(),
                };
            }
            StyledNode::Element {
                tag: e.tag.clone(),
                specified: specified_values(e, stylesheet),
                children: e
                    .children
                    .iter()
                    .map(|c| style_tree(c, stylesheet))
                    .collect(),
            }
        }
    }
}

/// Built-in user-agent stylesheet that gives basic defaults to common tags.
pub fn user_agent_css() -> &'static str {
    r#"
    body { display: block; margin: 8px; color: #111111; font-size: 16px; }
    h1 { display: block; font-size: 32px; margin: 16px; color: #111111; }
    h2 { display: block; font-size: 24px; margin: 14px; color: #111111; }
    h3 { display: block; font-size: 20px; margin: 12px; color: #111111; }
    p  { display: block; font-size: 16px; margin: 8px; color: #222222; }
    div { display: block; }
    ul { display: block; margin: 8px; }
    li { display: block; font-size: 16px; margin: 4px; color: #222222; }
    a { color: #1a0dab; }
    span { color: #111111; font-size: 16px; }
    "#
}
