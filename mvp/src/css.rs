//! Minimal CSS parser: supports simple selectors (tag, .class, #id, compound),
//! declarations with color/length/keyword values. M2 adds multi-value lists
//! (e.g. `border: 1px solid red`), hex #rgb/#rrggbb, rgb()/rgba(), and an
//! expanded named color table.

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
    Number(f32),
    Color(Color),
    /// Multi-token value, e.g. `1px solid red` for shorthand `border`.
    List(Vec<Value>),
}

#[derive(Debug, Clone, Copy)]
pub enum Unit {
    Px,
}

#[derive(Debug, Clone, Copy, Default)]
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
            Value::Number(v) => *v,
            Value::List(items) => items.first().map(|v| v.to_px()).unwrap_or(0.0),
            _ => 0.0,
        }
    }
    pub fn to_number(&self) -> Option<f32> {
        match self {
            Value::Number(v) => Some(*v),
            Value::Length(v, _) => Some(*v),
            _ => None,
        }
    }
    pub fn to_keyword(&self) -> Option<&str> {
        match self {
            Value::Keyword(k) => Some(k),
            Value::List(items) => items.iter().find_map(|v| v.to_keyword()),
            _ => None,
        }
    }
    pub fn to_color(&self) -> Option<Color> {
        match self {
            Value::Color(c) => Some(*c),
            Value::List(items) => items.iter().find_map(|v| v.to_color()),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Vec<Value> {
        match self {
            Value::List(items) => items.clone(),
            v => vec![v.clone()],
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
    /// Skip only spaces/tabs — used between tokens within a single declaration
    /// value so we don't eat newlines that separate declarations on malformed
    /// input. In practice skip_ws is fine inside `{ ... }`.
    fn skip_inline_ws(&mut self) {
        while !self.eof() && (self.peek() == ' ' || self.peek() == '\t') {
            self.pos += 1;
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

    /// Parse a single value token (one of: #hex, number+unit, rgb(..), ident, named color).
    fn parse_value_token(&mut self) -> Option<Value> {
        self.skip_inline_ws();
        if self.eof() {
            return None;
        }
        let c = self.peek();
        if c == ';' || c == '}' {
            return None;
        }
        if c == '#' {
            self.next();
            let hex = self.consume_while(|c| c.is_ascii_hexdigit());
            return Some(Value::Color(parse_hex(&hex)));
        }
        if c.is_ascii_digit() || c == '.' || c == '-' {
            let num_s = self.consume_while(|c| c.is_ascii_digit() || c == '.' || c == '-');
            let num: f32 = num_s.parse().unwrap_or(0.0);
            // optional unit/percent
            if !self.eof() && self.peek() == '%' {
                self.next();
                // treat percents as raw number for now
                return Some(Value::Number(num));
            }
            let unit = self.parse_identifier();
            if unit.is_empty() {
                return Some(Value::Number(num));
            }
            // px/em/rem/pt -> px (approximate, M2 keeps px authoritative)
            return Some(Value::Length(num, Unit::Px));
        }
        let ident = self.parse_identifier();
        if ident.is_empty() {
            // Skip unknown char so we don't loop forever.
            self.next();
            return None;
        }
        // rgb(...) / rgba(...)
        if ident.eq_ignore_ascii_case("rgb") || ident.eq_ignore_ascii_case("rgba") {
            self.skip_inline_ws();
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
                let r = parts.get(0).copied().unwrap_or(0.0).clamp(0.0, 255.0) as u8;
                let g = parts.get(1).copied().unwrap_or(0.0).clamp(0.0, 255.0) as u8;
                let b = parts.get(2).copied().unwrap_or(0.0).clamp(0.0, 255.0) as u8;
                let a = parts
                    .get(3)
                    .copied()
                    .map(|v| (v * 255.0).clamp(0.0, 255.0) as u8)
                    .unwrap_or(255);
                return Some(Value::Color(Color { r, g, b, a }));
            }
        }
        if let Some(c) = named_color(&ident) {
            return Some(Value::Color(c));
        }
        Some(Value::Keyword(ident.to_lowercase()))
    }

    /// Parse the whole value portion of a declaration (everything up to `;` or `}`).
    /// If more than one token, returns Value::List.
    fn parse_value(&mut self) -> Value {
        let mut items = Vec::new();
        loop {
            // allow any whitespace between tokens
            while !self.eof() && self.peek().is_whitespace() {
                self.next();
            }
            if self.eof() {
                break;
            }
            let c = self.peek();
            if c == ';' || c == '}' {
                break;
            }
            if let Some(v) = self.parse_value_token() {
                items.push(v);
            } else {
                break;
            }
        }
        match items.len() {
            0 => Value::Keyword(String::new()),
            1 => items.into_iter().next().unwrap(),
            _ => Value::List(items),
        }
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
        Some(Declaration {
            name: name.to_lowercase(),
            value,
        })
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
    } else if hex.len() == 4 {
        // #rgba — not commonly needed, but harmless
        hex.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        hex.to_string()
    };
    let r = u8::from_str_radix(expand.get(0..2).unwrap_or("00"), 16).unwrap_or(0);
    let g = u8::from_str_radix(expand.get(2..4).unwrap_or("00"), 16).unwrap_or(0);
    let b = u8::from_str_radix(expand.get(4..6).unwrap_or("00"), 16).unwrap_or(0);
    let a = if expand.len() >= 8 {
        u8::from_str_radix(expand.get(6..8).unwrap_or("ff"), 16).unwrap_or(255)
    } else {
        255
    };
    Color { r, g, b, a }
}

fn named_color(name: &str) -> Option<Color> {
    let c = |r, g, b| Some(Color { r, g, b, a: 255 });
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
        "gold" => c(255, 215, 0),
        "indigo" => c(75, 0, 130),
        "violet" => c(238, 130, 238),
        "crimson" => c(220, 20, 60),
        "coral" => c(255, 127, 80),
        "salmon" => c(250, 128, 114),
        "tomato" => c(255, 99, 71),
        "khaki" => c(240, 230, 140),
        "beige" => c(245, 245, 220),
        "ivory" => c(255, 255, 240),
        "azure" => c(240, 255, 255),
        "lavender" => c(230, 230, 250),
        "plum" => c(221, 160, 221),
        "orchid" => c(218, 112, 214),
        "turquoise" => c(64, 224, 208),
        "aquamarine" => c(127, 255, 212),
        "chocolate" => c(210, 105, 30),
        "sienna" => c(160, 82, 45),
        "tan" => c(210, 180, 140),
        "wheat" => c(245, 222, 179),
        "lightblue" => c(173, 216, 230),
        "lightgreen" => c(144, 238, 144),
        "lightpink" => c(255, 182, 193),
        "lightyellow" => c(255, 255, 224),
        "lightcoral" => c(240, 128, 128),
        "lightsalmon" => c(255, 160, 122),
        "lightcyan" => c(224, 255, 255),
        "darkblue" => c(0, 0, 139),
        "darkgreen" => c(0, 100, 0),
        "darkred" => c(139, 0, 0),
        "darkorange" => c(255, 140, 0),
        "darkviolet" => c(148, 0, 211),
        "darkcyan" => c(0, 139, 139),
        "dodgerblue" => c(30, 144, 255),
        "royalblue" => c(65, 105, 225),
        "steelblue" => c(70, 130, 180),
        "skyblue" => c(135, 206, 235),
        "seagreen" => c(46, 139, 87),
        "forestgreen" => c(34, 139, 34),
        "slategray" | "slategrey" => c(112, 128, 144),
        "whitesmoke" => c(245, 245, 245),
        "ghostwhite" => c(248, 248, 255),
        "snow" => c(255, 250, 250),
        "mintcream" => c(245, 255, 250),
        "honeydew" => c(240, 255, 240),
        "transparent" => Some(Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }),
        _ => None,
    }
}
