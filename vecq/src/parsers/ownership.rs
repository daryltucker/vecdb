// PURPOSE:
//   Which element a comment belongs to — one rule, every language.
//
// WHY THIS EXISTS:
//   The rule was first written inside the JSON scanner (`json_spans`), so only
//   JSONC comments had owners; tree-sitter languages emitted comments as loose
//   elements that the chunker placed by position — a comment describing a
//   function could close the PREVIOUS chunk. The rule never depended on JSON:
//   it needs only the siblings' byte spans and the source text. One
//   implementation, used by every parser that emits comments.
//
// THE RULE (the one a reader applies):
//   - a comment after an item on the SAME line describes that item (trailing);
//   - a comment on its own line describes what FOLLOWS it (leading);
//   - a comment with nothing after it belongs to the last item before it.
use crate::types::{attr, DocumentElement, ElementType};
use vecdb_common::{has_meaning, LineCounter};

/// The owner of a comment, as an index into the items it was resolved against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// The comment precedes `items[i]` and describes it.
    Next(usize),
    /// The comment follows `items[i]` (same line, or nothing follows).
    Previous(usize),
}

/// Resolve the owner of the comment at `comment` among `items` — the spans of
/// the non-comment siblings, sorted by start. `None` when there are no items.
pub fn comment_owner(
    source: &[u8],
    items: &[(usize, usize)],
    comment: (usize, usize),
) -> Option<Owner> {
    let prev = items.iter().rposition(|&(_, end)| end <= comment.0);
    let next = items.iter().position(|&(start, _)| start >= comment.1);
    if let Some(p) = prev {
        let between = source.get(items[p].1..comment.0).unwrap_or_default();
        if !between.contains(&b'\n') {
            return Some(Owner::Previous(p));
        }
    }
    match (next, prev) {
        (Some(n), _) => Some(Owner::Next(n)),
        (None, Some(p)) => Some(Owner::Previous(p)),
        (None, None) => None,
    }
}

/// Split the text BETWEEN two elements by the same rule, at byte level:
/// everything up to and including the first newline trails the element before
/// (the rest of its line — a same-line comment and its line break); the rest
/// leads the element after (blank lines and own-line comments describing it).
/// Returns `(trailing_of_previous, leading_of_next)`; concatenated they are
/// `gap`, so no byte is lost or counted twice.
pub fn split_gap(gap: &str) -> (&str, &str) {
    match gap.find('\n') {
        Some(i) => gap.split_at(i + 1),
        None => (gap, ""),
    }
}

/// The parts of `range` that no span in `covered` overlaps, in order.
pub(crate) fn uncovered((s, e): (usize, usize), covered: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut spans: Vec<(usize, usize)> = covered
        .iter()
        .filter(|&&(cs, ce)| cs < e && s < ce)
        .map(|&(cs, ce)| (cs.max(s), ce.min(e)))
        .collect();
    spans.sort_unstable();
    let mut gaps = Vec::new();
    let mut at = s;
    for (cs, ce) in spans {
        if cs > at {
            gaps.push((at, cs));
        }
        at = at.max(ce);
    }
    if e > at {
        gaps.push((at, e));
    }
    gaps
}

/// Make the top-level elements LOSSLESS: every byte between two of them —
/// whitespace, blank lines, comments — becomes trivia owned by one of them
/// (`SourceInfo::leading` / `trailing`), by the one byte rule
/// (`split_gap`). Concatenating each element's leading +
/// text + trailing, in order, then reproduces the file exactly.
///
/// `is_top` selects the elements that own trivia — every element for the
/// tree-sitter languages, depth-0 blocks for Markdown (whose element list is
/// flat and includes nested blocks).
///
/// Comments stop being loose elements here: a comment is trivia of the element
/// it describes, so it is embedded with that element and AST consumers see
/// the same element set as before. Only a file with nothing BUT comments keeps
/// them as elements (there is no owner).
///
/// Trivia may hold only whitespace, punctuation and comments. Anything else in
/// a gap is a construct the coverage pass missed — it must never be absorbed
/// silently: tests fail on it (`debug_assert`), and a release build still
/// emits it as a visible `uncovered` element rather than hiding it.
pub(crate) fn assign_trivia(
    elements: &mut Vec<DocumentElement>,
    source: &str,
    lines: &LineCounter,
    comments: &[(usize, usize)],
    is_top: impl Fn(&DocumentElement) -> bool + Copy,
) {
    // Group overlapping top-level spans (some walkers give several elements one
    // span — Go's grouped imports, Python's import aliases); trivia belongs to
    // the first element of each group.
    let mut order: Vec<usize> = (0..elements.len())
        .filter(|&i| is_top(&elements[i]) && elements[i].source_span().is_some())
        .collect();
    order.sort_by_key(|&i| elements[i].source_span().map(|s| s.0));
    let mut groups: Vec<(usize, usize, usize)> = Vec::new(); // (owner index, start, end)
    for i in order {
        let (s, e) = elements[i].source_span().expect("filtered above");
        match groups.last_mut() {
            Some(g) if s < g.2 => g.2 = g.2.max(e),
            _ => groups.push((i, s, e)),
        }
    }

    // A gap's meaning outside comments is a coverage miss: surface it.
    let mut stray = Vec::new();
    let mut check = |gs: usize, ge: usize| {
        for (a, b) in uncovered((gs, ge), comments) {
            if let Some(text) = source.get(a..b) {
                if has_meaning(text) {
                    debug_assert!(false, "uncovered text left for trivia: {text:?}");
                    let t = text.trim();
                    let start = a + (text.len() - text.trim_start().len());
                    let end = start + t.len();
                    let mut el = DocumentElement::new(
                        ElementType::Block,
                        None,
                        t.to_string(),
                        lines.get_line_number(start),
                        lines.get_line_number(end.saturating_sub(1)),
                    )
                    .with_attribute(attr::KIND.to_string(), "uncovered");
                    el.set_source_span(start, end);
                    stray.push(el);
                }
            }
        }
    };
    let mut prev_end = 0;
    for &(_, s, e) in &groups {
        check(prev_end, s);
        prev_end = e;
    }
    check(prev_end, source.len());
    if !stray.is_empty() {
        elements.extend(stray);
        elements.sort_by_key(|e| e.line_start);
        return assign_trivia(elements, source, lines, comments, is_top);
    }

    // Partition every gap by the shared rule.
    let mut leading = vec![String::new(); elements.len()];
    let mut trailing = vec![String::new(); elements.len()];
    let mut prev: Option<(usize, usize)> = None; // (owner, end)
    for &(owner, s, e) in &groups {
        let from = prev.map_or(0, |p| p.1);
        let gap = source.get(from..s).unwrap_or_default();
        match prev {
            None => leading[owner] = gap.to_string(),
            Some((p, _)) => {
                let (t, l) = split_gap(gap);
                trailing[p] = t.to_string();
                leading[owner] = l.to_string();
            }
        }
        prev = Some((owner, e));
    }
    if let Some((p, end)) = prev {
        trailing[p] = source.get(end..).unwrap_or_default().to_string();
    }
    for (i, el) in elements.iter_mut().enumerate() {
        el.set_trivia(
            std::mem::take(&mut leading[i]),
            std::mem::take(&mut trailing[i]),
        );
    }
}

/// Give every child of `parent` — and theirs, recursively — the trivia it owns
/// INSIDE its container, by the same reading rule as the top level, with one
/// addition: a container has text of its own (its frame — `impl S {`, a class
/// header, a JSON member's `"key": {`, the closing `}`), and that text never
/// becomes a child's trivia.
///
/// Why: only top-level elements
/// owned trivia, so a method's doc comment stayed in its `impl`'s frame — it
/// was embedded with the impl, not the method, and an editor deleting the
/// method left its comment behind.
///
/// For each gap between children (and before the first, after the last):
/// - **trailing** of the child before: the rest of its line — a separator
///   (`,` `;`), a same-line comment, the line break. After the LAST child it
///   also takes own-line comments with nothing but the closing text after
///   them (a comment with nothing after it belongs to the last item before it).
/// - **leading** of the child after: from the line after the gap's last word
///   of container text (or after the child before) to the child — blank
///   lines, indentation and the comments that describe it.
/// - the rest is the container's frame.
///
/// `comments` are the byte spans of every comment in the file, sorted; text
/// inside them is never mistaken for container text. Children that overlap an
/// earlier sibling's span own nothing (shared spans).
pub(crate) fn assign_nested_trivia(
    parent: &mut DocumentElement,
    source: &str,
    comments: &[(usize, usize)],
) {
    let Some((lo, hi)) = parent.source_span() else {
        return;
    };
    let mut order: Vec<(usize, usize, usize)> = parent
        .children
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.source_span().map(|(s, e)| (s, e, i)))
        .filter(|&(s, e, _)| lo <= s && e <= hi)
        .collect();
    order.sort_unstable();
    let mut kept: Vec<(usize, usize, usize)> = Vec::new();
    for item in order {
        if kept.last().is_none_or(|&(_, e, _)| item.0 >= e) {
            kept.push(item);
        }
    }

    let b = source.as_bytes();
    let in_comment = |p: usize| {
        let i = comments.partition_point(|&(s, _)| s <= p);
        i > 0 && p < comments[i - 1].1
    };
    // A byte of the container's own words: not whitespace, not punctuation,
    // not inside a comment (`vecdb_common::has_meaning`, byte-wise).
    let is_word = |p: usize| {
        let c = b[p];
        !c.is_ascii_whitespace() && !c.is_ascii_punctuation() && !in_comment(p)
    };
    // Any byte of container text, punctuation included (`}`), outside comments.
    let is_solid = |p: usize| !b[p].is_ascii_whitespace() && !in_comment(p);
    // One past the first newline in `from..to`.
    let line_end = |from: usize, to: usize| (from..to).find(|&p| b[p] == b'\n').map(|p| p + 1);

    let mut owned: Vec<(usize, String, String)> = Vec::new(); // (child, leading, trailing)
    let mut leads: Vec<usize> = vec![0; kept.len()];
    let mut trails: Vec<usize> = vec![0; kept.len()];
    for k in 0..=kept.len() {
        let a = if k == 0 { lo } else { kept[k - 1].1 };
        let z = if k == kept.len() { hi } else { kept[k].0 };
        if k > 0 {
            // Trailing of the child before: the run of separators, spaces and
            // same-line comments after it, and the line break ending it. If the
            // line goes on with container text (`1 }`), only the separators
            // and comments of the run.
            // A line break ends the run even inside a comment: some grammars
            // include a line comment's newline in its node.
            let is_run = |q: usize| {
                b[q] != b'\n'
                    && (matches!(b[q], b',' | b';' | b' ' | b'\t' | b'\r') || in_comment(q))
            };
            let p = (a..z).find(|&q| !is_run(q)).unwrap_or(z);
            let mut end = if p < z && b[p] == b'\n' {
                p + 1
            } else if p == z {
                z
            } else {
                (a..p)
                    .rev()
                    .find(|&q| matches!(b[q], b',' | b';') || in_comment(q))
                    .map_or(a, |q| q + 1)
            };
            if k == kept.len() {
                // After the last child: own-line comments before the closing
                // text describe nothing that follows — they are this child's.
                let close = (end..z).find(|&q| is_solid(q)).unwrap_or(z);
                let last_comment = comments
                    .iter()
                    .filter(|&&(s, e)| s >= end && e <= close)
                    .map(|&(_, e)| e)
                    .max();
                if let Some(ce) = last_comment {
                    end = line_end(ce, close).unwrap_or(ce);
                }
            }
            trails[k - 1] = end;
        }
        if k < kept.len() {
            let from = if k == 0 { a } else { trails[k - 1] };
            // Leading of the child after: from the line after the gap's last
            // container word, or from the end of the child before.
            let last_word = (from..z).rev().find(|&q| is_word(q));
            let start = match last_word {
                Some(w) => line_end(w, z).unwrap_or(z),
                None if k == 0 => line_end(from, z).unwrap_or(z),
                None => from,
            };
            leads[k] = start;
        }
    }
    for (k, &(s, e, i)) in kept.iter().enumerate() {
        let lead = source.get(leads[k]..s).unwrap_or_default().to_string();
        let trail = source.get(e..trails[k]).unwrap_or_default().to_string();
        owned.push((i, lead, trail));
    }
    for (i, lead, trail) in owned {
        parent.children[i].set_trivia(lead, trail);
    }
    for child in parent.children.iter_mut() {
        assign_nested_trivia(child, source, comments);
    }
}

/// Rebuild the text of a document from its top-level elements: each one's
/// owned leading trivia + its source text + its owned trailing trivia, in
/// order. For a parser that assigned trivia (`assign_trivia`) this reproduces
/// `source` exactly — the lossless property, and the test of it.
///
/// Elements sharing a span (Go's grouped imports) contribute their text once;
/// an element with no span cannot be placed and is reported as an error.
pub fn reconstruct(
    elements: &[DocumentElement],
    source: &str,
    is_top: impl Fn(&DocumentElement) -> bool,
) -> Result<String, String> {
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    for (i, e) in elements.iter().enumerate().filter(|(_, e)| is_top(e)) {
        match e.source_span() {
            Some((s, t)) => spans.push((s, t, i)),
            None => {
                return Err(format!(
                    "element {i} ({}) has no source span",
                    e.element_type
                ))
            }
        }
    }
    spans.sort_unstable();
    let mut out = String::new();
    let mut group_end = 0;
    for (s, e, i) in spans {
        if s < group_end {
            if e > group_end {
                out.push_str(source.get(group_end..e).unwrap_or_default());
                group_end = e;
            }
            continue;
        }
        let el = &elements[i];
        out.push_str(el.leading_trivia());
        out.push_str(source.get(s..e).unwrap_or_default());
        out.push_str(el.trailing_trivia());
        group_end = e;
    }
    Ok(out)
}

/// Test helper: `reconstruct` must equal `source`; panics naming the first
/// differing byte and line.
#[cfg(test)]
pub(crate) fn assert_reconstructs(
    lang: &str,
    elements: &[DocumentElement],
    source: &str,
    is_top: impl Fn(&DocumentElement) -> bool,
) {
    let rebuilt = reconstruct(elements, source, is_top).unwrap_or_else(|e| panic!("{lang}: {e}"));
    if rebuilt != source {
        let at = rebuilt
            .bytes()
            .zip(source.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(rebuilt.len().min(source.len()));
        let line = LineCounter::new(source).get_line_number(at);
        panic!(
            "{lang}: reconstruction differs from the source at byte {at} (line {line}); \
             rebuilt {} bytes, source {} bytes",
            rebuilt.len(),
            source.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_gap_keeps_every_byte_and_follows_the_comment_rule() {
        let gap = " // trails a\n\n// leads b\n";
        let (t, l) = split_gap(gap);
        assert_eq!(t, " // trails a\n");
        assert_eq!(l, "\n// leads b\n");
        assert_eq!(format!("{t}{l}"), gap);
        assert_eq!(split_gap(" "), (" ", ""));
    }

    #[test]
    fn own_line_leads_same_line_trails_tail_belongs_to_last() {
        //          0         1         2         3
        //          0123456789012345678901234567890123456
        let src = b"// a\nfn a() {} // t\n// b\nfn b() {}\n// end\n";
        let a = (5, 14);
        let b = (25, 34);
        let items = [a, b];
        assert_eq!(comment_owner(src, &items, (0, 4)), Some(Owner::Next(0)));
        assert_eq!(
            comment_owner(src, &items, (15, 19)),
            Some(Owner::Previous(0))
        );
        assert_eq!(comment_owner(src, &items, (20, 24)), Some(Owner::Next(1)));
        assert_eq!(
            comment_owner(src, &items, (35, 41)),
            Some(Owner::Previous(1))
        );
        assert_eq!(comment_owner(src, &[], (0, 4)), None);
    }
}
