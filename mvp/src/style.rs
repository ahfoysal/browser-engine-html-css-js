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
            map.insert(d.name.clone(), d.value.clone());
        }
    }
    // inline style="..." attribute
    if let Some(inline) = el.attrs.get("style") {
        let wrapped = format!("x {{ {} }}", inline);
        let ss = crate::css::parse(&wrapped);
        if let Some(rule) = ss.rules.first() {
            for d in &rule.declarations {
                map.insert(d.name.clone(), d.value.clone());
            }
        }
    }
    map
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
