//! Minimal CSS parser: supports simple selectors (tag, .class, #id, compound),
//! declarations with color/length/keyword values.

#[derive(Debug, Clone)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone, Default)]
pub struct Selector {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
}

impl Selector {
    /// Specificity: (id_count, class_count, tag_count)
    pub fn specificity(&self) -> (usize, usize, usize) {
        (
            self.id.iter().count(),
            self.classes.len(),
            self.tag.iter().count(),
        )
    }
}

#[derive(Debug, Clone)]
pub struct Declaration {
    pub name: String,
    pub value: Value,
}

#[derive(Debug, Clone)]
pub enum Value {
    Keyword(String),
    Length(f32, Unit),
    Color(Color),
}

#[derive(Debug, Clone, Copy)]
pub enum Unit {
    Px,
}

#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Value {
    pub fn to_px(&self) -> f32 {
        match self {
            Value::Length(v, Unit::Px) => *v,
            _ => 0.0,
        }
    }
    pub fn to_keyword(&self) -> Option<&str> {
        if let Value::Keyword(k) = self {
            Some(k)
        } else {
            None
        }
    }
    pub fn to_color(&self) -> Option<Color> {
        if let Value::Color(c) = self {
            Some(*c)
        } else {
            None
        }
    }
}

struct P {
    pos: usize,
    input: Vec<char>,
}

impl P {
    fn new(s: &str) -> Self {
        Self {
            pos: 0,
            input: s.chars().collect(),
        }
    }
    fn eof(&self) -> bool {
        self.pos >= self.input.len()
    }
    fn peek(&self) -> char {
        self.input[self.pos]
    }
    fn next(&mut self) -> char {
        let c = self.input[self.pos];
        self.pos += 1;
        c
    }
    fn skip_ws(&mut self) {
        while !self.eof() && (self.peek().is_whitespace() || self.starts_with("/*")) {
            if self.starts_with("/*") {
                self.pos += 2;
                while !self.eof() && !self.starts_with("*/") {
                    self.pos += 1;
                }
                if !self.eof() {
                    self.pos += 2;
                }
            } else {
                self.pos += 1;
            }
        }
    }
    fn starts_with(&self, s: &str) -> bool {
        self.input[self.pos..]
            .iter()
            .collect::<String>()
            .starts_with(s)
    }
    fn consume_while<F: Fn(char) -> bool>(&mut self, t: F) -> String {
        let mut out = String::new();
        while !self.eof() && t(self.peek()) {
            out.push(self.next());
        }
        out
    }
    fn parse_identifier(&mut self) -> String {
        self.consume_while(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    fn parse_selector(&mut self) -> Selector {
        let mut sel = Selector::default();
        loop {
            self.skip_ws();
            if self.eof() {
                break;
            }
            match self.peek() {
                '#' => {
                    self.next();
                    sel.id = Some(self.parse_identifier());
                }
                '.' => {
                    self.next();
                    sel.classes.push(self.parse_identifier());
                }
                c if c.is_ascii_alphanumeric() || c == '*' => {
                    let tag = self.parse_identifier();
                    if !tag.is_empty() {
                        sel.tag = Some(tag);
                    } else {
                        self.next();
                    }
                }
                _ => break,
            }
        }
        sel
    }

    fn parse_selectors(&mut self) -> Vec<Selector> {
        let mut sels = Vec::new();
        loop {
            self.skip_ws();
            let s = self.parse_selector();
            sels.push(s);
            self.skip_ws();
            if !self.eof() && self.peek() == ',' {
                self.next();
            } else {
                break;
            }
        }
        sels
    }

    fn parse_value(&mut self) -> Value {
        self.skip_ws();
        if self.peek() == '#' {
            self.next();
            let hex = self.consume_while(|c| c.is_ascii_hexdigit());
            return Value::Color(parse_hex(&hex));
        }
        if self.peek().is_ascii_digit() || self.peek() == '.' || self.peek() == '-' {
            let num_s = self.consume_while(|c| c.is_ascii_digit() || c == '.' || c == '-');
            let num: f32 = num_s.parse().unwrap_or(0.0);
            let unit = self.parse_identifier();
            if unit == "px" || unit.is_empty() {
                return Value::Length(num, Unit::Px);
            }
            return Value::Length(num, Unit::Px);
        }
        let ident = self.parse_identifier();
        // rgb(...) or named color
        if ident == "rgb" || ident == "rgba" {
            self.skip_ws();
            if !self.eof() && self.peek() == '(' {
                self.next();
                let body = self.consume_while(|c| c != ')');
                if !self.eof() {
                    self.next();
                }
                let parts: Vec<f32> = body
                    .split(',')
                    .map(|s| s.trim().parse().unwrap_or(0.0))
                    .collect();
                let r = parts.get(0).copied().unwrap_or(0.0) as u8;
                let g = parts.get(1).copied().unwrap_or(0.0) as u8;
                let b = parts.get(2).copied().unwrap_or(0.0) as u8;
                let a = parts
                    .get(3)
                    .copied()
                    .map(|v| (v * 255.0).clamp(0.0, 255.0) as u8)
                    .unwrap_or(255);
                return Value::Color(Color { r, g, b, a });
            }
        }
        if let Some(c) = named_color(&ident) {
            return Value::Color(c);
        }
        Value::Keyword(ident)
    }

    fn parse_declaration(&mut self) -> Option<Declaration> {
        self.skip_ws();
        let name = self.parse_identifier();
        if name.is_empty() {
            return None;
        }
        self.skip_ws();
        if self.eof() || self.peek() != ':' {
            return None;
        }
        self.next();
        let value = self.parse_value();
        self.skip_ws();
        if !self.eof() && self.peek() == ';' {
            self.next();
        }
        Some(Declaration { name, value })
    }

    fn parse_declarations(&mut self) -> Vec<Declaration> {
        let mut out = Vec::new();
        self.skip_ws();
        if !self.eof() && self.peek() == '{' {
            self.next();
        }
        loop {
            self.skip_ws();
            if self.eof() || self.peek() == '}' {
                if !self.eof() {
                    self.next();
                }
                break;
            }
            if let Some(d) = self.parse_declaration() {
                out.push(d);
            } else {
                // recover: skip to ; or }
                while !self.eof() && self.peek() != ';' && self.peek() != '}' {
                    self.next();
                }
                if !self.eof() && self.peek() == ';' {
                    self.next();
                }
            }
        }
        out
    }

    fn parse_rule(&mut self) -> Option<Rule> {
        self.skip_ws();
        if self.eof() {
            return None;
        }
        let selectors = self.parse_selectors();
        let declarations = self.parse_declarations();
        Some(Rule {
            selectors,
            declarations,
        })
    }
}

pub fn parse(input: &str) -> Stylesheet {
    let mut p = P::new(input);
    let mut rules = Vec::new();
    loop {
        p.skip_ws();
        if p.eof() {
            break;
        }
        if let Some(r) = p.parse_rule() {
            rules.push(r);
        } else {
            break;
        }
    }
    Stylesheet { rules }
}

fn parse_hex(hex: &str) -> Color {
    let expand = if hex.len() == 3 {
        hex.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        hex.to_string()
    };
    let r = u8::from_str_radix(expand.get(0..2).unwrap_or("00"), 16).unwrap_or(0);
    let g = u8::from_str_radix(expand.get(2..4).unwrap_or("00"), 16).unwrap_or(0);
    let b = u8::from_str_radix(expand.get(4..6).unwrap_or("00"), 16).unwrap_or(0);
    Color { r, g, b, a: 255 }
}

fn named_color(name: &str) -> Option<Color> {
    let c = |r, g, b| {
        Some(Color {
            r,
            g,
            b,
            a: 255,
        })
    };
    match name.to_lowercase().as_str() {
        "black" => c(0, 0, 0),
        "white" => c(255, 255, 255),
        "red" => c(255, 0, 0),
        "green" => c(0, 128, 0),
        "blue" => c(0, 0, 255),
        "yellow" => c(255, 255, 0),
        "orange" => c(255, 165, 0),
        "purple" => c(128, 0, 128),
        "gray" | "grey" => c(128, 128, 128),
        "lightgray" | "lightgrey" => c(211, 211, 211),
        "darkgray" | "darkgrey" => c(169, 169, 169),
        "silver" => c(192, 192, 192),
        "navy" => c(0, 0, 128),
        "teal" => c(0, 128, 128),
        "olive" => c(128, 128, 0),
        "maroon" => c(128, 0, 0),
        "lime" => c(0, 255, 0),
        "aqua" | "cyan" => c(0, 255, 255),
        "fuchsia" | "magenta" => c(255, 0, 255),
        "pink" => c(255, 192, 203),
        "brown" => c(165, 42, 42),
        "transparent" => Some(Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }),
        _ => None,
    }
}
