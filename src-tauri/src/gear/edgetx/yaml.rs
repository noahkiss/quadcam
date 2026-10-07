//! A line-level reader and editor for EdgeTX YAML files (design 6.3).
//!
//! EdgeTX writes a small, regular subset of YAML: `key: value` lines, block headers
//! (`key: ` or `N:`), and ` -` list items, indented with spaces. This module reads exactly
//! that subset and nothing more. It never round-trips through a generic YAML library,
//! which would reorder keys, change quoting and drop layout.
//!
//! - A file is bytes. They are read as Latin-1, so every byte maps to one `char` and back:
//!   a file renders to the same bytes it was read from, whatever it holds.
//! - Every line keeps its own line ending (CRLF or LF). New lines take the file's ending.
//! - `Doc::parse` refuses a line it does not understand, with the file and the line
//!   number, and refuses a file that does not render back to its own bytes.
//! - Edits change whole lines. Untouched lines stay byte for byte.

use crate::gear::model::{Refusal, RefusalCode};
use std::ops::Range;

/// One classified line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// `key: value`, `key: ` (a block header with a space) or `key:` (`N:` headers).
    Key {
        key: String,
        /// A space follows the colon.
        space: bool,
        /// Everything after `": "`, raw (quotes kept).
        value: String,
    },
    /// ` -`: a list item; its fields are the deeper lines that follow.
    Dash,
    /// An empty line.
    Blank,
}

/// A line's indent and kind. `render` gives back the line's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub indent: usize,
    pub kind: Kind,
}

impl Line {
    /// Classifies one line (without its ending). None: not understood.
    pub fn parse(s: &str) -> Option<Line> {
        if s.is_empty() {
            return Some(Line {
                indent: 0,
                kind: Kind::Blank,
            });
        }
        let indent = s.bytes().take_while(|b| *b == b' ').count();
        let rest = &s[indent..];
        if rest.is_empty() {
            return None;
        }
        if rest == "-" {
            return Some(Line {
                indent,
                kind: Kind::Dash,
            });
        }
        let klen = rest
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
            .count();
        if klen == 0 {
            return None;
        }
        let key = &rest[..klen];
        let after = rest[klen..].strip_prefix(':')?;
        let (space, value) = match after.strip_prefix(' ') {
            Some(v) => (true, v),
            None if after.is_empty() => (false, ""),
            None => return None,
        };
        if value.contains(['\t', '\r']) {
            return None;
        }
        Some(Line {
            indent,
            kind: Kind::Key {
                key: key.to_string(),
                space,
                value: value.to_string(),
            },
        })
    }

    pub fn render(&self) -> String {
        let pad = " ".repeat(self.indent);
        match &self.kind {
            Kind::Blank => String::new(),
            Kind::Dash => format!("{pad}-"),
            Kind::Key { key, space, value } => {
                format!("{pad}{key}:{}{value}", if *space { " " } else { "" })
            }
        }
    }
}

/// A node of the block tree: a key or a list item, the lines it spans and its children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Its own line.
    pub line: usize,
    /// One past its last line (its children included).
    pub end: usize,
    pub indent: usize,
    /// None for a ` -` item.
    pub key: Option<String>,
    /// The raw value; empty for a block.
    pub value: String,
    pub children: Vec<Node>,
}

impl Node {
    /// The child with this key.
    pub fn child(&self, key: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.key.as_deref() == Some(key))
    }

    /// The raw value of the child with this key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.child(key).map(|c| c.value.as_str())
    }

    /// The node at this path of keys below this one.
    pub fn path(&self, path: &[&str]) -> Option<&Node> {
        let mut n = self;
        for k in path {
            n = n.child(k)?;
        }
        Some(n)
    }

    /// The lines of its body (children), without its own line.
    pub fn body(&self) -> Range<usize> {
        self.line + 1..self.end
    }

    /// The lines it spans, its own included.
    pub fn span(&self) -> Range<usize> {
        self.line..self.end
    }

    /// Its key as an index (`3:`), when it is one.
    pub fn index(&self) -> Option<u32> {
        self.key.as_deref().and_then(|k| k.parse().ok())
    }

    /// `(key, raw value)` of each scalar child, in file order.
    pub fn fields(&self) -> Vec<(String, String)> {
        self.children
            .iter()
            .filter_map(|c| Some((c.key.clone()?, c.value.clone())))
            .collect()
    }
}

/// An EdgeTX YAML file, as lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    /// The file name used in messages (`model01.yml`).
    pub name: String,
    lines: Vec<String>,
    /// Per line: it ended with CRLF.
    crlf: Vec<bool>,
    /// The last line had a line ending.
    final_eol: bool,
    /// The file's own ending, for new lines: CRLF when most lines use it.
    default_crlf: bool,
    raw: Vec<u8>,
}

/// "<file> line <n> is not understood; nothing was written."
pub fn shape_refusal(file: &str, line: usize, why: &str) -> Refusal {
    let why = if why.is_empty() {
        String::new()
    } else {
        format!(" ({why})")
    };
    Refusal::new(
        RefusalCode::ShapeUnknown,
        format!("{file} line {line} is not understood{why}; nothing was written."),
    )
}

/// Bytes as Latin-1 text: one `char` per byte.
pub fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|b| *b as char).collect()
}

/// Latin-1 text as bytes. A `char` above U+00FF has no byte: None.
pub fn to_latin1(s: &str) -> Option<Vec<u8>> {
    s.chars()
        .map(|c| u8::try_from(c as u32).ok())
        .collect::<Option<Vec<u8>>>()
}

impl Doc {
    /// Reads a file's bytes. Refuses a line it does not understand (`shape_unknown`) and a
    /// file that does not render back to the same bytes (`round_trip`).
    pub fn parse(name: &str, bytes: &[u8]) -> Result<Doc, Refusal> {
        let text = latin1(bytes);
        let mut lines = Vec::new();
        let mut crlf = Vec::new();
        let mut parts: Vec<&str> = text.split('\n').collect();
        let final_eol = parts.last() == Some(&"");
        if final_eol {
            parts.pop();
        }
        for p in parts {
            match p.strip_suffix('\r') {
                Some(l) => {
                    lines.push(l.to_string());
                    crlf.push(true);
                }
                None => {
                    lines.push(p.to_string());
                    crlf.push(false);
                }
            }
        }
        // The last line without an ending: its CR (if any) was part of the text.
        let crs = crlf.iter().filter(|c| **c).count();
        let doc = Doc {
            name: name.to_string(),
            default_crlf: crs * 2 >= crlf.len() && crs > 0,
            lines,
            crlf,
            final_eol,
            raw: bytes.to_vec(),
        };
        doc.tree()?;
        if doc.render() != bytes {
            return Err(Refusal::new(
                RefusalCode::RoundTrip,
                format!("QuadCam cannot rewrite {name} unchanged; nothing was written."),
            ));
        }
        Ok(doc)
    }

    /// The file's bytes as read.
    pub fn original(&self) -> &[u8] {
        &self.raw
    }

    /// The file's bytes now.
    pub fn render(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.raw.len() + 64);
        let n = self.lines.len();
        for (i, l) in self.lines.iter().enumerate() {
            out.extend(l.chars().map(|c| c as u32 as u8));
            if i + 1 < n || self.final_eol {
                if self.crlf[i] {
                    out.push(b'\r');
                }
                out.push(b'\n');
            }
        }
        out
    }

    /// True when the bytes differ from what was read.
    pub fn changed(&self) -> bool {
        self.render() != self.raw
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// True when the file uses CRLF line endings.
    pub fn is_crlf(&self) -> bool {
        self.default_crlf
    }

    /// The top-level nodes. Refuses a line it does not understand, a value line with
    /// children, and an indent that fits no block.
    pub fn tree(&self) -> Result<Vec<Node>, Refusal> {
        let parsed: Vec<Line> = self
            .lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                Line::parse(l)
                    .filter(|p| p.render() == *l)
                    .ok_or_else(|| shape_refusal(&self.name, i + 1, ""))
            })
            .collect::<Result<_, _>>()?;
        let mut i = 0;
        let nodes = self.children(&parsed, &mut i, None)?;
        if i < parsed.len() {
            return Err(shape_refusal(&self.name, i + 1, "indent"));
        }
        Ok(nodes)
    }

    /// Nodes from line `*i` while lines are deeper than `parent` (None: the top level).
    fn children(
        &self,
        lines: &[Line],
        i: &mut usize,
        parent: Option<usize>,
    ) -> Result<Vec<Node>, Refusal> {
        let mut out: Vec<Node> = Vec::new();
        let mut level: Option<usize> = None;
        while *i < lines.len() {
            let l = &lines[*i];
            if l.kind == Kind::Blank {
                *i += 1;
                continue;
            }
            if parent.is_some_and(|p| l.indent <= p) {
                break;
            }
            match level {
                None => {
                    if parent.is_none() && l.indent != 0 {
                        return Err(shape_refusal(&self.name, *i + 1, "indent"));
                    }
                    level = Some(l.indent);
                }
                Some(lv) if l.indent < lv => {
                    return Err(shape_refusal(&self.name, *i + 1, "indent"));
                }
                Some(lv) if l.indent > lv => {
                    return Err(shape_refusal(&self.name, *i + 1, "indent"));
                }
                _ => {}
            }
            let start = *i;
            *i += 1;
            let (key, value) = match &l.kind {
                Kind::Key { key, value, .. } => (Some(key.clone()), value.clone()),
                _ => (None, String::new()),
            };
            let children = self.children(lines, i, Some(l.indent))?;
            if !children.is_empty() && !value.is_empty() {
                return Err(shape_refusal(
                    &self.name,
                    start + 2,
                    "a value line cannot hold a block",
                ));
            }
            // A trailing blank line belongs to no block.
            let mut end = *i;
            while end > start + 1 && lines[end - 1].kind == Kind::Blank {
                end -= 1;
            }
            out.push(Node {
                line: start,
                end,
                indent: l.indent,
                key,
                value,
                children,
            });
        }
        Ok(out)
    }

    /// The top-level node with this key.
    pub fn top(&self, key: &str) -> Result<Option<Node>, Refusal> {
        Ok(self
            .tree()?
            .into_iter()
            .find(|n| n.key.as_deref() == Some(key)))
    }

    /// The node at this path of keys from the top.
    pub fn find(&self, path: &[&str]) -> Result<Option<Node>, Refusal> {
        let Some((first, rest)) = path.split_first() else {
            return Ok(None);
        };
        Ok(self.top(first)?.and_then(|n| n.path(rest).cloned()))
    }

    /// The raw value of a top-level scalar.
    pub fn top_value(&self, key: &str) -> Option<String> {
        self.top(key).ok().flatten().map(|n| n.value)
    }

    /// Replaces lines `range` with `new` (new lines take the file's line ending).
    pub fn splice(&mut self, range: Range<usize>, new: Vec<String>) {
        let n = new.len();
        let at_end = range.end == self.lines.len();
        self.lines.splice(range.clone(), new);
        self.crlf
            .splice(range, std::iter::repeat_n(self.default_crlf, n));
        if at_end && n > 0 && !self.final_eol && self.lines.len() > n {
            // Lines added after a last line that had no ending: that line now needs one.
            let i = self.lines.len() - n - 1;
            self.crlf[i] = self.default_crlf;
            self.final_eol = false;
        }
    }

    /// Replaces line `i` when it differs. True when it changed.
    pub fn set_line(&mut self, i: usize, text: String) -> bool {
        if self.lines[i] == text {
            return false;
        }
        self.lines[i] = text;
        true
    }

    /// Sets a top-level scalar that the file already has. Refuses a key the file lacks,
    /// or a block. True when it changed.
    pub fn set_top_scalar(&mut self, key: &str, value: &str) -> Result<bool, Refusal> {
        let n = self.top(key)?.ok_or_else(|| {
            Refusal::new(
                RefusalCode::ShapeUnknown,
                format!(
                    "{} has no {key} line; QuadCam does not add it; nothing was written.",
                    self.name
                ),
            )
        })?;
        if !n.children.is_empty() {
            return Err(Refusal::new(
                RefusalCode::ShapeUnknown,
                format!(
                    "{key} in {} is a block, not a value; nothing was written.",
                    self.name
                ),
            ));
        }
        Ok(self.set_line(n.line, format!("{key}: {value}")))
    }

    /// Sets `checksum: 0` when the file has a checksum line (EdgeTX accepts 0 or no line).
    pub fn zero_checksum(&mut self) -> bool {
        match self.top("checksum") {
            Ok(Some(n)) if n.value != "0" => self.set_line(n.line, "checksum: 0".into()),
            _ => false,
        }
    }

    /// The text of lines `range`.
    pub fn slice(&self, range: Range<usize>) -> &[String] {
        &self.lines[range]
    }
}

/// `"x"` -> `x`; anything else unchanged.
pub fn unquote(v: &str) -> &str {
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

/// `x` -> `"x"`.
pub fn quote(v: &str) -> String {
    format!("\"{v}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_classify_and_render() {
        for s in [
            "header: ",
            "   0:",
            "      name: \"A B\"",
            " -",
            "   destCh: 0",
            "",
            "semver: 2.12.4",
            "x: a: b",
        ] {
            let l = Line::parse(s).unwrap_or_else(|| panic!("{s:?}"));
            assert_eq!(l.render(), s);
        }
        for s in [
            "  ",
            "\tkey: 1",
            "# comment",
            "- a",
            "key:value",
            "\"q\": 1",
        ] {
            assert!(Line::parse(s).is_none(), "{s:?}");
        }
    }

    #[test]
    fn round_trip_keeps_crlf_lf_and_latin1() {
        for raw in [
            &b"a: 1\r\nb: \r\n   c: 2\r\n"[..],
            b"a: 1\nb: \n   c: 2\n",
            b"a: 1\nb: 2",
            b"a: \"\x80\xff\"\r\nb: 2\n",
            b"",
        ] {
            let d = Doc::parse("t.yml", raw).unwrap();
            assert_eq!(d.render(), raw);
            assert!(!d.changed());
        }
    }

    #[test]
    fn unknown_lines_refuse_with_file_and_line() {
        let e = Doc::parse("model01.yml", b"a: 1\r\nb: \r\n\tc: 2\r\n").unwrap_err();
        assert_eq!(e.code, RefusalCode::ShapeUnknown);
        assert_eq!(
            e.reason,
            "model01.yml line 3 is not understood; nothing was written."
        );
        let e = Doc::parse("m.yml", b"a: 1\n   b: 2\n").unwrap_err();
        assert!(
            e.reason.starts_with("m.yml line 2 is not understood"),
            "{e}"
        );
        let e = Doc::parse("m.yml", b"a: \n    b: 1\n  c: 2\n").unwrap_err();
        assert!(e.reason.starts_with("m.yml line 3"), "{e}");
    }

    #[test]
    fn tree_and_edits() {
        let raw = b"checksum: 123\r\nheader: \r\n   name: \"X\"\r\nmixData: \r\n -\r\n   destCh: 0\r\n   weight: 100\r\n -\r\n   destCh: 1\r\n   weight: 50\r\nlast: 1\r\n";
        let mut d = Doc::parse("m.yml", raw).unwrap();
        assert!(d.is_crlf());
        let mix = d.top("mixData").unwrap().unwrap();
        assert_eq!(mix.children.len(), 2);
        assert_eq!(mix.children[1].get("weight"), Some("50"));
        assert_eq!(mix.span(), 3..10);
        assert_eq!(d.find(&["header", "name"]).unwrap().unwrap().value, "\"X\"");
        assert!(d.set_top_scalar("last", "2").unwrap());
        assert!(!d.set_top_scalar("last", "2").unwrap());
        assert!(d.set_top_scalar("header", "1").is_err());
        assert!(d.set_top_scalar("nope", "1").is_err());
        assert!(d.zero_checksum());
        d.splice(10..10, vec!["added: 1".into()]);
        let out = d.render();
        assert!(out.ends_with(b"added: 1\r\nlast: 2\r\n"));
        assert!(out.starts_with(b"checksum: 0\r\n"));
    }

    #[test]
    fn appending_after_a_last_line_without_ending() {
        let mut d = Doc::parse("m.yml", b"a: 1\nb: 2").unwrap();
        d.splice(2..2, vec!["c: 3".into()]);
        assert_eq!(d.render(), b"a: 1\nb: 2\nc: 3");
    }
}
