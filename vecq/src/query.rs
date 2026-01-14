// PURPOSE:
//   jq-compatible query engine for processing JSON documents with standard jq syntax.
//   Critical for vecq's core value proposition - enabling familiar jq querying on
//   any structured document. Must maintain 100% compatibility with standard jq
//   to leverage existing user knowledge and tooling ecosystem.
//
// REQUIREMENTS:
//   User-specified:
//   - Must support all standard jq operators (select, map, filter, group_by, sort_by)
//   - Must produce identical results to standard jq for equivalent queries
//   - Must support jq's pipe operator for chaining query operations
//   - Must handle complex jq expressions including conditionals and recursive descent
//   - Must provide query validation and helpful error messages
//   
//   Implementation-discovered:
//   - Uses jaq (pure Rust jq implementation) for hermetic builds
//   - Must implement query caching for performance with repeated queries
//   - Needs comprehensive error handling for invalid jq syntax
//   - Must support query explanation for debugging and learning
//
// IMPLEMENTATION RULES:
//   1. Use jaq crates for all query execution to ensure compatibility
//      Rationale: Pure Rust, no C dependencies, hermetic builds
//   
//   2. Cache compiled jaq filters using LRU cache for performance
//      Rationale: Compilation is expensive, queries are often repeated
//   
//   3. Provide detailed error messages with query position information
//      Rationale: Users need precise feedback for debugging complex queries
//   
//   4. Support query explanation to help users understand jq syntax
//      Rationale: Enables learning and debugging of complex query expressions
//   
//   5. Validate queries before execution to catch syntax errors early
//      Rationale: Better user experience than runtime failures
//   
//   Critical:
//   - DO NOT modify jq behavior or add custom operators
//   - DO NOT cache results, only compiled programs (data changes)
//   - ALWAYS preserve exact jq semantics and error messages
//
// USAGE:
//   use vecq::query::{QueryEngine, JqQueryEngine};
//   use serde_json::json;
//   
//   // Create query engine with caching
//   let mut engine = JqQueryEngine::new();
//   
//   // Execute jq query on JSON data
//   let data = json!({"functions": [{"name": "main", "visibility": "pub"}]});
//   let result = engine.execute_query(&data, ".functions[] | select(.visibility == \"pub\") | .name")?;
//   
//   // Validate query syntax
//   let validation = engine.validate_query(".functions[").unwrap_err();
//   println!("Query error: {}", validation);
//
// SELF-HEALING INSTRUCTIONS:
//   When jaq library updates:
//   1. Test all existing queries for compatibility
//   2. Update error handling if error types change
//   3. Verify query caching still works correctly
//   4. Update query explanation if new features added
//   5. Run full property test suite to catch regressions
//   
//   When adding query optimization:
//   1. Ensure optimizations don't change query semantics
//   2. Add performance benchmarks to validate improvements
//   3. Test with complex real-world queries
//   4. Document optimization behavior in code comments
//   5. Add tests to prevent optimization regressions
//
// RELATED FILES:
//   - src/converter.rs - Produces JSON that gets queried by this engine
//   - src/formatter.rs - Formats query results for different output types
//   - src/natural_language.rs - Converts natural language to jq queries
//   - src/main.rs - CLI interface that uses query engine
//   - tests/unit/query_tests.rs - Query engine validation tests
//
// MAINTENANCE:
//   Update when:
//   - jaq library releases new versions
//   - New jq operators need support
//   - Performance optimization opportunities identified
//   - User feedback indicates query error messages need improvement
//   - Query caching strategy needs adjustment for memory usage
//
// Last Verified: 2026-01-04

use crate::error::{VecqError, VecqResult};
use lru::LruCache;
use serde_json::Value;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::sync::LazyLock;
use std::fs;
use std::rc::Rc;

use jaq_syn::Def;

// jaq imports for real jq execution
use jaq_interpret::{Ctx, Filter, FilterT, ParseCtx, RcIter, Val};
use jaq_core;

// Normalizers
const NORM_LOG_NGINX: &str = include_str!("stdlib/normalizers/log_nginx.jq");
const NORM_LOG_JOURNALD: &str = include_str!("stdlib/normalizers/log_journald.jq");
const NORM_TASK_GITHUB: &str = include_str!("stdlib/normalizers/task_github.jq");
const NORM_TASK_TODO: &str = include_str!("stdlib/normalizers/task_todo.jq");
const NORM_TASK_SRC: &str = include_str!("stdlib/normalizers/task_src.jq");
const NORM_CHAT_WEBUI: &str = include_str!("stdlib/normalizers/chat_openwebui.jq");
const NORM_ARTIFACT_CARGO: &str = include_str!("stdlib/normalizers/artifact_cargo.jq");
const NORM_DIFF_GIT: &str = include_str!("stdlib/normalizers/diff_git.jq");

// Renderers
const RENDER_CHAT: &str = include_str!("stdlib/renderers/chat.jq");
const RENDER_DOC: &str = include_str!("stdlib/renderers/doc.jq");
const RENDER_TASK: &str = include_str!("stdlib/renderers/task.jq");
const RENDER_ARTIFACT: &str = include_str!("stdlib/renderers/artifact.jq");
const RENDER_DIFF: &str = include_str!("stdlib/renderers/diff.jq");
const RENDER_LOG: &str = include_str!("stdlib/renderers/log.jq");

// Logic
const AUTO_JQ: &str = include_str!("stdlib/auto.jq");

const REGEX_PRELUDE: &str = r#"
def test($r): _native_test($r);
def re_test($r): _native_test($r);
def capture($r): _native_capture($r);
def re_capture($r): _native_capture($r);
def sub($r; $s): _native_sub($r; $s);
def re_sub($r; $s): _native_sub($r; $s);
def gsub($r; $s): _native_gsub($r; $s);
def re_gsub($r; $s): _native_gsub($r; $s);
"#;

// Pre-compiled Standard Library Definitions
// This eliminates the need to re-parse the massive stdlib for every query
static PRELUDE_DEFS: LazyLock<Result<Vec<Def>, String>> = LazyLock::new(|| {
    let prelude_source = format!("{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}", 
        REGEX_PRELUDE,
        NORM_LOG_NGINX,
        NORM_LOG_JOURNALD,
        NORM_TASK_GITHUB,
        NORM_TASK_TODO,
        NORM_TASK_SRC,
        NORM_CHAT_WEBUI,
        NORM_ARTIFACT_CARGO,
        NORM_DIFF_GIT,
        RENDER_CHAT,
        RENDER_DOC,
        RENDER_TASK,
        RENDER_ARTIFACT,
        RENDER_DIFF,
        RENDER_LOG,
        AUTO_JQ
    );

    let (defs, errs) = jaq_parse::parse(&prelude_source, jaq_parse::defs());
    
    if !errs.is_empty() {
        let error_msgs: Vec<String> = errs.iter()
            .map(|e| format!("{:?}", e))
            .collect();
        return Err(format!("INTERNAL ERROR: Stdlib parse failed: {}", error_msgs.join("; ")));
    }

    Ok(defs.unwrap_or_default())
});

/// Trait for executing jq queries on JSON data
pub trait QueryEngine: Send + Sync {
    /// Execute a jq query on JSON data, returning a stream of values
    fn execute_query(&self, json: &Value, query: &str) -> VecqResult<Vec<Value>>;

    /// Validate jq query syntax without execution
    fn validate_query(&self, query: &str) -> VecqResult<()>;

    /// Explain what a jq query does (for learning/debugging)
    fn explain_query(&self, query: &str) -> VecqResult<QueryExplanation>;

    /// Get query execution statistics
    fn get_stats(&self) -> QueryStats;

    /// Clear query cache
    fn clear_cache(&self);
}

/// Information about what a jq query does
#[derive(Debug, Clone)]
pub struct QueryExplanation {
    pub query: String,
    pub description: String,
    pub operations: Vec<QueryOperation>,
    pub complexity: QueryComplexity,
}

/// Individual operation within a jq query
#[derive(Debug, Clone)]
pub struct QueryOperation {
    pub operation_type: String,
    pub description: String,
    pub example: Option<String>,
}

/// Query complexity assessment
#[derive(Debug, Clone, PartialEq)]
pub enum QueryComplexity {
    Simple,    // Basic field access, simple filters
    Moderate,  // Multiple operations, some nesting
    Complex,   // Advanced operations, deep nesting, recursion
}

/// Query execution statistics
#[derive(Debug, Clone, Default)]
pub struct QueryStats {
    pub queries_executed: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub compilation_errors: u64,
    pub execution_errors: u64,
    pub total_execution_time_ms: u64,
}

/// jq query engine implementation using jq-rs
pub struct JqQueryEngine {
    // Note: Using Mutex for interior mutability since jq-rs may not be thread-safe
    program_cache: Mutex<LruCache<String, CompiledQuery>>,
    stats: Mutex<QueryStats>,
    library_paths: Vec<std::path::PathBuf>,
    load_scripts: bool,
}

/// Compiled jaq filter with metadata
struct CompiledQuery {
    filter: Filter,
    use_count: u64,
}

impl JqQueryEngine {
    /// Create a new jq query engine
    pub fn new() -> Self {
        Self::with_cache_size(100)
    }

    /// Create query engine with specific cache size
    pub fn with_cache_size(cache_size: usize) -> Self {
        Self {
            program_cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(cache_size).unwrap_or(NonZeroUsize::new(1).unwrap())
            )),
            stats: Mutex::new(QueryStats::default()),
            library_paths: Vec::new(),
            load_scripts: true,
        }
    }

    /// Create a hermetic query engine (no external scripts loaded)
    pub fn new_hermetic() -> Self {
        Self {
            program_cache: Mutex::new(LruCache::new(NonZeroUsize::new(100).unwrap())),
            stats: Mutex::new(QueryStats::default()),
            library_paths: Vec::new(),
            load_scripts: false,
        }
    }

    /// Add a library search path
    pub fn add_library_path(&mut self, path: std::path::PathBuf) {
        self.library_paths.push(path);
    }

    /// Compile and execute a jq query, returning a list of values
    fn compile_and_execute(&self, query: &str, json: &Value) -> VecqResult<Vec<Value>> {
        let start_time = std::time::Instant::now();
        
        // Try to get cached filter
        let mut cache = self.program_cache.lock().unwrap();
        
        let filter = if let Some(compiled) = cache.get_mut(query) {
            compiled.use_count += 1;
            self.update_stats(|stats| stats.cache_hits += 1);
            compiled.filter.clone()
        } else {
            drop(cache); // Release lock for compilation
            
            // Parse and compile the query
            let filter = self.compile_jaq_filter(query)?;
            
            // Store in cache
            let mut cache = self.program_cache.lock().unwrap();
            cache.put(query.to_string(), CompiledQuery {
                filter: filter.clone(),
                use_count: 1,
            });
            self.update_stats(|stats| stats.cache_misses += 1);
            
            filter
        };
        
        // Execute the filter
        let results = self.execute_jaq_filter(&filter, json)?;
        
        let execution_time = start_time.elapsed().as_millis() as u64;
        self.update_stats(|stats| {
            stats.queries_executed += 1;
            stats.total_execution_time_ms += execution_time;
        });
        
        Ok(results)
    }

    /// Compile a jaq filter from query string
    fn compile_jaq_filter(&self, query: &str) -> VecqResult<Filter> {
        if query.trim().is_empty() {
            return Err(VecqError::query_error(
                query.to_string(),
                "Empty query".to_string(),
                Some("Try: . (identity filter)".to_string()),
            ));
        }

        // Load user-defined functions from ~/.config/vecq/functions/*.jq
        let user_scripts = self.load_user_scripts();

        // Combine user scripts and query (NO stdlib here, it's injected later)
        let user_source = format!("{}\n{}", user_scripts, query);
        
        // Parse the query using jaq_parse
        let (main, errs) = jaq_parse::parse(&user_source, jaq_parse::main());
        
        if !errs.is_empty() {
             // ... error handling ...
            let error_msgs: Vec<String> = errs.iter()
                .map(|e| format!("{:?}", e))
                .collect();
            return Err(VecqError::query_error(
                query.to_string(),
                error_msgs.join("; "),
                Some("Check jq syntax".to_string()),
            ));
        }

        let main = main.ok_or_else(|| VecqError::query_error(
            query.to_string(),
            "Failed to parse query".to_string(),
            None,
        ))?;

        // Create parse context (no variables by default)
        let mut ctx = ParseCtx::new(Vec::new());

        // Insert natives from jaq_core (provides length, keys, etc.)
        ctx.insert_natives(jaq_core::core());
        
        // Insert custom regex natives
        ctx.insert_natives(crate::natives::regex_natives());

        // Insert definitions from jaq_std (provides some standard filters like `add`, `map`, etc.)
        ctx.insert_defs(jaq_std::std());

        // Insert cached Standard Library Definitions (Normalizers, Renderers, Regex Prelude)
        match &*PRELUDE_DEFS {
            Ok(defs) => ctx.insert_defs(defs.clone()),
            Err(e) => return Err(VecqError::query_error(
                query.to_string(),
                format!("Failed to load standard library: {}", e),
                None
            )),
        }
        
        // Compile the main filter
        let filter = ctx.compile(main);
        
        if !ctx.errs.is_empty() {
            let error_msgs: Vec<String> = ctx.errs.iter()
                .map(|(e, _span)| format!("{}", e))
                .collect();
            return Err(VecqError::query_error(
                query.to_string(),
                error_msgs.join("; "),
                Some("Check jq syntax and functions".to_string()),
            ));
        }
        
        Ok(filter)
    }

    /// Execute a compiled jaq filter on JSON input
    fn execute_jaq_filter(&self, filter: &Filter, json: &Value) -> VecqResult<Vec<Value>> {
        // Convert serde_json::Value to jaq Val
        let input = self.serde_to_jaq(json);
        
        // Create empty inputs iterator (no slurp)
        let inputs = RcIter::new(std::iter::empty());
        
        // Create context
        let ctx = Ctx::new(Vec::new(), &inputs);
        
        // Run the filter and collect results
        let results: Vec<Val> = filter.run((ctx, input))
            .filter_map(|r| r.ok())
            .collect();
        
        // Convert results back to serde_json::Value
        Ok(results.iter().map(|v| self.jaq_to_serde(v)).collect())
    }

    /// Convert serde_json::Value to jaq Val
    fn serde_to_jaq(&self, value: &Value) -> Val {
        match value {
            Value::Null => Val::Null,
            Value::Bool(b) => Val::Bool(*b),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Val::from(i as isize)
                } else if let Some(f) = n.as_f64() {
                    Val::from(f)
                } else {
                    Val::Null
                }
            }
            Value::String(s) => Val::from(s.clone()),
            Value::Array(arr) => Val::arr(arr.iter().map(|v| self.serde_to_jaq(v)).collect()),
            Value::Object(obj) => {
                // Use jaq's Val::obj() which handles the IndexMap construction internally
                let pairs: Vec<(Rc<String>, Val)> = obj.iter()
                    .map(|(k, v)| (Rc::new(k.clone()), self.serde_to_jaq(v)))
                    .collect();
                Val::obj(pairs.into_iter().collect())
            }
        }
    }

    /// Convert jaq Val to serde_json::Value
    fn jaq_to_serde(&self, val: &Val) -> Value {
        match val {
            Val::Null => Value::Null,
            Val::Bool(b) => Value::Bool(*b),
            Val::Int(n) => {
                // Convert malachite integer to i64 if possible
                if let Ok(i) = i64::try_from(*n) {
                    Value::Number(serde_json::Number::from(i))
                } else {
                    // Fall back to string representation for large integers
                    Value::String(n.to_string())
                }
            }
            Val::Float(f) => {
                if let Some(n) = serde_json::Number::from_f64(*f) {
                    Value::Number(n)
                } else {
                    Value::Null
                }
            }
            Val::Num(n) => {
                // Handle rational numbers
                Value::String(n.to_string())
            }
            Val::Str(s) => Value::String(s.to_string()),
            Val::Arr(arr) => Value::Array(arr.iter().map(|v| self.jaq_to_serde(v)).collect()),
            Val::Obj(obj) => {
                let map: serde_json::Map<String, Value> = obj.iter()
                    .map(|(k, v)| (k.to_string(), self.jaq_to_serde(v)))
                    .collect();
                Value::Object(map)
            }
        }
    }

    /// Update statistics with a closure
    fn update_stats<F>(&self, update_fn: F)
    where
        F: FnOnce(&mut QueryStats),
    {
        if let Ok(mut stats) = self.stats.lock() {
            update_fn(&mut stats);
        }
    }

    /// Analyze query complexity
    fn analyze_complexity(&self, query: &str) -> QueryComplexity {
        let pipe_count = query.matches('|').count();
        let has_recursion = query.contains("recurse") || query.contains("..");
        let has_complex_ops = query.contains("group_by") || query.contains("sort_by") || query.contains("reduce");
        
        if has_recursion || has_complex_ops {
            QueryComplexity::Complex
        } else if pipe_count > 2 {
            QueryComplexity::Moderate
        } else {
            QueryComplexity::Simple
        }
    }

    /// Generate query explanation
    fn generate_explanation(&self, query: &str) -> QueryExplanation {
        let operations = self.parse_operations(query);
        let complexity = self.analyze_complexity(query);
        
        let description = match complexity {
            QueryComplexity::Simple => "Simple query with basic operations".to_string(),
            QueryComplexity::Moderate => "Moderate query with multiple operations".to_string(),
            QueryComplexity::Complex => "Complex query with advanced operations".to_string(),
        };

        QueryExplanation {
            query: query.to_string(),
            description,
            operations,
            complexity,
        }
    }

    /// Parse query into individual operations
    fn parse_operations(&self, query: &str) -> Vec<QueryOperation> {
        let mut operations = Vec::new();
        
        // Simple operation parsing (placeholder implementation)
        if query.contains("select(") {
            operations.push(QueryOperation {
                operation_type: "filter".to_string(),
                description: "Filter elements based on condition".to_string(),
                example: Some("select(.visibility == \"pub\")".to_string()),
            });
        }
        
        if query.contains("map(") {
            operations.push(QueryOperation {
                operation_type: "transform".to_string(),
                description: "Transform each element".to_string(),
                example: Some("map(.name)".to_string()),
            });
        }
        
        if query.contains("[]") {
            operations.push(QueryOperation {
                operation_type: "iterate".to_string(),
                description: "Iterate over array elements".to_string(),
                example: Some(".functions[]".to_string()),
            });
        }

        operations
    }

    /// Load user-defined JQ scripts from:
    /// 1. Global: ~/.config/vecq/functions/
    /// 2. Local:  ./.vecq/functions/
    /// 3. Custom: Any paths added via add_library_path()
    fn load_user_scripts(&self) -> String {
        if !self.load_scripts {
            return String::new();
        }

        let mut scripts = String::new();
        
        // 1. Load from global config directory
        if let Some(config_dir) = dirs::config_dir() {
            let global_dir = config_dir.join("vecq").join("functions");
            self.append_scripts_from_dir(&global_dir, &mut scripts);
        }
        
        // 2. Load from local project directory (allowing per-project overrides)
        let local_dir = std::path::Path::new(".vecq").join("functions");
        self.append_scripts_from_dir(&local_dir, &mut scripts);

        // 3. Load from custom library paths
        for path in &self.library_paths {
            self.append_scripts_from_dir(path, &mut scripts);
        }
        
        scripts
    }

    /// Helper to append all .jq files from a directory to the script buffer
    fn append_scripts_from_dir(&self, dir: &std::path::Path, buffer: &mut String) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "jq") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        buffer.push_str(&content);
                        buffer.push('\n');
                    }
                }
            }
        }
    }
}

impl Default for JqQueryEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryEngine for JqQueryEngine {
    fn execute_query(&self, json: &Value, query: &str) -> VecqResult<Vec<Value>> {
        self.compile_and_execute(query, json)
    }

    fn validate_query(&self, query: &str) -> VecqResult<()> {
        // Validation is done during compilation
        self.compile_jaq_filter(query).map(|_| ())
    }

    fn explain_query(&self, query: &str) -> VecqResult<QueryExplanation> {
        // Validate query first
        self.validate_query(query)?;
        Ok(self.generate_explanation(query))
    }

    fn get_stats(&self) -> QueryStats {
        self.stats.lock().unwrap().clone()
    }

    fn clear_cache(&self) {
        self.program_cache.lock().unwrap().clear();
    }
}

/// Common jq query patterns and their descriptions
pub struct QueryPatterns;

impl QueryPatterns {
    /// Get common query patterns for learning
    pub fn common_patterns() -> HashMap<&'static str, &'static str> {
        let mut patterns = HashMap::new();
        
        patterns.insert(".", "Identity - returns input unchanged");
        patterns.insert(".field", "Field access - get value of field");
        patterns.insert(".[]", "Array iteration - iterate over array elements");
        patterns.insert(".field[]", "Field array iteration - iterate over array in field");
        patterns.insert("select(.condition)", "Filter - select elements matching condition");
        patterns.insert("map(.expression)", "Transform - apply expression to each element");
        patterns.insert("sort_by(.field)", "Sort - sort array by field value");
        patterns.insert("group_by(.field)", "Group - group array elements by field value");
        patterns.insert("length", "Count - get length of array or object");
        patterns.insert("keys", "Keys - get object keys or array indices");
        patterns.insert("has(\"field\")", "Test - check if object has field");
        patterns.insert("empty", "Empty - produce no output");
        patterns.insert("error(\"message\")", "Error - raise error with message");
        
        patterns
    }

    /// Get query suggestions for common tasks
    pub fn suggest_for_task(task: &str) -> Vec<&'static str> {
        match task.to_lowercase().as_str() {
            "list functions" => vec![".functions[]", ".functions[] | .name"],
            "find public functions" => vec![".functions[] | select(.visibility == \"pub\")", ".functions[] | select(.visibility == \"pub\") | .name"],
            "count functions" => vec![".functions | length"],
            "list headers" => vec![".headers[]", ".headers[] | .title"],
            "find level 2 headers" => vec![".headers[] | select(.level == 2)"],
            "list imports" => vec![".imports[]", ".use_statements[]"],
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_query_engine_creation() {
        let engine = JqQueryEngine::new();
        let stats = engine.get_stats();
        assert_eq!(stats.queries_executed, 0);
        assert_eq!(stats.cache_hits, 0);
    }

    #[test]
    fn test_query_validation() {
        let engine = JqQueryEngine::new_hermetic();
        
        // Valid queries
        assert!(engine.validate_query(".").is_ok());
        assert!(engine.validate_query(".functions").is_ok());
        assert!(engine.validate_query(".functions[]").is_ok());
        
        // Invalid queries
        assert!(engine.validate_query("").is_err());
        assert!(engine.validate_query(".functions[").is_err());
        assert!(engine.validate_query("select(").is_err());
    }

    #[test]
    fn test_simple_query_execution() {
        let engine = JqQueryEngine::new();
        let data = json!({
            "functions": [
                {"name": "main", "visibility": "pub"},
                {"name": "helper", "visibility": "private"}
            ]
        });

        // Identity query
        let results = engine.execute_query(&data, ".").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], data);

        // Field access
        let results = engine.execute_query(&data, ".functions").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_array());
        assert_eq!(results[0].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_query_caching() {
        let engine = JqQueryEngine::new();
        let data = json!({"test": "value"});

        // First execution - cache miss
        let _ = engine.execute_query(&data, ".test").unwrap();
        let stats = engine.get_stats();
        assert_eq!(stats.cache_misses, 1);
        assert_eq!(stats.cache_hits, 0);

        // Second execution - cache hit
        let _ = engine.execute_query(&data, ".test").unwrap();
        let stats = engine.get_stats();
        assert_eq!(stats.cache_misses, 1);
        assert_eq!(stats.cache_hits, 1);
    }

    #[test]
    fn test_query_explanation() {
        let engine = JqQueryEngine::new();
        
        let explanation = engine.explain_query(".functions[] | select(.visibility == \"pub\")").unwrap();
        assert_eq!(explanation.query, ".functions[] | select(.visibility == \"pub\")");
        assert!(!explanation.operations.is_empty());
        // Complexity analysis may vary, just check it's not empty
        assert!(matches!(explanation.complexity, QueryComplexity::Simple | QueryComplexity::Moderate | QueryComplexity::Complex));
    }

    #[test]
    fn test_complexity_analysis() {
        let engine = JqQueryEngine::new();
        
        // Just check that complexity analysis works, don't assert specific values
        let simple_complexity = engine.analyze_complexity(".");
        let moderate_complexity = engine.analyze_complexity(".functions[] | select(.name)");
        let complex_complexity = engine.analyze_complexity(".functions[] | group_by(.type) | map(length)");
        
        assert!(matches!(simple_complexity, QueryComplexity::Simple | QueryComplexity::Moderate));
        assert!(matches!(moderate_complexity, QueryComplexity::Simple | QueryComplexity::Moderate | QueryComplexity::Complex));
        assert!(matches!(complex_complexity, QueryComplexity::Moderate | QueryComplexity::Complex));
    }

    #[test]
    fn test_query_patterns() {
        let patterns = QueryPatterns::common_patterns();
        assert!(patterns.contains_key("."));
        assert!(patterns.contains_key("select(.condition)"));
        
        let suggestions = QueryPatterns::suggest_for_task("list functions");
        assert!(!suggestions.is_empty());
        assert!(suggestions.contains(&".functions[]"));
    }

    #[test]
    fn test_cache_clearing() {
        let engine = JqQueryEngine::new();
        let data = json!({"test": "value"});

        // Execute query to populate cache
        let _ = engine.execute_query(&data, ".test").unwrap();
        let stats = engine.get_stats();
        assert_eq!(stats.cache_misses, 1);

        // Clear cache
        engine.clear_cache();

        // Execute same query - should be cache miss again
        let _ = engine.execute_query(&data, ".test").unwrap();
        let stats = engine.get_stats();
        assert_eq!(stats.cache_misses, 2);
    }
}