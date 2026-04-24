//! Mutable DOM arena shared between Rust and JS.
//!
//! Unlike `html::Node` (which is an owned recursive enum built once during
//! parse), the JS bindings need to mutate the tree during script execution:
//! change text, flip inline styles, add event listeners, etc. So we flatten
//! the tree into an arena keyed by `NodeId` and have parent/child pointers
//! live in the arena. The original `html::Node` is still what layout/paint
//! consume — we just re-serialize the arena back to `html::Node` after JS
//! has mutated it.

use std::collections::HashMap;

use crate::html::{Element as HtmlElement, Node as HtmlNode};

pub type NodeId = usize;

#[derive(Debug, Clone)]
pub enum DomKind {
    Element {
        tag: String,
        attrs: HashMap<String, String>,
        children: Vec<NodeId>,
    },
    Text(String),
}

#[derive(Debug, Clone)]
pub struct DomNode {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub kind: DomKind,
}

#[derive(Debug, Default)]
pub struct Dom {
    pub nodes: Vec<DomNode>,
    pub root: NodeId,
    /// Script source blocks in document order.
    pub scripts: Vec<String>,
}

impl Dom {
    pub fn from_html(root: &HtmlNode) -> Self {
        let mut dom = Dom {
            nodes: Vec::new(),
            root: 0,
            scripts: Vec::new(),
        };
        let rid = dom.build(root, None);
        dom.root = rid;
        dom
    }

    fn alloc(&mut self, parent: Option<NodeId>, kind: DomKind) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(DomNode { id, parent, kind });
        id
    }

    fn build(&mut self, node: &HtmlNode, parent: Option<NodeId>) -> NodeId {
        match node {
            HtmlNode::Text(s) => self.alloc(parent, DomKind::Text(s.clone())),
            HtmlNode::Element(e) => {
                // <script> contents: capture source, leave the element in the
                // tree with no children so it doesn't render or confuse layout.
                if e.tag == "script" {
                    // The html parser already stores script inner text as a
                    // single Text child when the tag is <style>; for <script>
                    // it throws it away. We don't have access to that raw
                    // text here, so we rely on the parser also storing it —
                    // see `js::extract_scripts` which patches parse.
                    if let Some(src) = e.attrs.get("__script_src").cloned() {
                        self.scripts.push(src);
                    }
                    let id = self.alloc(
                        parent,
                        DomKind::Element {
                            tag: e.tag.clone(),
                            attrs: e.attrs.clone(),
                            children: Vec::new(),
                        },
                    );
                    return id;
                }
                let id = self.alloc(
                    parent,
                    DomKind::Element {
                        tag: e.tag.clone(),
                        attrs: e.attrs.clone(),
                        children: Vec::new(),
                    },
                );
                let child_ids: Vec<NodeId> = e
                    .children
                    .iter()
                    .map(|c| self.build(c, Some(id)))
                    .collect();
                if let DomKind::Element { children, .. } = &mut self.nodes[id].kind {
                    *children = child_ids;
                }
                id
            }
        }
    }

    pub fn get_by_id(&self, target: &str) -> Option<NodeId> {
        for n in &self.nodes {
            if let DomKind::Element { attrs, .. } = &n.kind {
                if attrs.get("id").map(|s| s.as_str()) == Some(target) {
                    return Some(n.id);
                }
            }
        }
        None
    }

    /// Tiny selector engine: supports `tag`, `#id`, `.class`, and
    /// compound `tag.class`/`tag#id`. No combinators.
    pub fn query_selector(&self, sel: &str) -> Option<NodeId> {
        let (tag, id, classes) = parse_simple_selector(sel);
        for n in &self.nodes {
            if let DomKind::Element { tag: t, attrs, .. } = &n.kind {
                if let Some(want) = &tag {
                    if want != t {
                        continue;
                    }
                }
                if let Some(want) = &id {
                    if attrs.get("id").map(|s| s.as_str()) != Some(want.as_str()) {
                        continue;
                    }
                }
                if !classes.is_empty() {
                    let have: Vec<&str> = attrs
                        .get("class")
                        .map(|s| s.split_whitespace().collect())
                        .unwrap_or_default();
                    if !classes.iter().all(|c| have.contains(&c.as_str())) {
                        continue;
                    }
                }
                return Some(n.id);
            }
        }
        None
    }

    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out);
        out
    }

    fn collect_text(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].kind {
            DomKind::Text(s) => out.push_str(s),
            DomKind::Element { children, .. } => {
                for c in children.clone() {
                    self.collect_text(c, out);
                }
            }
        }
    }

    /// Replace an element's children with a single text node.
    pub fn set_text_content(&mut self, id: NodeId, text: &str) {
        match &self.nodes[id].kind {
            DomKind::Text(_) => {
                self.nodes[id].kind = DomKind::Text(text.to_string());
            }
            DomKind::Element { .. } => {
                let new_text_id = self.alloc(Some(id), DomKind::Text(text.to_string()));
                if let DomKind::Element { children, .. } = &mut self.nodes[id].kind {
                    *children = vec![new_text_id];
                }
            }
        }
    }

    pub fn get_attr(&self, id: NodeId, name: &str) -> Option<String> {
        if let DomKind::Element { attrs, .. } = &self.nodes[id].kind {
            attrs.get(name).cloned()
        } else {
            None
        }
    }

    pub fn set_attr(&mut self, id: NodeId, name: &str, value: &str) {
        if let DomKind::Element { attrs, .. } = &mut self.nodes[id].kind {
            attrs.insert(name.to_string(), value.to_string());
        }
    }

    /// Merge `prop: value` into the element's inline `style=""` attribute,
    /// replacing any existing same-property declaration.
    pub fn set_style(&mut self, id: NodeId, prop: &str, value: &str) {
        let prop_css = camel_to_kebab(prop);
        let current = self.get_attr(id, "style").unwrap_or_default();
        let mut rebuilt = String::new();
        let mut replaced = false;
        for decl in current.split(';') {
            let decl = decl.trim();
            if decl.is_empty() {
                continue;
            }
            if let Some((k, _v)) = decl.split_once(':') {
                if k.trim().eq_ignore_ascii_case(&prop_css) {
                    if !replaced {
                        rebuilt.push_str(&format!("{}: {};", prop_css, value));
                        replaced = true;
                    }
                    continue;
                }
            }
            rebuilt.push_str(decl);
            if !decl.ends_with(';') {
                rebuilt.push(';');
            }
        }
        if !replaced {
            rebuilt.push_str(&format!("{}: {};", prop_css, value));
        }
        self.set_attr(id, "style", &rebuilt);
    }

    /// Serialize the arena back to an `html::Node` tree so the existing
    /// style/layout/paint pipeline can consume it.
    pub fn to_html(&self) -> HtmlNode {
        self.emit(self.root)
    }

    fn emit(&self, id: NodeId) -> HtmlNode {
        match &self.nodes[id].kind {
            DomKind::Text(s) => HtmlNode::Text(s.clone()),
            DomKind::Element { tag, attrs, children } => HtmlNode::Element(HtmlElement {
                tag: tag.clone(),
                attrs: attrs.clone(),
                children: children.iter().map(|c| self.emit(*c)).collect(),
            }),
        }
    }
}

fn parse_simple_selector(sel: &str) -> (Option<String>, Option<String>, Vec<String>) {
    let mut tag: Option<String> = None;
    let mut id: Option<String> = None;
    let mut classes: Vec<String> = Vec::new();
    let mut i = 0;
    let bytes = sel.as_bytes();
    // leading tag name (if any)
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_') {
        i += 1;
    }
    if i > 0 {
        tag = Some(sel[..i].to_lowercase());
    }
    while i < bytes.len() {
        let marker = bytes[i];
        i += 1;
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_') {
            i += 1;
        }
        let name = &sel[start..i];
        match marker {
            b'#' => id = Some(name.to_string()),
            b'.' => classes.push(name.to_string()),
            _ => {}
        }
    }
    (tag, id, classes)
}

fn camel_to_kebab(s: &str) -> String {
    // backgroundColor -> background-color; already-kebab passes through.
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
