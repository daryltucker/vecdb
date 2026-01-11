/// Stitches two text fragments by finding the maximum overlapping suffix of `base`
/// that matches the prefix of `extension`.
/// 
/// Returns the merged string.
pub fn stitch_text(base: &str, extension: &str) -> String {
    if extension.is_empty() { return base.to_string(); }
    if base.is_empty() { return extension.to_string(); }

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
