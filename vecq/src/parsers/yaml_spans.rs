// PURPOSE:
//   Recover per-node line spans for YAML, which `serde_yaml_ng` discards.
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
//   yaml-rust exposes `MarkedEventReceiver`, whose `Marker` carries a real
//   line for every event. Its lines are 1-indexed at the source
//   (`Marker::new(0, 1, 0)` in scanner.rs), which is already the convention
//   every other vecq parser uses, so no adjustment is applied here — see
//   `tier1_line_index_convention.rs` for why that matters.
//
// WHY A SECOND PARSE:
//   yaml-rust is an event parser, not a data model: it will not give us the
//   tagged/merged/aliased `Value` that `serde_yaml_ng` produces and that the
//   `value` attribute needs. Running both and matching by key is cheaper than
//   reimplementing either, and matching by key rather than by position keeps it
//   correct regardless of either library's iteration order.
//
// A MEMBER'S SPAN STARTS AT ITS KEY and ends at the last line of its value,
// mirroring `json_spans`: a consumer replacing `image: qdrant/qdrant` means the
// whole pair.

use yaml_rust::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust::scanner::Marker;

/// A node's line range, plus its children keyed the way YAML addresses them.
#[derive(Debug, Clone, Default)]
pub(crate) struct YamlSpan {
    /// 1-indexed first line — the key token, for mapping members.
    pub start_line: usize,
    /// 1-indexed last line covered by this node.
    pub end_line: usize,
    pub fields: Vec<(String, YamlSpan)>,
    pub items: Vec<YamlSpan>,
}

impl YamlSpan {
    pub(crate) fn field(&self, key: &str) -> Option<&YamlSpan> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub(crate) fn item(&self, index: usize) -> Option<&YamlSpan> {
        self.items.get(index)
    }

    fn leaf(line: usize) -> Self {
        Self {
            start_line: line,
            end_line: line,
            fields: Vec::new(),
            items: Vec::new(),
        }
    }
}

enum Frame {
    /// `pending_key` holds the key scalar and its line until its value arrives.
    Mapping {
        start_line: usize,
        fields: Vec<(String, YamlSpan)>,
        pending_key: Option<(String, usize)>,
    },
    Sequence {
        start_line: usize,
        items: Vec<YamlSpan>,
    },
}

#[derive(Default)]
struct SpanBuilder {
    stack: Vec<Frame>,
    roots: Vec<YamlSpan>,
}

impl SpanBuilder {
    /// Attach a completed node to whatever contains it.
    fn emit(&mut self, node: YamlSpan, scalar_text: Option<String>) {
        match self.stack.last_mut() {
            Some(Frame::Mapping {
                fields,
                pending_key,
                ..
            }) => {
                if pending_key.is_none() {
                    // Mapping keys arrive as ordinary scalars. A non-scalar key
                    // (YAML permits them) has no name to match against
                    // serde_yaml_ng, so it is recorded as its rendered position
                    // and will simply not be found later — better than
                    // desynchronising the key/value alternation.
                    *pending_key = Some((scalar_text.unwrap_or_default(), node.start_line));
                } else {
                    let (key, key_line) = pending_key.take().expect("checked above");
                    fields.push((
                        key,
                        YamlSpan {
                            // The pair opens at its key, not at its value.
                            start_line: key_line,
                            end_line: node.end_line.max(key_line),
                            fields: node.fields,
                            items: node.items,
                        },
                    ));
                }
            }
            Some(Frame::Sequence { items, .. }) => items.push(node),
            None => self.roots.push(node),
        }
    }

    fn close(&mut self, frame: Frame) {
        let node = match frame {
            Frame::Mapping {
                start_line, fields, ..
            } => {
                // A container ends where its last child ends. The MappingEnd
                // marker is unreliable for this: block collections are
                // terminated by the *next* token, so its line can belong to a
                // sibling at lower indentation.
                let end = fields
                    .iter()
                    .map(|(_, v)| v.end_line)
                    .max()
                    .unwrap_or(start_line);
                YamlSpan {
                    start_line,
                    end_line: end.max(start_line),
                    fields,
                    items: Vec::new(),
                }
            }
            Frame::Sequence { start_line, items } => {
                let end = items.iter().map(|v| v.end_line).max().unwrap_or(start_line);
                YamlSpan {
                    start_line,
                    end_line: end.max(start_line),
                    fields: Vec::new(),
                    items,
                }
            }
        };
        self.emit(node, None);
    }
}

impl MarkedEventReceiver for SpanBuilder {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        match ev {
            Event::MappingStart(_) => self.stack.push(Frame::Mapping {
                start_line: mark.line(),
                fields: Vec::new(),
                pending_key: None,
            }),
            Event::SequenceStart(_) => self.stack.push(Frame::Sequence {
                start_line: mark.line(),
                items: Vec::new(),
            }),
            Event::MappingEnd | Event::SequenceEnd => {
                if let Some(frame) = self.stack.pop() {
                    self.close(frame);
                }
            }
            Event::Scalar(text, _, _, _) => {
                self.emit(YamlSpan::leaf(mark.line()), Some(text));
            }
            // An alias resolves elsewhere; its own position is still where it
            // appears, which is what an editor needs.
            Event::Alias(_) => self.emit(YamlSpan::leaf(mark.line()), None),
            _ => {}
        }
    }
}

/// Line spans for every top-level document in `content`, in document order.
///
/// `None` when the text does not scan. The document still parses through
/// `serde_yaml_ng`; only precise spans are unavailable, and the caller falls
/// back to a document-wide range rather than inventing one.
pub(crate) fn scan(content: &str) -> Option<Vec<YamlSpan>> {
    let mut builder = SpanBuilder::default();
    let mut parser = Parser::new(content.chars());
    parser.load(&mut builder, true).ok()?;

    if builder.roots.is_empty() {
        None
    } else {
        Some(builder.roots)
    }
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
