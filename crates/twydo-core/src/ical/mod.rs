//! Lossless iCalendar content-line layer.
//!
//! Hand-written on purpose: library parsers normalise, and unchanged resources must stay
//! byte-equal. Every begin/end line and property keeps its original bytes (folds and line
//! endings included); [`serialize`] writes those bytes back.

use thiserror::Error;

pub mod value;

/// A parsed resource: the top-level `VCALENDAR` component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    pub root: Component,
    /// Leading UTF-8 BOM and blank lines before `BEGIN:VCALENDAR`, kept for serialization.
    prefix: Vec<u8>,
    /// Blank lines after `END:VCALENDAR`, kept for serialization.
    trailer: Vec<u8>,
}

/// `serialize` writes the original raw bytes of every unedited line.
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum Slot {
    Prop,
    Child,
    /// A blank line: ignored semantically, re-emitted verbatim.
    Blank(Vec<u8>),
}

/// `serialize` writes the original raw bytes until the property is edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    /// Uppercased property name.
    pub name: String,
    pub params: Vec<Param>,
    /// Unfolded, UTF-8 decoded value, exactly as written (no TEXT unescaping).
    pub value: String,
    /// Original bytes (folds and terminator included); `None` = edited, re-emit on write.
    raw: Option<Vec<u8>>,
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

    fn write_to(&self, out: &mut Vec<u8>, eol: &str) {
        emit_marker(out, &self.raw_begin, "BEGIN", &self.name, eol);
        let (mut p, mut c) = (0, 0);
        for slot in &self.layout {
            match slot {
                Slot::Prop if p < self.props.len() => {
                    self.props[p].write_to(out, eol);
                    p += 1;
                }
                Slot::Child if c < self.children.len() => {
                    self.children[c].write_to(out, eol);
                    c += 1;
                }
                Slot::Blank(raw) => out.extend_from_slice(raw),
                _ => {}
            }
        }
        for prop in &self.props[p..] {
            prop.write_to(out, eol);
        }
        for child in &self.children[c..] {
            child.write_to(out, eol);
        }
        emit_marker(out, &self.raw_end, "END", &self.name, eol);
    }

    /// A new, empty component. Its BEGIN/END lines are generated with the file's line ending.
    pub fn new(name: &str) -> Component {
        Component {
            name: name.to_ascii_uppercase(),
            props: Vec::new(),
            children: Vec::new(),
            raw_begin: Vec::new(),
            raw_end: Vec::new(),
            layout: Vec::new(),
        }
    }

    /// Append a child component after everything already in this component.
    pub fn push_child(&mut self, c: Component) {
        self.layout.push(Slot::Child);
        self.children.push(c);
    }

    pub fn prop_mut(&mut self, name: &str) -> Option<&mut Property> {
        self.props
            .iter_mut()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn children_named_mut<'a>(
        &'a mut self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a mut Component> {
        self.children
            .iter_mut()
            .filter(move |c| c.name.eq_ignore_ascii_case(name))
    }

    /// Replace the first property of the same name in place, else insert a new one.
    pub fn set_prop(&mut self, p: Property) {
        match self
            .props
            .iter_mut()
            .find(|q| q.name.eq_ignore_ascii_case(&p.name))
        {
            Some(existing) => *existing = p,
            None => self.insert_prop(p, None),
        }
    }

    /// Insert after the first property named `after`; otherwise (or if `after` is `None`)
    /// after the last property, before child components. All other lines keep their place.
    pub fn insert_prop(&mut self, p: Property, after: Option<&str>) {
        let after_idx = after.and_then(|n| {
            self.props
                .iter()
                .position(|q| q.name.eq_ignore_ascii_case(n))
        });
        let after_idx = after_idx.or_else(|| self.props.len().checked_sub(1));
        let slot_pos = match after_idx {
            Some(i) => {
                let pos = self
                    .layout
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| matches!(s, Slot::Prop))
                    .nth(i)
                    .map(|(k, _)| k + 1);
                pos.unwrap_or(self.layout.len())
            }
            None => self
                .layout
                .iter()
                .position(|s| matches!(s, Slot::Child))
                .unwrap_or(self.layout.len()),
        };
        let prop_pos = after_idx.map_or(0, |i| i + 1);
        self.layout.insert(slot_pos, Slot::Prop);
        self.props.insert(prop_pos, p);
    }

    /// Remove every property of that name (and its layout slot); returns how many.
    pub fn remove_props(&mut self, name: &str) -> usize {
        let gone: Vec<bool> = self
            .props
            .iter()
            .map(|q| q.name.eq_ignore_ascii_case(name))
            .collect();
        let n = gone.iter().filter(|&&g| g).count();
        if n == 0 {
            return 0;
        }
        let mut i = 0;
        self.layout.retain(|s| {
            if matches!(s, Slot::Prop) {
                i += 1;
                !gone.get(i - 1).copied().unwrap_or(false)
            } else {
                true
            }
        });
        let mut k = 0;
        self.props.retain(|_| {
            k += 1;
            !gone[k - 1]
        });
        n
    }

    /// Remove every child component matching `pred` (and its layout slot); returns how many.
    pub fn remove_children(&mut self, pred: impl Fn(&Component) -> bool) -> usize {
        let gone: Vec<bool> = self.children.iter().map(&pred).collect();
        let n = gone.iter().filter(|&&g| g).count();
        if n == 0 {
            return 0;
        }
        let mut i = 0;
        self.layout.retain(|s| {
            if matches!(s, Slot::Child) {
                i += 1;
                !gone.get(i - 1).copied().unwrap_or(false)
            } else {
                true
            }
        });
        let mut k = 0;
        self.children.retain(|_| {
            k += 1;
            !gone[k - 1]
        });
        n
    }
}

/// Original BEGIN/END bytes, or a generated line when the component is new.
fn emit_marker(out: &mut Vec<u8>, raw: &[u8], kw: &str, name: &str, eol: &str) {
    if raw.is_empty() {
        out.extend_from_slice(format!("{kw}:{name}{eol}").as_bytes());
    } else {
        out.extend_from_slice(raw);
    }
}

impl Property {
    /// A new property with no original bytes: it is emitted fresh when written.
    pub fn new(name: &str, params: Vec<Param>, value: &str) -> Property {
        Property {
            name: name.to_ascii_uppercase(),
            params,
            value: value.to_string(),
            raw: None,
        }
    }

    /// Change the value, keep the parameters; the line is re-emitted on write.
    pub fn set_value(&mut self, value: &str) {
        self.value = value.to_string();
        self.raw = None;
    }

    pub fn set_params(&mut self, params: Vec<Param>) {
        self.params = params;
        self.raw = None;
    }

    fn write_to(&self, out: &mut Vec<u8>, eol: &str) {
        match &self.raw {
            Some(raw) => out.extend_from_slice(raw),
            None => fold_into(out, &self.render(), eol),
        }
    }

    fn render(&self) -> String {
        let mut line = self.name.clone();
        for p in &self.params {
            line.push(';');
            line.push_str(&p.name);
            line.push('=');
            let vals: Vec<String> = p
                .values
                .iter()
                .map(|v| {
                    if v.contains([':', ';', ',']) {
                        format!("\"{v}\"")
                    } else {
                        v.clone()
                    }
                })
                .collect();
            line.push_str(&vals.join(","));
        }
        line.push(':');
        line.push_str(&self.value);
        line
    }
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
        raw: Some(l.raw.clone()),
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
    let (prefix, bytes) = match bytes.strip_prefix(b"\xEF\xBB\xBF") {
        Some(rest) => (b"\xEF\xBB\xBF".to_vec(), rest),
        None => (Vec::new(), bytes),
    };
    let mut prefix = prefix;
    let mut trailer = Vec::new();
    let lines = logical_lines(bytes);
    let last_line = bytes.iter().filter(|&&b| b == b'\n').count()
        + usize::from(bytes.last().is_some_and(|&b| b != b'\n'));
    let mut stack: Vec<Component> = Vec::new();
    let mut root: Option<Component> = None;

    for l in &lines {
        if l.content.is_empty() {
            match stack.last_mut() {
                Some(top) => top.layout.push(Slot::Blank(l.raw.clone())),
                None if root.is_some() => trailer.extend_from_slice(&l.raw),
                None => prefix.extend_from_slice(&l.raw),
            }
            continue;
        }
        if root.is_some() {
            return Err(err(l.line, "content after END:VCALENDAR"));
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
        (Some(root), _) => Ok(Calendar {
            root,
            prefix,
            trailer,
        }),
        (None, Some(open)) => Err(err(last_line, format!("missing END:{}", open.name))),
        (None, None) => Err(err(1, "no VCALENDAR found")),
    }
}

/// Fold to physical lines of at most 75 octets (continuations: one space + up to 74),
/// never splitting a UTF-8 sequence.
fn fold_into(out: &mut Vec<u8>, line: &str, eol: &str) {
    let mut limit = 75;
    let mut used = 0;
    for ch in line.chars() {
        let n = ch.len_utf8();
        if used + n > limit {
            out.extend_from_slice(eol.as_bytes());
            out.push(b' ');
            limit = 75;
            used = 1;
        }
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        used += n;
    }
    out.extend_from_slice(eol.as_bytes());
}

impl Calendar {
    /// `"\r\n"` or `"\n"`, taken from the BEGIN:VCALENDAR line (CRLF for a new calendar).
    pub fn line_ending(&self) -> &'static str {
        let b = &self.root.raw_begin;
        if b.is_empty() || b.ends_with(b"\r\n") {
            "\r\n"
        } else {
            "\n"
        }
    }
}

/// A calendar for a newly created resource (CRLF line endings).
pub fn new_calendar(root: Component) -> Calendar {
    Calendar {
        root,
        prefix: Vec::new(),
        trailer: Vec::new(),
    }
}

pub fn serialize(cal: &Calendar) -> Vec<u8> {
    let mut out = cal.prefix.clone();
    cal.root.write_to(&mut out, cal.line_ending());
    out.extend_from_slice(&cal.trailer);
    out
}
