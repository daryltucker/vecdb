/*
 * PURPOSE:
 *   Parses `.vecdbrc` — a per-project route-based ingestion config file.
 *   Allows mapping glob patterns to collections so that `vecdb ingest ./`
 *   routes files to different collections automatically.
 *
 * REQUIREMENTS:
 *   - TOML format, consistent with `~/.config/vecdb/config.toml`
 *   - `[default]` section: fallback collection
 *   - `[[routes]]` section: glob → collection mappings
 *   - Walk-up discovery from ingest path (like `.gitignore`)
 *   - First-match-wins route selection
 *
 * USAGE:
 *   let rc = VecdbRc::discover("/path/to/project")?;
 *   let collection = rc.route("data/foo.md", None);
 *
 * SELF-HEALING INSTRUCTIONS:
 *   - Missing `.vecdbrc` is not an error — returns default collection
 *   - Malformed `.vecdbrc` returns error with file location
 *   - Invalid glob patterns produce a warning, not a hard failure
 */

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// A parsed `.vecdbrc` file.
#[derive(Debug, Clone, Deserialize)]
pub struct VecdbRc {
    /// Default section: fallback collection when no route matches
    #[serde(default)]
    pub default: Option<DefaultSection>,
    /// Route table: ordered list of glob → collection mappings
    #[serde(default)]
    pub routes: Vec<Route>,
}

/// The `[default]` section of `.vecdbrc`.
#[derive(Debug, Clone, Deserialize)]
pub struct DefaultSection {
    /// Collection to use when no route matches
    pub collection: Option<String>,
}

/// A single `[[routes]]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct Route {
    /// Glob pattern (gitignore-style, relative to project root)
    pub glob: String,
    /// Target collection for matching files
    pub collection: String,
    /// If true, bypass `.vectorignore` for files matching this route
    #[serde(default)]
    pub ignore_vector_ignore: bool,
}

impl VecdbRc {
    /// Discover and parse `.vecdbrc` by walking up from `start_path`.
    ///
    /// Looks for a file named `.vecdbrc` in `start_path` and each ancestor
    /// directory up to the filesystem root. The first one found wins.
    /// Returns `Ok(None)` if no `.vecdbrc` exists (which is not an error).
    pub fn discover(start_path: &Path) -> Result<Option<(PathBuf, Self)>> {
        let mut current = if start_path.is_file() {
            start_path.parent().unwrap_or(start_path).to_path_buf()
        } else {
            start_path.to_path_buf()
        };

        loop {
            let rc_path = current.join(".vecdbrc");
            if rc_path.exists() {
                let content = std::fs::read_to_string(&rc_path)
                    .with_context(|| format!("Failed to read {}", rc_path.display()))?;
                let rc: VecdbRc = toml::from_str(&content)
                    .with_context(|| format!("Failed to parse {} (invalid TOML)", rc_path.display()))?;
                return Ok(Some((rc_path, rc)));
            }

            // Walk up to parent
            if !current.pop() {
                break; // Reached filesystem root
            }
        }

        Ok(None)
    }

    /// Determine the target collection for a file path.
    ///
    /// Precedence:
    /// 1. First matching `[[routes]]` entry (top-down, first-match-wins)
    /// 2. CLI `--collection` flag value (provided via `cli_collection`)
    /// 3. `.vecdbrc [default] collection`
    ///
    /// Returns `(collection_name, ignore_vector_ignore)`.
    pub fn route(&self, rel_path: &str, cli_collection: Option<&str>) -> (String, bool) {
        // 1. Check routes in order
        for route in &self.routes {
            if let Ok(glob) = glob::Pattern::new(&route.glob) {
                if glob.matches(rel_path) {
                    return (route.collection.clone(), route.ignore_vector_ignore);
                }
            }
        }

        // 2. CLI flag fills the unrouted-default slot
        if let Some(cli) = cli_collection {
            return (cli.to_string(), false);
        }

        // 3. .vecdbrc [default] section
        if let Some(ref default) = self.default {
            if let Some(ref coll) = default.collection {
                return (coll.clone(), false);
            }
        }

        // 4. No route matched, no CLI flag, no default — caller must handle
        ("".to_string(), false)
    }

    /// Returns the default collection from `[default]` section, if any.
    pub fn default_collection(&self) -> Option<&str> {
        self.default.as_ref()?.collection.as_deref()
    }

    /// Returns the path of the project root (parent dir of .vecdbrc).
    /// This is the directory that contains the .vecdbrc file.
    pub fn project_root(rc_path: &Path) -> Option<&Path> {
        rc_path.parent()
    }

    /// Check if a route with `ignore_vector_ignore = true` matches the file.
    /// Returns the target collection if matched, None otherwise.
    pub fn matches_ignore_override(&self, rel_path: &str) -> Option<&str> {
        for route in &self.routes {
            if route.ignore_vector_ignore {
                if let Ok(glob) = glob::Pattern::new(&route.glob) {
                    if glob.matches(rel_path) {
                        return Some(&route.collection);
                    }
                }
            }
        }
        None
    }
}

/// Resolve a route for a given file path against a list of routes.
/// This is a free function for ergonomic use from the ingestion pipeline.
/// Returns `(collection_name, ignore_vector_ignore)`.
///
/// Precedence:
/// 1. First matching `[[routes]]` entry (top-down, first-match-wins via user ordering)
/// 2. `cli_collection` (CLI `--collection` flag) — fills the unrouted-default slot
/// 3. `fallback_collection` (e.g., `.vecdbrc [default]` or profile default)
///
/// If all of the above are empty, returns the `fallback_collection` (may be empty).
pub fn resolve_route(
    routes: &[Route],
    rel_path: &str,
    cli_collection: Option<&str>,
) -> (String, bool) {
    for route in routes {
        if let Ok(glob) = glob::Pattern::new(&route.glob) {
            if glob.matches(rel_path) {
                return (route.collection.clone(), route.ignore_vector_ignore);
            }
        }
    }
    if let Some(cli) = cli_collection {
        if !cli.is_empty() {
            return (cli.to_string(), false);
        }
    }
    ("".to_string(), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parse_basic() {
        let toml_str = r#"
[default]
collection = "code"

[[routes]]
glob = "data/descriptions_export/**/*.md"
collection = "music"

[[routes]]
glob = "docs/research/**"
collection = "docs-lts"
"#;

        let rc: VecdbRc = toml::from_str(toml_str).unwrap();
        assert_eq!(rc.default.as_ref().unwrap().collection.as_deref(), Some("code"));
        assert_eq!(rc.routes.len(), 2);
        assert_eq!(rc.routes[0].collection, "music");
        assert_eq!(rc.routes[1].collection, "docs-lts");
    }

    #[test]
    fn test_route_resolution_first_match_wins() {
        let toml_str = r#"
[default]
collection = "code"

[[routes]]
glob = "*.rs"
collection = "rust-code"

[[routes]]
glob = "docs/**"
collection = "docs-lts"
"#;

        let rc: VecdbRc = toml::from_str(toml_str).unwrap();

        // First match wins
        let (coll, _) = rc.route("src/main.rs", None);
        assert_eq!(coll, "rust-code");

        let (coll, _) = rc.route("docs/guide.md", None);
        assert_eq!(coll, "docs-lts");
    }

    #[test]
    fn test_route_fallback_to_default() {
        let toml_str = r#"
[default]
collection = "everything-else"
"#;

        let rc: VecdbRc = toml::from_str(toml_str).unwrap();

        let (coll, _) = rc.route("random/file.txt", None);
        assert_eq!(coll, "everything-else");
    }

    #[test]
    fn test_cli_flag_fills_unrouted_slot() {
        let toml_str = r#"
[[routes]]
glob = "src/**"
collection = "code"
"#;

        let rc: VecdbRc = toml::from_str(toml_str).unwrap();

        // Routed file ignores CLI flag per RFC spec
        let (coll, _) = rc.route("src/main.rs", Some("override"));
        assert_eq!(coll, "code");

        // Unrouted file uses CLI flag
        let (coll, _) = rc.route("data/file.txt", Some("override"));
        assert_eq!(coll, "override");
    }

    #[test]
    fn test_ignore_vector_ignore_flag() {
        let toml_str = r#"
[[routes]]
glob = "private/**"
collection = "secret-stuff"
ignore_vector_ignore = true
"#;

        let rc: VecdbRc = toml::from_str(toml_str).unwrap();

        let (coll, ignore) = rc.route("private/notes.txt", None);
        assert_eq!(coll, "secret-stuff");
        assert!(ignore);
    }

    #[test]
    fn test_discover_no_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = VecdbRc::discover(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_discover_finds_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let rc_path = tmp.path().join(".vecdbrc");
        let mut f = std::fs::File::create(&rc_path).unwrap();
        f.write_all(b"[default]\ncollection = \"code\"\n").unwrap();

        let result = VecdbRc::discover(tmp.path()).unwrap();
        assert!(result.is_some());
        let (path, rc) = result.unwrap();
        assert_eq!(path, rc_path);
        assert_eq!(rc.default_collection(), Some("code"));
    }

    #[test]
    fn test_discover_walks_up() {
        let tmp = tempfile::TempDir::new().unwrap();
        let subdir = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&subdir).unwrap();

        // Place .vecdbrc in project root (a/)
        let project_root = tmp.path().join("a");
        let rc_path = project_root.join(".vecdbrc");
        let mut f = std::fs::File::create(&rc_path).unwrap();
        f.write_all(b"[[routes]]\nglob = \"**\"\ncollection = \"found\"\n").unwrap();

        // Start from deep subdirectory
        let result = VecdbRc::discover(&subdir).unwrap();
        assert!(result.is_some());
        let (found_path, _) = result.unwrap();
        assert_eq!(found_path, rc_path);
    }

    #[test]
    fn test_ignore_override_via_helper() {
        let toml_str = r#"
[[routes]]
glob = "private/**"
collection = "secret"
ignore_vector_ignore = true

[[routes]]
glob = "public/**"
collection = "open"
ignore_vector_ignore = false
"#;

        let rc: VecdbRc = toml::from_str(toml_str).unwrap();
        assert_eq!(rc.matches_ignore_override("private/notes.txt"), Some("secret"));
        assert_eq!(rc.matches_ignore_override("public/readme.md"), None);
        assert_eq!(rc.matches_ignore_override("other/file.txt"), None);
    }
}
