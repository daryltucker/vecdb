// PURPOSE:
//   jq-compatible query engine using jaq 3.0

use crate::error::{VecqError, VecqResult};
use lru::LruCache;
use serde_json::Value;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, LazyLock};
use std::fs;
use std::rc::Rc;

// jaq imports
use jaq_core::{self, Ctx, Filter, Vars};
use jaq_core::data::JustLut;
use jaq_core::load::{Arena, File, Loader};
use jaq_json::{self, Val};

// Normalizers
const NORM_LOG_NGINX: &str = include_str!("stdlib/normalizers/log_nginx.jq");
const NORM_LOG_JOURNALD: &str = include_str!("stdlib/normalizers/log_journald.jq");
const NORM_TASK_GITHUB: &str = include_str!("stdlib/normalizers/task_github.jq");
const NORM_TASK_TODO: &str = include_str!("stdlib/normalizers/task_todo.jq");
const NORM_TASK_SRC: &str = include_str!("stdlib/normalizers/task_src.jq");
const NORM_CHAT_WEBUI: &str = include_str!("stdlib/normalizers/chat_openwebui.jq");
const NORM_ARTIFACT_CARGO: &str = include_str!("stdlib/normalizers/artifact_cargo.jq");
const NORM_DIFF_GIT: &str = include_str!("stdlib/normalizers/diff_git.jq");
const NORM_GRAPH_SRC: &str = include_str!("stdlib/normalizers/graph_src.jq");
const NORM_GRAPH_TO_ARCH: &str = include_str!("stdlib/normalizers/graph_to_architecture.jq");

// Renderers
const RENDER_CHAT: &str = include_str!("stdlib/renderers/chat.jq");
const RENDER_DOC: &str = include_str!("stdlib/renderers/doc.jq");
const RENDER_TASK: &str = include_str!("stdlib/renderers/task.jq");
const RENDER_ARTIFACT: &str = include_str!("stdlib/renderers/artifact.jq");
const RENDER_DIFF: &str = include_str!("stdlib/renderers/diff.jq");
const RENDER_LOG: &str = include_str!("stdlib/renderers/log.jq");
const RENDER_GRAPH: &str = include_str!("stdlib/renderers/graph.jq");
const RENDER_ARCH: &str = include_str!("stdlib/renderers/architecture.jq");

// Logic
const AUTO_JQ: &str = include_str!("stdlib/auto.jq");

const REGEX_PRELUDE: &str = r#"
def test($r): [., $r] | _native_test;
def test($r; $flags): [., $r, $flags] | _native_test;
def capture($r): [., $r] | _native_capture;
def capture($r; $flags): [., $r, $flags] | _native_capture;
def match($r): [., $r] | _native_match;
def match($r; $flags): [., $r, $flags] | _native_match;
def scan($r): match($r) | if (.captures | length > 0) then [.captures[].string] else .string end;
def scan($r; $flags): match($r; $flags) | if (.captures | length > 0) then [.captures[].string] else .string end;
def splits($r): [., $r] | _native_splits;
def splits($r; $flags): [., $r, $flags] | _native_splits;
def sub($r; $s): [., $r, $s] | _native_sub;
def sub($r; $s; $flags): [., $r, $s, $flags] | _native_sub;
def gsub($r; $s): [., $r, $s] | _native_gsub;
def gsub($r; $s; $flags): [., $r, $s, $flags] | _native_gsub;
def re_test($r): test($r);
def re_test($r; $f): test($r; $f);
def re_capture($r): capture($r);
def re_capture($r; $f): capture($r; $f);
def re_sub($r; $s): sub($r; $s);
def re_sub($r; $s; $f): sub($r; $s; $f);
"#;

static PRELUDE_SOURCE: LazyLock<String> = LazyLock::new(|| {
    format!("{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}", 
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
        RENDER_GRAPH,
        RENDER_ARCH,
        AUTO_JQ,
        NORM_GRAPH_SRC,
        NORM_GRAPH_TO_ARCH
    )
});

pub trait QueryEngine: Send + Sync {
    fn execute_query(&self, json: &Value, query: &str) -> VecqResult<Vec<Value>>;
    fn validate_query(&self, query: &str) -> VecqResult<()>;
    fn explain_query(&self, query: &str) -> VecqResult<QueryExplanation>;
    fn get_stats(&self) -> QueryStats;
    fn clear_cache(&self);
}

#[derive(Debug, Clone)]
pub struct QueryExplanation {
    pub query: String,
    pub description: String,
    pub operations: Vec<QueryOperation>,
    pub complexity: QueryComplexity,
}

#[derive(Debug, Clone)]
pub struct QueryOperation {
    pub operation_type: String,
    pub description: String,
    pub example: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryComplexity {
    Simple,
    Moderate,
    Complex,
}

#[derive(Debug, Clone, Default)]
pub struct QueryStats {
    pub queries_executed: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub compilation_errors: u64,
    pub execution_errors: u64,
    pub total_execution_time_ms: u64,
}

pub struct JqQueryEngine {
    program_cache: Mutex<LruCache<String, CompiledQuery>>,
    stats: Mutex<QueryStats>,
    library_paths: Vec<std::path::PathBuf>,
    load_scripts: bool,
}

// Fix: Filter is not Clone in 3.0 because Native not Clone. Wrap in Arc.
struct CompiledQuery {
    filter: Arc<Filter<jaq_core::data::JustLut<Val>>>,
    use_count: u64,
}

impl JqQueryEngine {
    pub fn new() -> Self {
        Self::with_cache_size(100)
    }

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

    pub fn new_hermetic() -> Self {
        Self {
            program_cache: Mutex::new(LruCache::new(NonZeroUsize::new(100).unwrap())),
            stats: Mutex::new(QueryStats::default()),
            library_paths: Vec::new(),
            load_scripts: false,
        }
    }

    pub fn add_library_path(&mut self, path: std::path::PathBuf) {
        self.library_paths.push(path);
    }

    fn compile_and_execute(&self, query: &str, json: &Value) -> VecqResult<Vec<Value>> {
        let start_time = std::time::Instant::now();
        
        // Cache logic
        let filter_arc = {
            let mut cache = self.program_cache.lock().unwrap();
            
            if let Some(compiled) = cache.get_mut(query) {
                compiled.use_count += 1;
                self.update_stats(|stats| stats.cache_hits += 1);
                compiled.filter.clone()
            } else {
                drop(cache); // Drop lock to compile
                
                let filter = self.compile_jaq_filter(query)?;
                let arc_filter = Arc::new(filter);
                
                let mut cache = self.program_cache.lock().unwrap();
                cache.put(query.to_string(), CompiledQuery {
                    filter: arc_filter.clone(),
                    use_count: 1,
                });
                self.update_stats(|stats| stats.cache_misses += 1);
                
                arc_filter
            }
        };
        
        let results = self.execute_jaq_filter(&filter_arc, json)?;
        
        let execution_time = start_time.elapsed().as_millis() as u64;
        self.update_stats(|stats| {
            stats.queries_executed += 1;
            stats.total_execution_time_ms += execution_time;
        });
        
        Ok(results)
    }

    fn compile_jaq_filter(&self, query: &str) -> VecqResult<Filter<jaq_core::data::JustLut<Val>>> {
        if query.trim().is_empty() {
            return Err(VecqError::query_error(
                query.to_string(),
                "Empty query".to_string(),
                Some("Try: . (identity filter)".to_string()),
            ));
        }

        let user_scripts = self.load_user_scripts();

        let prelude = &*PRELUDE_SOURCE;
        let full_source = format!("{}\n{}\n{}", prelude, user_scripts, query);
        
        let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));
        let arena = Arena::default();

        let program = File { code: full_source.as_str(), path: () };
        let modules = loader.load(&arena, program).map_err(|e| {
             let error_msgs: Vec<String> = e.iter()
                .map(|(file, undefined)| format!("File: {:?}, Undefined: {:?}", file, undefined))
                .collect();
             VecqError::query_error(
                query.to_string(),
                error_msgs.join("; "),
                Some("Check jq syntax".to_string()),
            )
        })?;

        let compiler = jaq_core::Compiler::default()
            .with_funs(jaq_std::funs().chain(jaq_json::funs()).chain(crate::natives::regex_natives()));

        let filter = compiler.compile(modules).map_err(|e| {
             let error_msgs: Vec<String> = e.iter()
                .map(|e| format!("{:?}", e))
                .collect();
             VecqError::query_error(
                query.to_string(),
                error_msgs.join("; "),
                Some("Check jq syntax and functions".to_string()),
            )
        })?;
        
        Ok(filter)
    }

    fn execute_jaq_filter(&self, filter: &Filter<jaq_core::data::JustLut<Val>>, json: &Value) -> VecqResult<Vec<Value>> {
        let input = self.serde_to_jaq(json);
        
        let ctx: Ctx<'_, JustLut<Val>> = Ctx::new(&filter.lut, Vars::new([]));
        
        let results: Vec<Val> = filter.id.run((ctx, input))
            .filter_map(|r| r.ok())
            .collect();
        
        Ok(results.iter().map(|v| self.jaq_to_serde(v)).collect())
    }

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
                     Val::from(n.as_f64().unwrap_or(0.0))
                }
            }
            Value::String(s) => Val::from(s.clone()),
            Value::Array(arr) => {
                let vec: Vec<Val> = arr.iter().map(|v| self.serde_to_jaq(v)).collect();
                Val::Arr(Rc::new(vec))
            },
            Value::Object(obj) => {
                // Fix: Uses foldhash for IndexMap to match Val expectation
                use indexmap::IndexMap;
                use foldhash::fast::RandomState;
                let mut map = IndexMap::with_hasher(RandomState::default());
                for (k, v) in obj {
                    map.insert(Val::from(k.clone()), self.serde_to_jaq(v));
                }
                Val::obj(map)
            }
        }
    }

    fn jaq_to_serde(&self, val: &Val) -> Value {
        match val {
            Val::Null => Value::Null,
            Val::Bool(b) => Value::Bool(*b),
            Val::Num(n) => {
                 let s = n.to_string();
                 if let Ok(v) = serde_json::from_str(&s) {
                     v
                 } else {
                     Value::Number(serde_json::Number::from_f64(0.0).unwrap())
                 }
            }
            Val::Str(s, _) => Value::String(String::from_utf8_lossy(s.as_ref()).to_string()),
            Val::Arr(arr) => Value::Array(arr.iter().map(|v| self.jaq_to_serde(v)).collect()),
            Val::Obj(obj) => {
                let map: serde_json::Map<String, Value> = obj.iter()
                    .map(|(k, v)| (
                        match k {
                            Val::Str(s, _) => String::from_utf8_lossy(s.as_ref()).to_string(),
                            _ => k.to_string(), // Best effort for non-string keys
                        }, 
                        self.jaq_to_serde(v)
                    ))
                    .collect();
                Value::Object(map)
            }
        }
    }

    fn update_stats<F>(&self, update_fn: F)
    where
        F: FnOnce(&mut QueryStats),
    {
        if let Ok(mut stats) = self.stats.lock() {
            update_fn(&mut stats);
        }
    }

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

    fn parse_operations(&self, query: &str) -> Vec<QueryOperation> {
        let mut operations = Vec::new();
        
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

    fn load_user_scripts(&self) -> String {
        if !self.load_scripts {
            return String::new();
        }

        let mut scripts = String::new();
        
        if let Some(config_dir) = dirs::config_dir() {
            let global_dir = config_dir.join("vecq").join("functions");
            self.append_scripts_from_dir(&global_dir, &mut scripts);
        }
        
        let local_dir = std::path::Path::new(".vecq").join("functions");
        self.append_scripts_from_dir(&local_dir, &mut scripts);

        for path in &self.library_paths {
            self.append_scripts_from_dir(path, &mut scripts);
        }
        
        scripts
    }

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
        self.compile_jaq_filter(query).map(|_| ())
    }

    fn explain_query(&self, query: &str) -> VecqResult<QueryExplanation> {
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

pub struct QueryPatterns;

impl QueryPatterns {
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
        
        assert!(engine.validate_query(".").is_ok());
        assert!(engine.validate_query(".functions").is_ok());
        assert!(engine.validate_query(".functions[]").is_ok());
        
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

        let results = engine.execute_query(&data, ".").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], data);

        let results = engine.execute_query(&data, ".functions").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_array());
        assert_eq!(results[0].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_query_caching() {
        let engine = JqQueryEngine::new();
        let data = json!({"test": "value"});

        let _ = engine.execute_query(&data, ".test").unwrap();
        let stats = engine.get_stats();
        assert_eq!(stats.cache_misses, 1);
        assert_eq!(stats.cache_hits, 0);

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
        assert!(matches!(explanation.complexity, QueryComplexity::Simple | QueryComplexity::Moderate | QueryComplexity::Complex));
    }

    #[test]
    fn test_complexity_analysis() {
        let engine = JqQueryEngine::new();
        
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

        let _ = engine.execute_query(&data, ".test").unwrap();
        let stats = engine.get_stats();
        assert_eq!(stats.cache_misses, 1);

        engine.clear_cache();

        let _ = engine.execute_query(&data, ".test").unwrap();
        let stats = engine.get_stats();
        assert_eq!(stats.cache_misses, 2);
    }
}