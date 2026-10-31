//! Lossless iCalendar content-line layer.
//!
//! Hand-written on purpose: library parsers normalise, and unchanged resources must stay
//! byte-equal. Every begin/end line and property keeps its original bytes (folds and line
//! endings included); [`serialize`] writes those bytes back.

use thiserror::Error;

/// A parsed resource: the top-level `VCALENDAR` component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    pub root: Component,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    /// Uppercased component name.
    pub name: String,
    pub props: Vec<Property>,
    pub children: Vec<Component>,
    raw_begin: Vec<u8>,
    raw_end: Vec<u8>,
    /// Original interleaving of properties and children, so serialization keeps line order.
    layout: Vec<Slot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Prop,
    Child,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    /// Uppercased property name.
    pub name: String,
    pub params: Vec<Param>,
    /// Unfolded, UTF-8 decoded value, exactly as written (no TEXT unescaping).
    pub value: String,
    raw: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    /// Uppercased parameter name.
    pub name: String,
    /// Comma-separated values, DQUOTEs removed.
    pub values: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("line {line}: {msg}")]
    At { line: usize, msg: String },
}

fn err(line: usize, msg: impl Into<String>) -> ParseError {
    ParseError::At {
        line,
        msg: msg.into(),
    }
}

impl Component {
    pub fn prop(&self, name: &str) -> Option<&Property> {
        self.props
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn props_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Property> {
        self.props
            .iter()
            .filter(move |p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Component> {
        self.children
            .iter()
            .filter(move |c| c.name.eq_ignore_ascii_case(name))
    }

    fn write_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.raw_begin);
        let (mut p, mut c) = (0, 0);
        for slot in &self.layout {
            match slot {
                Slot::Prop if p < self.props.len() => {
                    out.extend_from_slice(&self.props[p].raw);
                    p += 1;
                }
                Slot::Child if c < self.children.len() => {
                    self.children[c].write_to(out);
                    c += 1;
                }
                _ => {}
            }
        }
        for prop in &self.props[p..] {
            out.extend_from_slice(&prop.raw);
        }
        for child in &self.children[c..] {
            child.write_to(out);
        }
        out.extend_from_slice(&self.raw_end);
    }
}

impl Property {
    /// First value of the named parameter (case-insensitive name).
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .and_then(|p| p.values.first().map(String::as_str))
    }
}

/// One logical (unfolded) content line.
struct Logical {
    /// 1-based number of its first physical line.
    line: usize,
    /// Original bytes, folds and terminator included.
    raw: Vec<u8>,
    /// Unfolded content without terminator.
    content: Vec<u8>,
}

/// Split into physical lines (terminators kept) and group continuation lines.
fn logical_lines(bytes: &[u8]) -> Vec<Logical> {
    let mut out: Vec<Logical> = Vec::new();
    let mut pos = 0;
    let mut line_no = 0;
    while pos < bytes.len() {
        let end = bytes[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |i| pos + i + 1);
        let phys = &bytes[pos..end];
        line_no += 1;
        let body = phys
            .strip_suffix(b"\n")
            .map_or(phys, |s| s.strip_suffix(b"\r").unwrap_or(s));
        match (phys.first(), out.last_mut()) {
            (Some(b' ' | b'\t'), Some(prev)) => {
                prev.raw.extend_from_slice(phys);
                prev.content.extend_from_slice(&body[1..]);
            }
            _ => out.push(Logical {
                line: line_no,
                raw: phys.to_vec(),
                content: body.to_vec(),
            }),
        }
        pos = end;
    }
    out
}

/// Parse `name *(";" param) ":" value`, honouring DQUOTE.
fn parse_property(l: &Logical) -> Result<Property, ParseError> {
    let s = std::str::from_utf8(&l.content).map_err(|_| err(l.line, "invalid UTF-8"))?;
    let b = s.as_bytes();
    let stop = b
        .iter()
        .position(|&c| c == b';' || c == b':')
        .ok_or_else(|| err(l.line, "missing ':'"))?;
    if stop == 0 {
        return Err(err(l.line, "empty property name"));
    }
    let name = s[..stop].to_ascii_uppercase();
    let mut i = stop;
    let mut params = Vec::new();
    while b[i] == b';' {
        i += 1;
        let eq = b[i..]
            .iter()
            .position(|&c| matches!(c, b'=' | b';' | b':'))
            .filter(|&k| b[i + k] == b'=')
            .ok_or_else(|| err(l.line, "parameter without '='"))?;
        let pname = s[i..i + eq].to_ascii_uppercase();
        if pname.is_empty() {
            return Err(err(l.line, "empty parameter name"));
        }
        i += eq + 1;
        let mut values = Vec::new();
        loop {
            if b.get(i) == Some(&b'"') {
                let close = b[i + 1..]
                    .iter()
                    .position(|&c| c == b'"')
                    .ok_or_else(|| err(l.line, "unterminated quoted parameter value"))?;
                values.push(s[i + 1..i + 1 + close].to_string());
                i += close + 2;
            } else {
                let k = b[i..]
                    .iter()
                    .position(|&c| matches!(c, b',' | b';' | b':'))
                    .ok_or_else(|| err(l.line, "missing ':'"))?;
                values.push(s[i..i + k].to_string());
                i += k;
            }
            match b.get(i) {
                Some(b',') => i += 1,
                Some(b';' | b':') => break,
                _ => return Err(err(l.line, "malformed parameter value")),
            }
        }
        params.push(Param {
            name: pname,
            values,
        });
    }
    Ok(Property {
        name,
        params,
        value: s[i + 1..].to_string(),
        raw: l.raw.clone(),
    })
}

/// Return `Some(upper-cased name)` when the content is `BEGIN:` / `END:` (per `keyword`).
fn marker(content: &[u8], keyword: &str) -> Option<String> {
    let k = keyword.len();
    if content.len() > k
        && content[..k].eq_ignore_ascii_case(keyword.as_bytes())
        && content[k] == b':'
    {
        Some(
            String::from_utf8_lossy(&content[k + 1..])
                .trim()
                .to_ascii_uppercase(),
        )
    } else {
        None
    }
}

pub fn parse(bytes: &[u8]) -> Result<Calendar, ParseError> {
    let lines = logical_lines(bytes);
    let last_line = bytes.iter().filter(|&&b| b == b'\n').count()
        + usize::from(bytes.last().is_some_and(|&b| b != b'\n'));
    let mut stack: Vec<Component> = Vec::new();
    let mut root: Option<Component> = None;

    for l in &lines {
        if root.is_some() {
            return Err(err(l.line, "content after END:VCALENDAR"));
        }
        if l.content.is_empty() {
            return Err(err(l.line, "empty line"));
        }
        if let Some(name) = marker(&l.content, "BEGIN") {
            if stack.is_empty() && name != "VCALENDAR" {
                return Err(err(l.line, "expected BEGIN:VCALENDAR"));
            }
            stack.push(Component {
                name,
                props: Vec::new(),
                children: Vec::new(),
                raw_begin: l.raw.clone(),
                raw_end: Vec::new(),
                layout: Vec::new(),
            });
        } else if let Some(name) = marker(&l.content, "END") {
            let mut done = stack
                .pop()
                .ok_or_else(|| err(l.line, "END without BEGIN"))?;
            if done.name != name {
                return Err(err(
                    l.line,
                    format!("END:{name} does not match BEGIN:{}", done.name),
                ));
            }
            done.raw_end = l.raw.clone();
            match stack.last_mut() {
                Some(parent) => {
                    parent.layout.push(Slot::Child);
                    parent.children.push(done);
                }
                None => root = Some(done),
            }
        } else {
            let prop = parse_property(l)?;
            let top = stack
                .last_mut()
                .ok_or_else(|| err(l.line, "expected BEGIN:VCALENDAR"))?;
            top.layout.push(Slot::Prop);
            top.props.push(prop);
        }
    }

    match (root, stack.last()) {
        (Some(root), _) => Ok(Calendar { root }),
        (None, Some(open)) => Err(err(last_line, format!("missing END:{}", open.name))),
        (None, None) => Err(err(1, "no VCALENDAR found")),
    }
}

pub fn serialize(cal: &Calendar) -> Vec<u8> {
    let mut out = Vec::new();
    cal.root.write_to(&mut out);
    out
}
