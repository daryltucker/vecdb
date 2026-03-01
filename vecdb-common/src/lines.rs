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
pub fn line_range_from_span(content: &str, start: usize, end: usize) -> (usize, usize) {
    let start_line = line_number_from_offset(content, start);
    let end_line = line_number_from_offset(content, end);
    (start_line, end_line)
}
