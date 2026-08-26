use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

/// Supported file types for parsing and conversion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileType {
    Markdown,
    Rust,
    Python,
    C,
    Cpp,
    Cuda,
    Go,
    Bash,
    Json,
    Html,
    Toml,
    Yaml,
    Text,

    Unknown,
}

impl FileType {
    /// Get file type from file extension
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "rs" => Some(Self::Rust),
            "py" | "pyw" => Some(Self::Python),
            "c" | "h" => Some(Self::C),
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Some(Self::Cpp),
            "cu" | "cuh" => Some(Self::Cuda),
            "go" => Some(Self::Go),
            "sh" | "bash" => Some(Self::Bash),
            "json" | "ndjson" | "jsonl" => Some(Self::Json),
            "html" | "htm" | "xml" | "xhtml" => Some(Self::Html),
            "toml" => Some(Self::Toml),
            // YAML used to map to Text, so every .yaml and .yml file was stored
            // as one undifferentiated blob. It is structured configuration and
            // is parsed as such.
            "yaml" | "yml" => Some(Self::Yaml),
            "txt" | "log" | "cfg" | "ini" | "conf" => Some(Self::Text),

            _ => None,
        }
    }

    /// Get file type from file path
    pub fn from_path<P: AsRef<Path>>(path: P) -> Self {
        let path = path.as_ref();

        // First try standard extension
        if let Some(ft) = path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension)
        {
            return ft;
        }

        // Handle .resolved.N files (e.g. task.md.resolved.0 -> task.md)
        let path_str = path.to_string_lossy();
        if path_str.contains(".resolved.") {
            let parts: Vec<&str> = path_str.split('.').collect();
            // iterate backwards
            for i in (0..parts.len()).rev() {
                if let Some(ft) = Self::from_extension(parts[i]) {
                    return ft;
                }
            }
        }

        Self::Unknown
    }

    /// Get list of common extensions for this file type
    pub fn file_extensions(&self) -> Vec<&'static str> {
        match self {
            Self::Markdown => vec!["md", "markdown"],
            Self::Rust => vec!["rs"],
            Self::Python => vec!["py", "pyw"],
            Self::C => vec!["c", "h"],
            Self::Cpp => vec!["cpp", "cc", "cxx", "hpp", "hxx"],
            Self::Cuda => vec!["cu", "cuh"],
            Self::Go => vec!["go"],
            Self::Bash => vec!["sh", "bash"],
            Self::Json => vec!["json", "ndjson", "jsonl"],
            Self::Html => vec!["html", "htm", "xml", "xhtml"],
            Self::Toml => vec!["toml"],
            Self::Yaml => vec!["yaml", "yml"],
            Self::Text => vec!["txt", "log", "cfg", "ini", "conf", "yaml", "yml"],
            Self::Unknown => vec![],
        }
    }

    /// Check if this file type is supported for parsing
    pub fn is_supported(&self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// Check if content is likely text (not binary soup)
    /// Scans first 1KB for control characters or low printable ratio.
    pub fn is_likely_text(content: &[u8]) -> bool {
        if content.is_empty() {
            return true;
        }

        let sample_len = content.len().min(1024);
        let sample = &content[..sample_len];

        // Fast scan for null bytes (indicates binary)
        if sample.contains(&0) {
            return false;
        }

        // Check ratio of printable characters (including whitespace and UTF-8)
        let printable = sample
            .iter()
            .filter(|&&b| (32..=126).contains(&b) || b == 9 || b == 10 || b == 13 || b >= 128)
            .count();

        (printable as f32 / sample_len as f32) > 0.85
    }

    /// Whether content should be treated as binary and skipped for ingestion.
    ///
    /// This is deliberately stricter than [`Self::is_likely_text`], which
    /// counts every byte >= 128 as printable. That is fine for display, but as
    /// an ingestion gate it lets compressed and encrypted data straight
    /// through: for uniformly random bytes roughly 88% land in its "printable"
    /// set, clearing the 0.85 threshold. Random data is the *most* binary thing
    /// there is, so a check it passes is not a check.
    ///
    /// Three independent signals, cheapest first:
    ///
    /// 1. **NUL byte** — decisive for most real formats.
    /// 2. **Magic number** — catches formats whose header is NUL-free (JPEG,
    ///    PNG, gzip, ZIP and every ZIP-based office/jar format).
    /// 3. **UTF-8 validity** — the discriminator the ratio test lacks. Random
    ///    high bytes almost never form valid UTF-8 sequences, whereas real text
    ///    in any language always does.
    ///
    /// Callers must apply this regardless of detected file type: an extension
    /// is a claim about content, not evidence of it, and `.json`/`.txt` get
    /// attached to blobs constantly.
    pub fn is_binary_content(content: &[u8]) -> bool {
        if content.is_empty() {
            return false;
        }

        let sample_len = content.len().min(8192);
        let sample = &content[..sample_len];

        if sample.contains(&0) {
            return true;
        }

        if Self::has_binary_magic(content) {
            return true;
        }

        match std::str::from_utf8(sample) {
            Ok(_) => {}
            Err(e) => {
                // `error_len() == None` means the sample ended mid-sequence,
                // which is expected when slicing a valid UTF-8 file at a fixed
                // byte offset. Only a genuinely malformed sequence counts.
                if e.error_len().is_some() {
                    return true;
                }
                // Truncated tail: re-check only the part known to be complete.
                if std::str::from_utf8(&sample[..e.valid_up_to()]).is_err() {
                    return true;
                }
            }
        }

        // Control characters other than tab/newline/carriage-return/form-feed
        // are rare in text and common in binary that survived the above.
        let control = sample
            .iter()
            .filter(|&&b| b < 32 && !matches!(b, 9 | 10 | 13 | 12))
            .count();

        (control as f32 / sample_len as f32) > 0.02
    }

    /// Signature check for binary container formats that contain no early NUL.
    fn has_binary_magic(content: &[u8]) -> bool {
        const MAGICS: &[&[u8]] = &[
            b"\x7fELF",             // ELF executable / shared object
            b"\x89PNG\r\n\x1a\n",   // PNG
            b"\xff\xd8\xff",        // JPEG
            b"GIF87a",              // GIF
            b"GIF89a",              // GIF
            b"%PDF-",               // PDF
            b"PK\x03\x04",          // ZIP, and thus docx/xlsx/pptx/jar/odt
            b"PK\x05\x06",          // empty ZIP
            b"\x1f\x8b",            // gzip
            b"BZh",                 // bzip2
            b"\xfd7zXZ\x00",        // xz
            b"7z\xbc\xaf\x27\x1c",  // 7-zip
            b"Rar!\x1a\x07",        // RAR
            b"\x00asm",             // WebAssembly
            b"\xca\xfe\xba\xbe",    // Java class / Mach-O fat binary
            b"SQLite format 3\x00", // SQLite database
            b"OggS",                // Ogg
            b"fLaC",                // FLAC
            b"\x1aE\xdf\xa3",       // Matroska / WebM
            b"wOFF",                // WOFF font
            b"wOF2",                // WOFF2 font
            b"\x00\x01\x00\x00",    // TrueType font
            b"OTTO",                // OpenType font
        ];

        // Deliberately absent: "BM" (BMP) and "ID3" (tagged MP3). Both are
        // short enough to begin a legitimate text file — "BM" opens plenty of
        // prose — and a false positive here silently drops real content, which
        // is worse than missing a format the NUL and UTF-8 checks already
        // catch on the very next bytes.
        MAGICS.iter().any(|m| content.starts_with(m))
    }

    /// Categorize file type into a broader capability class for strategy selection
    pub fn capability(&self) -> ParsingCapability {
        match self {
            Self::Markdown | Self::Html => ParsingCapability::Document,
            Self::Json | Self::Toml | Self::Yaml => ParsingCapability::Data,

            Self::Rust
            | Self::Python
            | Self::C
            | Self::Cpp
            | Self::Cuda
            | Self::Go
            | Self::Bash => ParsingCapability::Code,

            Self::Text => ParsingCapability::Simple,
            Self::Unknown => ParsingCapability::Simple, // The Lua fallback
        }
    }
}

/// Broad categories of parsing behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParsingCapability {
    /// Structured documents (Markdown, HTML)
    Document,
    /// Source code (Python, Rust, etc)
    Code,
    /// Structured data (JSON, TOML)
    Data,
    /// Unstructured or unknown text
    Simple,
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Markdown => "Markdown",
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Cuda => "CUDA",
            Self::Go => "Go",
            Self::Bash => "Bash",
            Self::Json => "JSON",
            Self::Html => "HTML",
            Self::Toml => "TOML",
            Self::Yaml => "YAML",
            Self::Text => "Text",

            Self::Unknown => "Unknown",
        };
        write!(f, "{}", name)
    }
}

/// Trait for detecting file types
/// This allows dependency injection of the detection logic into vecdb-core
pub trait FileTypeDetector: Send + Sync {
    /// Detect file type from path and content
    fn detect(&self, path: &Path, content: &[u8]) -> FileType;
}

#[cfg(test)]
mod binary_detection_tests {
    use super::FileType;

    #[test]
    fn plain_text_and_source_are_not_binary() {
        assert!(!FileType::is_binary_content(b"hello world\n"));
        assert!(!FileType::is_binary_content(
            b"fn main() {\n    println!(\"hi\");\n}\n"
        ));
        assert!(!FileType::is_binary_content(b"{\"key\": [1, 2, 3]}"));
        // Empty is not binary; there is simply nothing to reject.
        assert!(!FileType::is_binary_content(b""));
    }

    #[test]
    fn non_ascii_text_is_not_binary() {
        // The check must not be an ASCII filter — that would drop most of the
        // world's prose and every file with an em dash in it.
        assert!(!FileType::is_binary_content("héllo wörld".as_bytes()));
        assert!(!FileType::is_binary_content("日本語のテキスト".as_bytes()));
        assert!(!FileType::is_binary_content("emoji: 🚀🎉".as_bytes()));
        assert!(!FileType::is_binary_content("— en/em – dashes".as_bytes()));
    }

    #[test]
    fn nul_byte_is_binary() {
        assert!(FileType::is_binary_content(b"text\0more text"));
    }

    #[test]
    fn magic_numbers_are_binary_even_without_a_nul() {
        // JPEG and gzip carry no NUL in their first bytes, so the old
        // NUL-only check passed them straight through.
        assert!(FileType::is_binary_content(b"\xff\xd8\xff\xe0abcdefgh"));
        assert!(FileType::is_binary_content(b"\x1f\x8b\x08\x00abcdefgh"));
        assert!(FileType::is_binary_content(b"PK\x03\x04rest of a zip"));
        assert!(FileType::is_binary_content(b"%PDF-1.7 trailing"));
        assert!(FileType::is_binary_content(b"\x7fELF\x02\x01\x01"));
    }

    /// The case the ratio-based heuristic cannot catch.
    ///
    /// For uniformly random bytes about 88% fall in `is_likely_text`'s
    /// "printable" set (32..=126 plus everything >= 128), clearing its 0.85
    /// threshold. Compressed and encrypted payloads look exactly like this.
    #[test]
    fn high_byte_noise_is_binary_despite_passing_the_ratio_test() {
        // Deterministic pseudo-random bytes; no NUL, no magic number.
        let mut data = Vec::with_capacity(4096);
        let mut x: u32 = 0x1234_5678;
        while data.len() < 4096 {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let b = (x >> 16) as u8;
            if b != 0 {
                data.push(b);
            }
        }

        assert!(
            !data.contains(&0),
            "test fixture must not rely on the NUL check"
        );
        assert!(
            FileType::is_binary_content(&data),
            "random high-byte noise must be rejected; this is what compressed \
             and encrypted data looks like"
        );
    }

    #[test]
    fn utf8_text_split_mid_sequence_is_not_binary() {
        // Slicing a preview at a fixed byte offset can cut a multi-byte
        // character in half. That is a truncated read, not binary content.
        let text = "aaaa日本語のテキストです".as_bytes();
        for cut in 1..text.len() {
            let sample = &text[..cut];
            assert!(
                !FileType::is_binary_content(sample),
                "valid UTF-8 truncated at byte {cut} was misread as binary"
            );
        }
    }
}

#[cfg(test)]
mod binary_magic_false_positive_tests {
    use super::FileType;

    /// Short signatures must not shadow real prose.
    #[test]
    fn text_starting_with_short_signature_like_bytes_is_kept() {
        assert!(!FileType::is_binary_content(
            b"BM is an abbreviation used throughout this document.\n"
        ));
        assert!(!FileType::is_binary_content(
            b"ID3 tags are metadata containers for MP3 files.\n"
        ));
    }
}
