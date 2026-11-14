// PURPOSE:
//   Recover byte spans for every node in a JSON document.
//
// WHY THIS EXISTS:
//   `serde_json::Value` discards source positions, so `JsonParser` had nothing
//   to report and emitted the literal `1, 1` for every element in every file:
//
//       vecq cfg.json -q '.pairs[] | {name, line_start, line_end}'
//       {"line_end":1,"line_start":1,"name":"alpha"}
//       {"line_end":1,"line_start":1,"name":"gamma"}     # actually line 5
//
//   A consumer that resolves a node to a line range and rewrites those lines
//   therefore destroyed line 1 — usually the opening brace — and left the
//   intended target in place, producing invalid JSON and reporting success.
//   That is the mechanism behind the long-standing "edits result in duplicated
//   code" complaints.
//
//   This module re-scans the raw text alongside the parsed tree and hands back
//   real offsets, which `LineCounter` turns into real line numbers.
//
// WHY NOT REUSE THE serde_json PARSE:
//   `serde_json::Map` is a `BTreeMap` unless the `preserve_order` feature is on,
//   so its iteration order is alphabetical rather than document order. Zipping
//   the two trees positionally would silently mis-assign spans on any object
//   whose keys are not already sorted. Nodes are matched by key and by index
//   instead, which is order-independent.
//
// A MEMBER'S SPAN STARTS AT ITS KEY, not at its value: a consumer replacing
// `"theme": "dark"` means the whole pair, not just `"dark"`.
//
// DIALECT:
//   Accepts the same JSON5-ish input `JsonParser` already falls back to —
//   comments, trailing commas, single-quoted strings, unquoted keys — because
//   `tsconfig.json` and friends are the common case. If a scan cannot complete,
//   the caller keeps its previous behaviour rather than inventing a span.

/// A node's byte span, plus its children keyed the way JSON addresses them.
#[derive(Debug, Clone, Default)]
pub(crate) struct SpanNode {
    /// Byte offset where this node begins — the key token, for object members.
    pub start: usize,
    /// Byte offset one past this node's last byte.
    pub end: usize,
    /// Where the VALUE begins — `{` / `[` for a container. Equals `start`
    /// except for object members, whose span opens at the key.
    pub value_start: usize,
    /// Object members, in document order.
    pub fields: Vec<(String, SpanNode)>,
    /// Array elements, in document order.
    pub items: Vec<SpanNode>,
    /// Comments that belong to this node and lie OUTSIDE its span: those on
    /// their own lines before it (leading), and those after it on the same line,
    /// or before the closing bracket when it is the last member (trailing).
    /// Each is a byte range from the first comment's start to the last one's
    /// end. Set by `attach_comments`; `None` when there are none.
    pub leading: Option<(usize, usize)>,
    pub trailing: Option<(usize, usize)>,
}

impl SpanNode {
    pub(crate) fn field(&self, key: &str) -> Option<&SpanNode> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub(crate) fn item(&self, index: usize) -> Option<&SpanNode> {
        self.items.get(index)
    }
}

struct Scanner<'a> {
    b: &'a [u8],
    i: usize,
}

/// Scan every top-level value in `content`, in document order.
///
/// Returns `None` if the text does not scan cleanly. A `None` is not an error
/// worth surfacing — the document still parses through `serde_json`; only the
/// spans are unavailable, and inventing one is worse than omitting it.
pub(crate) fn scan(content: &str) -> Option<Vec<SpanNode>> {
    let mut s = Scanner {
        b: content.as_bytes(),
        i: 0,
    };
    let mut roots = Vec::new();

    s.trivia();
    while s.i < s.b.len() {
        roots.push(s.value()?);
        s.trivia();
    }

    if roots.is_empty() {
        None
    } else {
        attach_comments(&mut roots, content.as_bytes(), 0, content.len());
        Some(roots)
    }
}

/// Comment byte ranges in a gap between values. A gap holds only whitespace,
/// separators and comments — the scan has already proved that.
fn comments_in(b: &[u8], from: usize, to: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = from;
    while i < to {
        match (b[i], b.get(i + 1)) {
            (b'/', Some(b'/')) => {
                let start = i;
                while i < to && b[i] != b'\n' {
                    i += 1;
                }
                out.push((start, i));
            }
            (b'/', Some(b'*')) => {
                let start = i;
                i += 2;
                while i + 1 < to && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(to);
                out.push((start, i));
            }
            _ => i += 1,
        }
    }
    out
}

/// Byte spans of every comment in a scanned document, in order — the gaps
/// `attach_comments` reads, at every depth. Comments inside strings are never
/// seen: only gaps between values are read.
pub(crate) fn all_comments(roots: &[SpanNode], b: &[u8]) -> Vec<(usize, usize)> {
    fn walk(
        siblings: &[&SpanNode],
        b: &[u8],
        open: usize,
        close: usize,
        out: &mut Vec<(usize, usize)>,
    ) {
        let n = siblings.len();
        for i in 0..=n {
            let from = if i == 0 { open } else { siblings[i - 1].end };
            let to = if i == n { close } else { siblings[i].start };
            if from < to {
                out.extend(comments_in(b, from, to));
            }
        }
        for node in siblings {
            let (open, close) = (node.value_start + 1, node.end.saturating_sub(1));
            let kids: Vec<&SpanNode> = if node.fields.is_empty() {
                node.items.iter().collect()
            } else {
                node.fields.iter().map(|(_, v)| v).collect()
            };
            if !kids.is_empty() {
                walk(&kids, b, open, close, out);
            } else if open < close
                && (b.get(node.value_start) == Some(&b'{')
                    || b.get(node.value_start) == Some(&b'['))
            {
                // An empty container may still hold a comment: `{ /* none */ }`.
                out.extend(comments_in(b, open, close));
            }
        }
    }
    let mut out = Vec::new();
    walk(&roots.iter().collect::<Vec<_>>(), b, 0, b.len(), &mut out);
    out.sort_unstable();
    out
}

fn span_of(ranges: &[(usize, usize)]) -> Option<(usize, usize)> {
    Some((ranges.first()?.0, ranges.last()?.1))
}

/// Give every comment an owner among `siblings`, the values between `open`
/// and `close` (exclusive interior). The rule is the one a reader applies, and
/// the same in every language: a comment on its own line describes what
/// FOLLOWS it; one after a value on the same line describes that value; one
/// before the closing bracket belongs to the last member.
///
/// Without this a JSONC comment — usually the only place a setting's REASON is
/// written — sat outside every span and was never embedded.
fn attach_comments(siblings: &mut [SpanNode], b: &[u8], open: usize, close: usize) {
    // The comments BETWEEN siblings (those inside a sibling's value belong to
    // that value's own members, handled by the recursion below).
    let n = siblings.len();
    let mut comments = Vec::new();
    for i in 0..=n {
        let gap_start = if i == 0 { open } else { siblings[i - 1].end };
        let gap_end = if i == n { close } else { siblings[i].start };
        comments.extend(comments_in(b, gap_start, gap_end));
    }
    // Owners by the one shared rule — `super::ownership::comment_owner`.
    let items: Vec<(usize, usize)> = siblings.iter().map(|s| (s.start, s.end)).collect();
    let mut leading: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    let mut trailing: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for c in comments {
        match super::ownership::comment_owner(b, &items, c) {
            Some(super::ownership::Owner::Next(i)) => leading[i].push(c),
            Some(super::ownership::Owner::Previous(i)) => trailing[i].push(c),
            None => {}
        }
    }
    for (i, node) in siblings.iter_mut().enumerate() {
        node.leading = span_of(&leading[i]);
        node.trailing = span_of(&trailing[i]);
    }
    for node in siblings.iter_mut() {
        // Interior of a container: after its opening bracket, before its closing one.
        let (open, close) = (node.value_start + 1, node.end.saturating_sub(1));
        if !node.fields.is_empty() {
            let mut members: Vec<SpanNode> = node.fields.iter().map(|(_, v)| v.clone()).collect();
            attach_comments(&mut members, b, open, close);
            for ((_, v), m) in node.fields.iter_mut().zip(members) {
                *v = m;
            }
        } else if !node.items.is_empty() {
            attach_comments(&mut node.items, b, open, close);
        }
    }
}

impl<'a> Scanner<'a> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    /// Whitespace plus `//` line and `/* */` block comments.
    fn trivia(&mut self) {
        loop {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.i += 1;
            }
            match (self.peek(), self.b.get(self.i + 1)) {
                (Some(b'/'), Some(b'/')) => {
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                (Some(b'/'), Some(b'*')) => {
                    self.i += 2;
                    while self.i + 1 < self.b.len()
                        && !(self.b[self.i] == b'*' && self.b[self.i + 1] == b'/')
                    {
                        self.i += 1;
                    }
                    // Unterminated block comment: consume the rest rather than
                    // spinning. The scan will fail on the next real token.
                    self.i = (self.i + 2).min(self.b.len());
                }
                _ => return,
            }
        }
    }

    fn value(&mut self) -> Option<SpanNode> {
        self.trivia();
        let start = self.i;
        match self.peek()? {
            b'{' => self.object(start),
            b'[' => self.array(start),
            b'"' | b'\'' => {
                self.string()?;
                Some(self.leaf(start))
            }
            _ => {
                // Number, true/false/null, or a bare JSON5 token. Consume until
                // a structural byte; the parsed value comes from serde_json, so
                // this only has to find the right end offset.
                while let Some(c) = self.peek() {
                    if matches!(c, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                        break;
                    }
                    self.i += 1;
                }
                if self.i == start {
                    return None;
                }
                Some(self.leaf(start))
            }
        }
    }

    fn leaf(&self, start: usize) -> SpanNode {
        SpanNode {
            start,
            end: self.i,
            fields: Vec::new(),
            value_start: start,
            leading: None,
            trailing: None,
            items: Vec::new(),
        }
    }

    fn object(&mut self, start: usize) -> Option<SpanNode> {
        self.i += 1; // '{'
        let mut fields = Vec::new();

        loop {
            self.trivia();
            match self.peek()? {
                b'}' => {
                    self.i += 1;
                    break;
                }
                b',' => {
                    // Tolerates both separators and trailing commas.
                    self.i += 1;
                    continue;
                }
                _ => {}
            }

            // The member's span opens at the key.
            let key_start = self.i;
            let key = match self.peek()? {
                b'"' | b'\'' => self.string()?,
                _ => self.bare_key()?,
            };

            self.trivia();
            if self.peek()? != b':' {
                return None;
            }
            self.i += 1;

            let mut node = self.value()?;
            node.start = key_start;
            fields.push((key, node));
        }

        Some(SpanNode {
            start,
            end: self.i,
            fields,
            value_start: start,
            leading: None,
            trailing: None,
            items: Vec::new(),
        })
    }

    fn array(&mut self, start: usize) -> Option<SpanNode> {
        self.i += 1; // '['
        let mut items = Vec::new();

        loop {
            self.trivia();
            match self.peek()? {
                b']' => {
                    self.i += 1;
                    break;
                }
                b',' => {
                    self.i += 1;
                    continue;
                }
                _ => {}
            }
            items.push(self.value()?);
        }

        Some(SpanNode {
            start,
            end: self.i,
            fields: Vec::new(),
            value_start: start,
            leading: None,
            trailing: None,
            items,
        })
    }

    /// A quoted string, returning its decoded-enough-to-match contents.
    ///
    /// Only `\"`, `\'` and `\\` need real handling: the result is used to match
    /// against `serde_json`'s keys, and any other escape passes through
    /// unchanged in both.
    fn string(&mut self) -> Option<String> {
        let quote = self.peek()?;
        self.i += 1;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c == b'\\' {
                let next = *self.b.get(self.i + 1)?;
                match next {
                    b'"' => out.push('"'),
                    b'\'' => out.push('\''),
                    b'\\' => out.push('\\'),
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    other => {
                        out.push('\\');
                        out.push(other as char);
                    }
                }
                self.i += 2;
                continue;
            }
            if c == quote {
                self.i += 1;
                return Some(out);
            }
            // Multi-byte UTF-8 passes through as bytes and is reassembled below.
            let ch_start = self.i;
            self.i += 1;
            while self.i < self.b.len() && (self.b[self.i] & 0xC0) == 0x80 {
                self.i += 1;
            }
            out.push_str(std::str::from_utf8(&self.b[ch_start..self.i]).ok()?);
        }
        None
    }

    /// An unquoted JSON5 object key.
    fn bare_key(&mut self) -> Option<String> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' {
                self.i += 1;
            } else {
                break;
            }
        }
        if self.i == start {
            return None;
        }
        std::str::from_utf8(&self.b[start..self.i])
            .ok()
            .map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_span_starts_at_its_key_not_its_value() {
        let src = "{\n  \"theme\": \"dark\"\n}\n";
        let roots = scan(src).expect("scan");
        let theme = roots[0].field("theme").expect("theme");
        assert_eq!(&src[theme.start..theme.end], "\"theme\": \"dark\"");
    }

    #[test]
    fn nested_objects_report_their_own_spans() {
        let src = "{\n  \"a\": 1,\n  \"b\": {\n    \"c\": 2\n  }\n}\n";
        let roots = scan(src).expect("scan");
        let c = roots[0].field("b").unwrap().field("c").unwrap();
        assert_eq!(&src[c.start..c.end], "\"c\": 2");
    }

    #[test]
    fn array_items_are_addressable_by_index() {
        let src = "{\n  \"xs\": [\n    10,\n    20\n  ]\n}\n";
        let roots = scan(src).expect("scan");
        let xs = roots[0].field("xs").unwrap();
        assert_eq!(
            &src[xs.item(1).unwrap().start..xs.item(1).unwrap().end],
            "20"
        );
    }

    /// `tsconfig.json` and friends. The strict parse fails on these, so the
    /// span scan has to accept what the JSON5 fallback accepts.
    #[test]
    fn comments_and_trailing_commas_scan() {
        let src = "{\n  // a comment\n  \"a\": 1,\n  /* block */\n  \"b\": 2,\n}\n";
        let roots = scan(src).expect("scan");
        let b = roots[0].field("b").expect("b");
        assert_eq!(&src[b.start..b.end], "\"b\": 2");
    }

    #[test]
    fn every_comment_has_exactly_one_owner() {
        let src = "{\n  // why a\n  \"a\": 1, // about a\n  /* why b */\n  \"b\": { \"c\": 2 /* about c */ },\n  // dangling\n}\n";
        let roots = scan(src).expect("scan");
        let text = |r: Option<(usize, usize)>| r.map(|(s, e)| &src[s..e]);
        let a = roots[0].field("a").unwrap();
        let b = roots[0].field("b").unwrap();
        let c = b.field("c").unwrap();
        assert_eq!(text(a.leading), Some("// why a"));
        assert_eq!(text(a.trailing), Some("// about a"));
        assert_eq!(text(b.leading), Some("/* why b */"));
        assert_eq!(text(b.trailing), Some("// dangling"));
        assert_eq!(text(c.trailing), Some("/* about c */"));
    }

    #[test]
    fn jsonl_yields_one_root_per_line() {
        let src = "{\"a\": 1}\n{\"a\": 2}\n";
        let roots = scan(src).expect("scan");
        assert_eq!(roots.len(), 2);
        assert_eq!(&src[roots[1].start..roots[1].end], "{\"a\": 2}");
    }

    #[test]
    fn a_malformed_document_yields_no_spans_rather_than_wrong_ones() {
        assert!(scan("{\"a\": ").is_none());
    }
}
