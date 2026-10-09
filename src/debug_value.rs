//! Parsing and navigation of `Debug` output.
//!
//! Rust has no runtime reflection, so sub-field access like `order.items`
//! works by parsing the captured `{:?}` text of a variable into a tree and
//! navigating it. Output produced by `#[derive(Debug)]` (and the standard
//! `Formatter` helpers) parses reliably; exotic hand-written `Debug` impls
//! may not.

#[derive(Debug, PartialEq)]
pub enum DebugValue {
    Struct {
        name: String,
        fields: Vec<(String, DebugValue)>,
    },
    TupleStruct {
        name: String,
        items: Vec<DebugValue>,
    },
    List(Vec<DebugValue>),
    Tuple(Vec<DebugValue>),
    Set(Vec<DebugValue>),
    Map(Vec<(DebugValue, DebugValue)>),
    /// Anything atomic: numbers, strings (with quotes), bools, unit
    /// structs, `..`, etc.
    Lit(String),
}

#[derive(Debug, PartialEq)]
pub enum PathSeg {
    Field(String),
    Index(usize),
    Key(String),
}

/// Parses a path expression like `order.items[0].name` or `map["key"]`
/// into a base variable name and path segments. Returns `None` when the
/// input is not a path (no segments, or invalid syntax).
pub fn parse_path(expr: &str) -> Option<(&str, Vec<PathSeg>)> {
    let b = expr.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    let base = &expr[..i];
    let mut segs = Vec::new();
    while i < b.len() {
        match b[i] {
            b'.' => {
                i += 1;
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                if i == start {
                    return None;
                }
                let name = &expr[start..i];
                match name.parse::<usize>() {
                    Ok(n) => segs.push(PathSeg::Index(n)),
                    Err(_) => segs.push(PathSeg::Field(name.to_string())),
                }
            }
            b'[' => {
                i += 1;
                if i < b.len() && b[i] == b'"' {
                    i += 1;
                    let start = i;
                    while i < b.len() && b[i] != b'"' {
                        i += 1;
                    }
                    if i >= b.len() {
                        return None;
                    }
                    segs.push(PathSeg::Key(expr[start..i].to_string()));
                    i += 1;
                } else {
                    let start = i;
                    while i < b.len() && b[i] != b']' {
                        i += 1;
                    }
                    let n = expr[start..i].trim().parse::<usize>().ok()?;
                    segs.push(PathSeg::Index(n));
                }
                if i >= b.len() || b[i] != b']' {
                    return None;
                }
                i += 1;
            }
            _ => return None,
        }
    }
    if segs.is_empty() {
        None
    } else {
        Some((base, segs))
    }
}

/// Navigates a parsed value along path segments.
pub fn navigate<'a>(mut v: &'a DebugValue, segs: &[PathSeg]) -> Result<&'a DebugValue, String> {
    for seg in segs {
        v = step(v, seg)?;
    }
    Ok(v)
}

fn step<'a>(v: &'a DebugValue, seg: &PathSeg) -> Result<&'a DebugValue, String> {
    match seg {
        PathSeg::Field(name) => match v {
            DebugValue::Struct { fields, .. } => fields
                .iter()
                .find(|(f, _)| f == name)
                .map(|(_, v)| v)
                .ok_or_else(|| format!("no field {name:?} (available: {})", field_names(fields))),
            DebugValue::Map(entries) => lookup_key(entries, name),
            _ => Err(format!("cannot access field {name:?} on {}", kind(v))),
        },
        PathSeg::Index(n) => {
            let items = match v {
                DebugValue::List(items)
                | DebugValue::Tuple(items)
                | DebugValue::Set(items)
                | DebugValue::TupleStruct { items, .. } => items,
                _ => return Err(format!("cannot index {}", kind(v))),
            };
            items
                .get(*n)
                .ok_or_else(|| format!("index {n} out of range (len {})", items.len()))
        }
        PathSeg::Key(key) => match v {
            DebugValue::Map(entries) => lookup_key(entries, key),
            _ => Err(format!("cannot access key {key:?} on {}", kind(v))),
        },
    }
}

fn lookup_key<'a>(
    entries: &'a [(DebugValue, DebugValue)],
    key: &str,
) -> Result<&'a DebugValue, String> {
    let quoted = format!("{key:?}");
    entries
        .iter()
        .find(|(k, _)| match k {
            DebugValue::Lit(s) => s == key || *s == quoted,
            _ => false,
        })
        .map(|(_, v)| v)
        .ok_or_else(|| format!("no key {key:?} in map"))
}

fn field_names(fields: &[(String, DebugValue)]) -> String {
    fields
        .iter()
        .map(|(f, _)| f.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn kind(v: &DebugValue) -> &'static str {
    match v {
        DebugValue::Struct { .. } => "a struct",
        DebugValue::TupleStruct { .. } => "a tuple struct",
        DebugValue::List(_) => "a list",
        DebugValue::Tuple(_) => "a tuple",
        DebugValue::Set(_) => "a set",
        DebugValue::Map(_) => "a map",
        DebugValue::Lit(_) => "a scalar value",
    }
}

impl DebugValue {
    pub fn render_compact(&self) -> String {
        match self {
            DebugValue::Lit(s) => s.clone(),
            DebugValue::Struct { name, fields } => {
                if fields.is_empty() {
                    name.clone()
                } else {
                    let body = fields
                        .iter()
                        .map(|(f, v)| format!("{f}: {}", v.render_compact()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{name} {{ {body} }}")
                }
            }
            DebugValue::TupleStruct { name, items } => {
                format!("{name}({})", join_compact(items))
            }
            DebugValue::List(items) => format!("[{}]", join_compact(items)),
            DebugValue::Tuple(items) => format!("({})", join_compact(items)),
            DebugValue::Set(items) => format!("{{{}}}", join_compact(items)),
            DebugValue::Map(entries) => {
                let body = entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.render_compact(), v.render_compact()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{body}}}")
            }
        }
    }

    pub fn render_pretty(&self) -> String {
        let mut out = String::new();
        self.pretty(0, &mut out);
        out
    }

    fn pretty(&self, level: usize, out: &mut String) {
        let pad = "    ".repeat(level + 1);
        let close_pad = "    ".repeat(level);
        match self {
            DebugValue::Lit(s) => out.push_str(s),
            DebugValue::Struct { name, fields } => {
                if fields.is_empty() {
                    out.push_str(name);
                    return;
                }
                out.push_str(name);
                out.push_str(" {\n");
                for (f, v) in fields {
                    out.push_str(&pad);
                    out.push_str(f);
                    out.push_str(": ");
                    v.pretty(level + 1, out);
                    out.push_str(",\n");
                }
                out.push_str(&close_pad);
                out.push('}');
            }
            DebugValue::TupleStruct { name, items } => {
                out.push_str(name);
                pretty_seq(items, level, "(", ")", out);
            }
            DebugValue::List(items) => pretty_seq(items, level, "[", "]", out),
            DebugValue::Tuple(items) => pretty_seq(items, level, "(", ")", out),
            DebugValue::Set(items) => pretty_seq(items, level, "{", "}", out),
            DebugValue::Map(entries) => {
                if entries.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                for (k, v) in entries {
                    out.push_str(&pad);
                    k.pretty(level + 1, out);
                    out.push_str(": ");
                    v.pretty(level + 1, out);
                    out.push_str(",\n");
                }
                out.push_str(&close_pad);
                out.push('}');
            }
        }
    }
}

fn join_compact(items: &[DebugValue]) -> String {
    items
        .iter()
        .map(|v| v.render_compact())
        .collect::<Vec<_>>()
        .join(", ")
}

fn pretty_seq(items: &[DebugValue], level: usize, open: &str, close: &str, out: &mut String) {
    if items.is_empty() {
        out.push_str(open);
        out.push_str(close);
        return;
    }
    let pad = "    ".repeat(level + 1);
    out.push_str(open);
    out.push('\n');
    for v in items {
        out.push_str(&pad);
        v.pretty(level + 1, out);
        out.push_str(",\n");
    }
    out.push_str(&"    ".repeat(level));
    out.push_str(close);
}

/// Parses the `{:?}` (compact) Debug representation of a value.
pub fn parse(input: &str) -> Option<DebugValue> {
    let mut p = Parser {
        s: input.as_bytes(),
        src: input,
        pos: 0,
    };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    if p.pos == p.s.len() {
        Some(v)
    } else {
        None
    }
}

struct Parser<'a> {
    s: &'a [u8],
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ') | Some(b'\n') | Some(b'\t')) {
            self.pos += 1;
        }
    }

    fn value(&mut self) -> Option<DebugValue> {
        self.skip_ws();
        match self.peek()? {
            b'[' => self.seq(b'[', b']').map(DebugValue::List),
            b'(' => self.seq(b'(', b')').map(DebugValue::Tuple),
            b'{' => self.map_or_set(),
            b'"' => self.string_lit(),
            b'\'' => self.char_lit(),
            _ => self.ident_or_lit(),
        }
    }

    /// A named token: identifier chars plus `::` (for paths like
    /// `mod::Struct`) and leading `&`.
    fn named_token(&mut self) -> &'a str {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' || c == b':' || c == b'&' {
                self.pos += 1;
            } else {
                break;
            }
        }
        &self.src[start..self.pos]
    }

    fn ident_or_lit(&mut self) -> Option<DebugValue> {
        let start = self.pos;
        let token = self.named_token();
        let is_name = token
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '&');
        if is_name {
            let after_token = self.pos;
            self.skip_ws();
            match self.peek() {
                Some(b'{') => {
                    let fields = self.struct_fields()?;
                    return Some(DebugValue::Struct {
                        name: token.to_string(),
                        fields,
                    });
                }
                Some(b'(') => {
                    let items = self.seq(b'(', b')')?;
                    return Some(DebugValue::TupleStruct {
                        name: token.to_string(),
                        items,
                    });
                }
                _ => self.pos = after_token,
            }
        }
        // Plain literal: consume until a top-level delimiter.
        self.pos = start;
        self.lit_until_delim()
    }

    fn lit_until_delim(&mut self) -> Option<DebugValue> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            match c {
                b',' | b']' | b'}' | b')' => break,
                b':' => {
                    // `::` inside a path (e.g. `Kind::A`) is part of the
                    // literal; a single `:` is a map-key delimiter.
                    if self.s.get(self.pos + 1) == Some(&b':') {
                        self.pos += 2;
                    } else {
                        break;
                    }
                }
                b'"' => {
                    self.string_lit()?;
                }
                b'\'' => {
                    self.char_lit()?;
                }
                _ => self.pos += 1,
            }
        }
        let text = self.src[start..self.pos].trim();
        if text.is_empty() {
            None
        } else {
            Some(DebugValue::Lit(text.to_string()))
        }
    }

    fn string_lit(&mut self) -> Option<DebugValue> {
        let start = self.pos;
        self.pos += 1; // opening quote
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                b'\\' => self.pos += 1, // skip escaped char
                b'"' => return Some(DebugValue::Lit(self.src[start..self.pos].to_string())),
                _ => {}
            }
        }
        None
    }

    fn char_lit(&mut self) -> Option<DebugValue> {
        let start = self.pos;
        self.pos += 1; // opening quote
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                b'\\' => self.pos += 1,
                b'\'' => return Some(DebugValue::Lit(self.src[start..self.pos].to_string())),
                _ => {}
            }
        }
        None
    }

    fn seq(&mut self, open: u8, close: u8) -> Option<Vec<DebugValue>> {
        debug_assert_eq!(self.peek(), Some(open));
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek()? == close {
                self.pos += 1;
                return Some(items);
            }
            items.push(self.value()?);
            self.skip_ws();
            match self.peek()? {
                b',' => self.pos += 1,
                c if c == close => {}
                _ => return None,
            }
        }
    }

    fn map_or_set(&mut self) -> Option<DebugValue> {
        self.pos += 1; // '{'
        self.skip_ws();
        if self.peek()? == b'}' {
            self.pos += 1;
            return Some(DebugValue::Map(Vec::new()));
        }
        let first = self.value()?;
        self.skip_ws();
        if self.peek()? == b':' {
            // Map: { key: value, ... }
            self.pos += 1;
            let mut entries = Vec::new();
            let v = self.value()?;
            entries.push((first, v));
            loop {
                self.skip_ws();
                match self.peek()? {
                    b'}' => {
                        self.pos += 1;
                        return Some(DebugValue::Map(entries));
                    }
                    b',' => self.pos += 1,
                    _ => return None,
                }
                self.skip_ws();
                if self.peek()? == b'}' {
                    self.pos += 1;
                    return Some(DebugValue::Map(entries));
                }
                let k = self.value()?;
                self.skip_ws();
                if self.peek()? != b':' {
                    return None;
                }
                self.pos += 1;
                let v = self.value()?;
                entries.push((k, v));
            }
        }
        // Set: { a, b, ... }
        let mut items = vec![first];
        loop {
            self.skip_ws();
            match self.peek()? {
                b'}' => {
                    self.pos += 1;
                    return Some(DebugValue::Set(items));
                }
                b',' => self.pos += 1,
                _ => return None,
            }
            self.skip_ws();
            if self.peek()? == b'}' {
                self.pos += 1;
                return Some(DebugValue::Set(items));
            }
            items.push(self.value()?);
        }
    }

    fn struct_fields(&mut self) -> Option<Vec<(String, DebugValue)>> {
        debug_assert_eq!(self.peek(), Some(b'{'));
        self.pos += 1;
        let mut fields = Vec::new();
        loop {
            self.skip_ws();
            match self.peek()? {
                b'}' => {
                    self.pos += 1;
                    return Some(fields);
                }
                b'.' => {
                    // `..` from finish_non_exhaustive()
                    while self.peek() == Some(b'.') {
                        self.pos += 1;
                    }
                    continue;
                }
                _ => {}
            }
            // field name up to ':'
            let start = self.pos;
            while let Some(c) = self.peek() {
                if c == b':' || c == b'}' {
                    break;
                }
                self.pos += 1;
            }
            if self.peek()? != b':' {
                return None;
            }
            let name = self.src[start..self.pos].trim().to_string();
            self.pos += 1;
            let v = self.value()?;
            fields.push((name, v));
            self.skip_ws();
            match self.peek()? {
                b',' => self.pos += 1,
                b'}' => {}
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nav(input: &str, path: &str) -> Result<String, String> {
        let (_, segs) = parse_path(&format!("x{path}")).ok_or("bad path")?;
        let tree = parse(input).ok_or("parse failed")?;
        navigate(&tree, &segs).map(|v| v.render_compact())
    }

    #[test]
    fn parses_struct_and_accesses_field() {
        let s = r#"Order { id: 1001, customer: "tanaka", items: [Item { name: "kb", price: 12000, quantity: 1 }] }"#;
        assert_eq!(nav(s, ".id").unwrap(), "1001");
        assert_eq!(nav(s, ".customer").unwrap(), "\"tanaka\"");
        assert_eq!(
            nav(s, ".items").unwrap(),
            r#"[Item { name: "kb", price: 12000, quantity: 1 }]"#
        );
        assert_eq!(nav(s, ".items[0].name").unwrap(), "\"kb\"");
    }

    #[test]
    fn missing_field_reports_available_names() {
        let s = "Point { x: 1, y: 2 }";
        let err = nav(s, ".z").unwrap_err();
        assert!(err.contains("x, y"), "err: {err}");
    }

    #[test]
    fn indexes_lists_and_tuples() {
        assert_eq!(nav("[10, 20, 30]", "[1]").unwrap(), "20");
        assert_eq!(nav("(1, \"two\")", ".1").unwrap(), "\"two\"");
        assert_eq!(nav("Some(5)", "[0]").unwrap(), "5");
        assert!(nav("[1]", "[5]").unwrap_err().contains("out of range"));
    }

    #[test]
    fn map_access_by_key() {
        let s = r#"{"a": 1, "b": [2, 3]}"#;
        assert_eq!(nav(s, "[\"a\"]").unwrap(), "1");
        assert_eq!(nav(s, ".b[1]").unwrap(), "3");
        assert!(nav(s, "[\"zzz\"]").unwrap_err().contains("no key"));
    }

    #[test]
    fn handles_strings_with_delimiters() {
        let s = r#"S { msg: "a, b } ] weird", n: 1 }"#;
        assert_eq!(nav(s, ".msg").unwrap(), r#""a, b } ] weird""#);
        assert_eq!(nav(s, ".n").unwrap(), "1");
    }

    #[test]
    fn nested_enums_and_options() {
        let s = r#"S { v: Some(Inner { k: 7 }), e: Kind::A }"#;
        // Note: derive(Debug) prints enum variants bare (e.g. `A`), but
        // paths also parse.
        assert_eq!(nav(s, ".v[0].k").unwrap(), "7");
    }

    #[test]
    fn pretty_rendering_matches_alternate_format() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Item {
            name: &'static str,
            price: u32,
        }
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Order {
            id: u64,
            items: Vec<Item>,
        }
        let order = Order {
            id: 1,
            items: vec![Item {
                name: "kb",
                price: 100,
            }],
        };
        let tree = parse(&format!("{order:?}")).unwrap();
        assert_eq!(tree.render_pretty(), format!("{order:#?}"));
        assert_eq!(tree.render_compact(), format!("{order:?}"));
    }

    #[test]
    fn rejects_non_path_expressions() {
        assert!(parse_path("order.items.len()").is_none());
        assert!(parse_path("1 + 2").is_none());
        assert!(parse_path("order").is_none()); // no segments
    }
}
