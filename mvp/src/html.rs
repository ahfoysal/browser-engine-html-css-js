//! Minimal HTML parser: tags, attributes (id, class), text, and <style> blocks.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Node {
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone)]
pub struct Element {
    pub tag: String,
    pub attrs: HashMap<String, String>,
    pub children: Vec<Node>,
}

impl Element {
    pub fn id(&self) -> Option<&str> {
        self.attrs.get("id").map(|s| s.as_str())
    }
    pub fn classes(&self) -> Vec<&str> {
        self.attrs
            .get("class")
            .map(|s| s.split_whitespace().collect())
            .unwrap_or_default()
    }
}

pub struct Parser {
    pos: usize,
    input: Vec<char>,
}

// Void elements: no end tag, no children.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
    "source", "track", "wbr",
];

impl Parser {
    pub fn new(input: &str) -> Self {
        Self {
            pos: 0,
            input: input.chars().collect(),
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek(&self) -> char {
        self.input[self.pos]
    }

    fn starts_with(&self, s: &str) -> bool {
        self.input[self.pos..]
            .iter()
            .collect::<String>()
            .starts_with(s)
    }

    fn consume_char(&mut self) -> char {
        let c = self.input[self.pos];
        self.pos += 1;
        c
    }

    fn consume_while<F: Fn(char) -> bool>(&mut self, test: F) -> String {
        let mut s = String::new();
        while !self.eof() && test(self.peek()) {
            s.push(self.consume_char());
        }
        s
    }

    fn skip_whitespace(&mut self) {
        self.consume_while(|c| c.is_whitespace());
    }

    fn parse_tag_name(&mut self) -> String {
        self.consume_while(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            .to_lowercase()
    }

    fn parse_attr(&mut self) -> (String, String) {
        let name = self.parse_tag_name();
        if !self.eof() && self.peek() == '=' {
            self.consume_char();
            let quote = self.consume_char();
            let value = self.consume_while(|c| c != quote);
            if !self.eof() {
                self.consume_char();
            }
            (name, value)
        } else {
            (name, String::new())
        }
    }

    fn parse_attrs(&mut self) -> HashMap<String, String> {
        let mut attrs = HashMap::new();
        loop {
            self.skip_whitespace();
            if self.eof() || self.peek() == '>' || self.peek() == '/' {
                break;
            }
            let (k, v) = self.parse_attr();
            attrs.insert(k, v);
        }
        attrs
    }

    fn parse_text(&mut self) -> Node {
        let text = self.consume_while(|c| c != '<');
        // collapse whitespace
        let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let collapsed = if text.chars().next().map(|c| c.is_whitespace()).unwrap_or(false)
            && !collapsed.is_empty()
        {
            format!(" {}", collapsed)
        } else {
            collapsed
        };
        Node::Text(collapsed)
    }

    fn parse_comment(&mut self) {
        // Assumes we're past "<!--"
        while !self.eof() && !self.starts_with("-->") {
            self.consume_char();
        }
        if self.starts_with("-->") {
            self.pos += 3;
        }
    }

    fn parse_doctype(&mut self) {
        // skip until '>'
        while !self.eof() && self.peek() != '>' {
            self.consume_char();
        }
        if !self.eof() {
            self.consume_char();
        }
    }

    fn parse_element(&mut self) -> Option<Node> {
        // Expect '<'
        if self.peek() != '<' {
            return None;
        }
        // handle <!-- and <!DOCTYPE
        if self.starts_with("<!--") {
            self.pos += 4;
            self.parse_comment();
            return None;
        }
        if self.starts_with("<!") {
            self.pos += 2;
            self.parse_doctype();
            return None;
        }
        self.consume_char(); // '<'
        if !self.eof() && self.peek() == '/' {
            // Closing tag stray - consume & ignore
            self.consume_char();
            self.parse_tag_name();
            self.skip_whitespace();
            if !self.eof() {
                self.consume_char();
            }
            return None;
        }
        let tag = self.parse_tag_name();
        let attrs = self.parse_attrs();
        let mut self_closing = false;
        if !self.eof() && self.peek() == '/' {
            self_closing = true;
            self.consume_char();
        }
        if !self.eof() {
            self.consume_char(); // '>'
        }

        let is_void = VOID.contains(&tag.as_str()) || self_closing;

        // <style> and <script> are raw-text: consume until </tag>
        if tag == "style" || tag == "script" {
            let end = format!("</{}", tag);
            let mut text = String::new();
            while !self.eof() && !self.starts_with(&end) {
                text.push(self.consume_char());
            }
            // consume closing tag
            if self.starts_with(&end) {
                while !self.eof() && self.peek() != '>' {
                    self.consume_char();
                }
                if !self.eof() {
                    self.consume_char();
                }
            }
            let children = if tag == "style" {
                vec![Node::Text(text)]
            } else {
                vec![]
            };
            return Some(Node::Element(Element {
                tag,
                attrs,
                children,
            }));
        }

        let children = if is_void {
            Vec::new()
        } else {
            self.parse_nodes(&tag)
        };

        Some(Node::Element(Element {
            tag,
            attrs,
            children,
        }))
    }

    fn parse_nodes(&mut self, parent_tag: &str) -> Vec<Node> {
        let mut nodes = Vec::new();
        loop {
            if self.eof() {
                break;
            }
            if self.starts_with("</") {
                // closing tag for our parent: consume & stop
                let save = self.pos;
                self.pos += 2;
                let name = self.parse_tag_name();
                self.skip_whitespace();
                if !self.eof() && self.peek() == '>' {
                    self.consume_char();
                }
                if name != parent_tag {
                    // mismatched close — rewind if different parent; else drop
                    // For simplicity, drop it.
                    let _ = save;
                }
                break;
            }
            if self.peek() == '<' {
                if let Some(node) = self.parse_element() {
                    nodes.push(node);
                }
            } else {
                let t = self.parse_text();
                if let Node::Text(ref s) = t {
                    if !s.trim().is_empty() {
                        nodes.push(t);
                    }
                }
            }
        }
        nodes
    }

    pub fn parse_document(&mut self) -> Node {
        // Skip leading whitespace / doctype / comments
        loop {
            self.skip_whitespace();
            if self.eof() {
                break;
            }
            if self.starts_with("<!--") {
                self.pos += 4;
                self.parse_comment();
                continue;
            }
            if self.starts_with("<!") {
                self.pos += 2;
                self.parse_doctype();
                continue;
            }
            break;
        }
        // Parse one or more top-level nodes; wrap them in a synthetic root if needed.
        let mut top = Vec::new();
        while !self.eof() {
            self.skip_whitespace();
            if self.eof() {
                break;
            }
            if self.peek() == '<' {
                if let Some(n) = self.parse_element() {
                    top.push(n);
                }
            } else {
                let t = self.parse_text();
                if let Node::Text(ref s) = t {
                    if !s.trim().is_empty() {
                        top.push(t);
                    }
                }
            }
        }
        if top.len() == 1 {
            top.into_iter().next().unwrap()
        } else {
            Node::Element(Element {
                tag: "html".to_string(),
                attrs: HashMap::new(),
                children: top,
            })
        }
    }
}

pub fn parse(input: &str) -> Node {
    Parser::new(input).parse_document()
}

/// Extract concatenated text from all <style> elements in the tree.
pub fn extract_styles(node: &Node) -> String {
    let mut out = String::new();
    collect_styles(node, &mut out);
    out
}

fn collect_styles(node: &Node, out: &mut String) {
    if let Node::Element(e) = node {
        if e.tag == "style" {
            for c in &e.children {
                if let Node::Text(t) = c {
                    out.push_str(t);
                    out.push('\n');
                }
            }
            return;
        }
        for c in &e.children {
            collect_styles(c, out);
        }
    }
}
