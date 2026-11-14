/*
 * PURPOSE:
 *   Efficient line counting and offset-to-line mapping.
 *   Prevents O(N^2) performance meltdowns by pre-calculating newline positions.
 */

pub struct LineCounter {
    offsets: Vec<usize>,
}

impl LineCounter {
    /// Create a new LineCounter from content.
    /// O(N) one-time scan.
    pub fn new(content: &str) -> Self {
        let offsets = std::iter::once(0)
            .chain(content.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self { offsets }
    }

    /// Get the 1-indexed line number for a byte offset.
    /// O(log L) where L is the number of lines.
    pub fn get_line_number(&self, pos: usize) -> usize {
        match self.offsets.binary_search(&pos) {
            Ok(idx) => idx + 1,
            Err(idx) => idx,
        }
    }

    /// Get the total number of lines.
    pub fn count(&self) -> usize {
        self.offsets.len()
    }

    /// Byte offset where the 1-indexed `line` begins; `None` past the end.
    /// The inverse of [`get_line_number`](Self::get_line_number).
    pub fn line_offset(&self, line: usize) -> Option<usize> {
        line.checked_sub(1)
            .and_then(|i| self.offsets.get(i))
            .copied()
    }

    /// The lines of the byte span `start..end` (`end` exclusive), as vecq
    /// reports them for every element of every language: 1-indexed and
    /// INCLUSIVE at both ends — `line_start` holds the span's first byte,
    /// `line_end` its last byte. An empty span reports its start line twice.
    ///
    /// `line_end` is inclusive at both ends: it is the line holding byte
    /// `end - 1`, never the line after a newline. Every language uses this one
    /// implementation; no parser computes lines on its own.
    pub fn line_range(&self, start: usize, end: usize) -> (usize, usize) {
        let first = self.get_line_number(start);
        let last = self.get_line_number(end.saturating_sub(1).max(start));
        (first, last)
    }
}

/// Calculate line number from byte offset in content (Legacy O(N) implementation).
/// Use LineCounter for repeated lookups or large files.
pub fn line_number_from_offset(content: &str, offset: usize) -> usize {
    content[..offset.min(content.len())]
        .chars()
        .filter(|&c| c == '\n')
        .count()
        + 1
}

/// Extract line range for a span of text (Legacy O(N) implementation).
///
/// Inclusive at both ends, exactly as [`LineCounter::line_range`]: `end` is
/// exclusive, so the last line is the one holding byte `end - 1`.
/// It used to report the line holding byte `end` — the line after a span
/// ending in a newline; now it reports the line holding byte `end - 1`.
pub fn line_range_from_span(content: &str, start: usize, end: usize) -> (usize, usize) {
    let start_line = line_number_from_offset(content, start);
    let end_line = line_number_from_offset(content, end.saturating_sub(1).max(start));
    (start_line, end_line)
}

#[cfg(test)]
mod line_counter_tests {
    use super::LineCounter;

    #[test]
    fn line_offset_inverts_get_line_number() {
        let src = "ab\ncd\n\nef";
        let lc = LineCounter::new(src);
        for line in 1..=lc.count() {
            let at = lc.line_offset(line).unwrap();
            assert_eq!(lc.get_line_number(at), line);
        }
        assert_eq!(lc.line_offset(0), None);
        assert_eq!(lc.line_offset(lc.count() + 1), None);
    }

    /// A span ending in a newline ends on ITS line, not the next one; a one-line
    /// element reports `n–n`.
    #[test]
    fn line_range_is_inclusive_of_the_last_byte() {
        let src = "# Title\n\nIntro\n";
        let lc = LineCounter::new(src);
        assert_eq!(lc.line_range(0, 7), (1, 1)); // `# Title`
        assert_eq!(lc.line_range(0, 8), (1, 1)); // `# Title\n` — still line 1
        assert_eq!(lc.line_range(9, 14), (3, 3)); // `Intro`
        assert_eq!(lc.line_range(0, 14), (1, 3));
        assert_eq!(lc.line_range(9, 9), (3, 3)); // empty span
    }
}
