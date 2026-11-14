/// Whether `text` carries any meaning worth embedding or keeping as an element:
/// at least one character that is neither whitespace nor ASCII punctuation.
///
/// One definition, shared: vecq's coverage module uses it to decide whether a
/// wrapper's leftover text (`#ifndef GUARD_H` yes, `};` no) becomes an element,
/// and vecdb's chunker uses it to decide whether a container frame becomes a
/// chunk.
pub fn has_meaning(text: &str) -> bool {
    text.chars()
        .any(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
}

/// Stitches two text fragments by finding the maximum overlapping suffix of `base`
/// that matches the prefix of `extension`.
///
/// Returns the merged string.
pub fn stitch_text(base: &str, extension: &str) -> String {
    if extension.is_empty() {
        return base.to_string();
    }
    if base.is_empty() {
        return extension.to_string();
    }

    let base_bytes = base.as_bytes();
    let ext_bytes = extension.as_bytes();
    let mut max_overlap = 0;

    // Minimum overlap to consider stitching (prevents accidental matches)
    let min_overlap = 10;

    // Limits the search range to avoid O(N^2) on huge strings
    let search_range = std::cmp::min(std::cmp::min(base_bytes.len(), ext_bytes.len()), 5000);

    for i in min_overlap..=search_range {
        if base_bytes[base_bytes.len() - i..] == ext_bytes[..i] {
            max_overlap = i;
        }
    }

    let mut result = base.to_string();
    result.push_str(&extension[max_overlap..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_meaning_ignores_whitespace_and_punctuation() {
        assert!(has_meaning("#ifndef GUARD_H"));
        assert!(has_meaning("impl Foo {"));
        assert!(!has_meaning("};"));
        assert!(!has_meaning("  \n\t ////// "));
        assert!(!has_meaning(""));
    }

    #[test]
    fn test_stitch_text() {
        let base = "Hello world, this is a test";
        let extension = "is is a test of stitching.";
        let combined = stitch_text(base, extension);
        assert_eq!(combined, "Hello world, this is a test of stitching.");
    }

    #[test]
    fn test_stitch_text_no_overlap() {
        let base = "Hello world";
        let extension = "Goodbye world";
        let combined = stitch_text(base, extension);
        assert_eq!(combined, "Hello worldGoodbye world");
    }
}
