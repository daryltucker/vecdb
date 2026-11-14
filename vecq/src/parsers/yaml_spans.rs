// PURPOSE:
//   Recover per-node byte spans for YAML, which `serde_yaml_ng` discards.
//
// WHY THIS EXISTS:
//   `serde_yaml_ng::Value` carries no source positions, so `YamlParser` gave
//   every element in a file the same document-wide range. Before day 238 that
//   range was `0..line_count-1` — 0-indexed, so every YAML node in every
//   collection reported a line that does not exist in any editor. Making it
//   1-indexed fixed the *convention* but not the *precision*: a 200-line
//   docker-compose still reported `1..200` for every key, which is true and
//   useless. A consumer resolving a node to a line range got the whole file.
//
//   Day 238 recovered LINES from yaml-rust's event `Marker`s. That was still
//   not a span: yaml-rust marks only where a node starts — in characters, not
//   bytes — and never where it ends, so a quoted or block scalar's extent could
//   not be known and YAML elements carried no span at all.
//   yaml-rust is also unmaintained (RUSTSEC-2024-0320); the
//   recorded direction was to replace it, not bump it.
//   `saphyr-parser`, its maintained
//   YAML 1.2 successor, gives every event a `Span` with an exclusive end.
//   Its markers count characters; they are mapped to byte offsets here.
//
// WHY A SECOND PARSE:
//   saphyr-parser is an event parser, not a data model: it will not give us the
//   tagged/merged/aliased `Value` that `serde_yaml_ng` produces and that the
//   `value` attribute needs. Running both and matching by key is cheaper than
//   reimplementing either, and matching by key rather than by position keeps it
//   correct regardless of either library's iteration order.
//
// A MEMBER'S SPAN STARTS AT ITS KEY and ends at the end of its value,
// mirroring `json_spans`: a consumer replacing `image: qdrant/qdrant` means the
// whole pair. Lines are 1-indexed and inclusive (`LineCounter::line_range`) —
// see `tier1_line_index_convention.rs` for why that matters.

use saphyr_parser::{Event, Parser, Span, SpannedEventReceiver};
use vecdb_common::LineCounter;

/// A node's byte span and line range, plus its children keyed the way YAML
/// addresses them.
#[derive(Debug, Clone, Default)]
pub(crate) struct YamlSpan {
    /// Byte offset of the node's first byte — the key, for mapping members;
    /// its anchor or tag, when it has one.
    pub start: usize,
    /// Byte offset one past the node's last byte.
    pub end: usize,
    /// 1-indexed first line.
    pub start_line: usize,
    /// 1-indexed last line covered by this node (inclusive).
    pub end_line: usize,
    pub fields: Vec<(String, YamlSpan)>,
    pub items: Vec<YamlSpan>,
    /// The node is an alias (`*base`): its text is the alias, its value lives
    /// at the anchor. Its value's members are not written here.
    pub alias: bool,
}

impl YamlSpan {
    pub(crate) fn field(&self, key: &str) -> Option<&YamlSpan> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub(crate) fn item(&self, index: usize) -> Option<&YamlSpan> {
        self.items.get(index)
    }

    fn node(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            ..Self::default()
        }
    }

    fn set_lines(&mut self, lines: &LineCounter) {
        (self.start_line, self.end_line) = lines.line_range(self.start, self.end);
        for (_, f) in self.fields.iter_mut() {
            f.set_lines(lines);
        }
        for i in self.items.iter_mut() {
            i.set_lines(lines);
        }
    }
}

enum Frame {
    /// `pending_key` holds the key's text and span until its value arrives.
    Mapping {
        start: usize,
        fields: Vec<(String, YamlSpan)>,
        pending_key: Option<(String, usize, usize)>,
    },
    Sequence {
        start: usize,
        items: Vec<YamlSpan>,
    },
}

struct SpanBuilder<'s> {
    source: &'s str,
    /// Byte offset of each character index (and of the end).
    byte_at: Vec<usize>,
    stack: Vec<Frame>,
    roots: Vec<YamlSpan>,
}

impl SpanBuilder<'_> {
    fn bytes(&self, span: Span) -> (usize, usize) {
        let at = |i: usize| self.byte_at.get(i).copied().unwrap_or(self.source.len());
        (at(span.start.index()), at(span.end.index()))
    }

    /// Widen a node's start over the properties written before it on its line
    /// — an anchor (`&base`) or a tag (`!!str`, `!custom`). The grammar puts a
    /// node's properties immediately before its content, and the events carry
    /// no span for them; left out, they would be text no element owns.
    fn with_properties(&self, start: usize) -> usize {
        let b = self.source.as_bytes();
        let mut at = start;
        loop {
            let mut p = at;
            while p > 0 && matches!(b[p - 1], b' ' | b'\t') {
                p -= 1;
            }
            let token_end = p;
            while p > 0
                && !matches!(
                    b[p - 1],
                    b' ' | b'\t' | b'\n' | b'\r' | b':' | b'-' | b'[' | b'{' | b','
                )
            {
                p -= 1;
            }
            if p < token_end && matches!(b[p], b'&' | b'!') {
                at = p;
            } else {
                return at;
            }
        }
    }

    /// Attach a completed node to whatever contains it.
    fn emit(&mut self, node: YamlSpan, key_text: Option<String>) {
        match self.stack.last_mut() {
            Some(Frame::Mapping {
                fields,
                pending_key,
                ..
            }) => match pending_key.take() {
                // Mapping keys arrive as ordinary nodes. A non-scalar key (YAML
                // permits them) has no name to match against serde_yaml_ng, so
                // it is recorded with an empty name and will simply not be found
                // later — better than desynchronising the key/value alternation.
                None => *pending_key = Some((key_text.unwrap_or_default(), node.start, node.end)),
                Some((key, key_start, key_end)) => {
                    // An empty value (`key:`) has an empty span somewhere after
                    // the key — possibly on a later line. The pair then ends at
                    // its own `:` indicator, which is part of the pair as
                    // written (`empty:`), not of whatever follows.
                    let end = if node.end > node.start {
                        node.end.max(key_end)
                    } else {
                        let b = self.source.as_bytes();
                        let mut p = key_end;
                        while p < b.len() && matches!(b[p], b' ' | b'\t') {
                            p += 1;
                        }
                        if b.get(p) == Some(&b':') {
                            p + 1
                        } else {
                            key_end
                        }
                    };
                    fields.push((
                        key,
                        YamlSpan {
                            // The pair opens at its key, not at its value.
                            start: key_start,
                            end,
                            start_line: 0,
                            end_line: 0,
                            fields: node.fields,
                            items: node.items,
                            alias: node.alias,
                        },
                    ));
                }
            },
            Some(Frame::Sequence { items, .. }) => items.push(node),
            None => self.roots.push(node),
        }
    }

    /// Close a container. A block collection ends where its last child ends —
    /// its End event is empty, positioned at the NEXT token, which can belong
    /// to a sibling at lower indentation. A flow collection (`{…}`, `[…]`)
    /// ends at its closing bracket, which its End event spans.
    fn close(&mut self, frame: Frame, end_event: (usize, usize)) {
        let flow_end = (end_event.1 > end_event.0).then_some(end_event.1);
        let node = match frame {
            Frame::Mapping { start, fields, .. } => {
                let last = fields.iter().map(|(_, v)| v.end).max();
                let start = fields.first().map_or(start, |(_, f)| f.start.min(start));
                let end = flow_end.or(last).unwrap_or(start).max(start);
                YamlSpan {
                    fields,
                    ..YamlSpan::node(start, end)
                }
            }
            Frame::Sequence { start, items } => {
                let last = items.iter().map(|v| v.end).max();
                let end = flow_end.or(last).unwrap_or(start).max(start);
                YamlSpan {
                    items,
                    ..YamlSpan::node(start, end)
                }
            }
        };
        self.emit(node, None);
    }
}

impl<'input> SpannedEventReceiver<'input> for SpanBuilder<'_> {
    fn on_event(&mut self, ev: Event<'input>, span: Span) {
        let (s, e) = self.bytes(span);
        match ev {
            Event::MappingStart(..) => {
                let start = self.with_properties(s);
                self.stack.push(Frame::Mapping {
                    start,
                    fields: Vec::new(),
                    pending_key: None,
                })
            }
            Event::SequenceStart(..) => {
                let start = self.with_properties(s);
                self.stack.push(Frame::Sequence {
                    start,
                    items: Vec::new(),
                })
            }
            Event::MappingEnd | Event::SequenceEnd => {
                if let Some(frame) = self.stack.pop() {
                    self.close(frame, (s, e));
                }
            }
            Event::Scalar(text, ..) => {
                let start = if e > s { self.with_properties(s) } else { s };
                self.emit(YamlSpan::node(start, e), Some(text.into_owned()));
            }
            // An alias resolves elsewhere; its own position is still where it
            // appears, which is what an editor needs.
            Event::Alias(_) => self.emit(
                YamlSpan {
                    alias: true,
                    ..YamlSpan::node(s, e)
                },
                None,
            ),
            _ => {}
        }
    }
}

/// Spans for every top-level document in `content`, in document order.
///
/// `None` when the text does not scan. The document still parses through
/// `serde_yaml_ng`; only spans are unavailable, and nodes then carry none
/// rather than an invented one.
pub(crate) fn scan(content: &str) -> Option<Vec<YamlSpan>> {
    let mut byte_at: Vec<usize> = content.char_indices().map(|(b, _)| b).collect();
    byte_at.push(content.len());
    let mut builder = SpanBuilder {
        source: content,
        byte_at,
        stack: Vec::new(),
        roots: Vec::new(),
    };
    let mut parser = Parser::new_from_str(content);
    parser.load(&mut builder, true).ok()?;

    if builder.roots.is_empty() {
        return None;
    }
    let lines = LineCounter::new(content);
    for root in builder.roots.iter_mut() {
        root.set_lines(&lines);
    }
    Some(builder.roots)
}

/// Byte spans of the `#` comments in the text no node covers. Outside scalars
/// YAML has only indicators, whitespace and comments, and a comment starts at
/// a `#` preceded by whitespace (or at a line start) and runs to the end of
/// its line.
pub(crate) fn comments(source: &str, covered: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let b = source.as_bytes();
    let mut out = Vec::new();
    for (from, to) in super::ownership::uncovered((0, source.len()), covered) {
        let mut i = from;
        while i < to {
            let starts = b[i] == b'#' && (i == 0 || b[i - 1].is_ascii_whitespace());
            if !starts {
                i += 1;
                continue;
            }
            let start = i;
            while i < to && b[i] != b'\n' {
                i += 1;
            }
            let end = if b[i - 1] == b'\r' { i - 1 } else { i };
            out.push((start, end));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPOSE: &str = "\
version: \"3\"
services:
  qdrant:
    image: qdrant/qdrant
    ports:
      - 6333
      - 6334
";

    #[test]
    fn a_scalar_member_spans_its_own_line() {
        let roots = scan(COMPOSE).expect("scan");
        let version = roots[0].field("version").expect("version");
        assert_eq!((version.start_line, version.end_line), (1, 1));
    }

    /// The whole point: a nested key reports ITS line, not the document's.
    #[test]
    fn a_nested_member_reports_its_own_line() {
        let roots = scan(COMPOSE).expect("scan");
        let image = roots[0]
            .field("services")
            .unwrap()
            .field("qdrant")
            .unwrap()
            .field("image")
            .expect("image");
        assert_eq!((image.start_line, image.end_line), (4, 4));
    }

    /// A container covers its children — opening at its key, closing at the
    /// last line of its last child rather than at the next sibling's line.
    #[test]
    fn a_container_spans_from_its_key_to_its_last_child() {
        let roots = scan(COMPOSE).expect("scan");
        let services = roots[0].field("services").expect("services");
        assert_eq!((services.start_line, services.end_line), (2, 7));
    }

    #[test]
    fn sequence_items_are_addressable_by_index() {
        let roots = scan(COMPOSE).expect("scan");
        let ports = roots[0]
            .field("services")
            .unwrap()
            .field("qdrant")
            .unwrap()
            .field("ports")
            .expect("ports");
        assert_eq!(ports.item(0).unwrap().start_line, 6);
        assert_eq!(ports.item(1).unwrap().start_line, 7);
    }

    /// Kubernetes manifests and CI configs routinely hold several documents.
    #[test]
    fn each_document_keeps_its_own_line_numbers() {
        let src = "a: 1\n---\nb: 2\n";
        let roots = scan(src).expect("scan");
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].field("a").unwrap().start_line, 1);
        assert_eq!(roots[1].field("b").unwrap().start_line, 3);
    }

    #[test]
    fn a_malformed_document_yields_no_spans_rather_than_wrong_ones() {
        assert!(scan("a:\n  - [unclosed\n").is_none());
    }
}
