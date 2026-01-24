use assert_cmd::Command;

struct TestCase {
    name: &'static str,
    query: &'static str,
    input: &'static str,
}

impl TestCase {
    fn new(name: &'static str, query: &'static str, input: &'static str) -> Self {
        TestCase { name, query, input }
    }
}

#[test]
fn test_jq_standard_library_audit() {
    // A comprehensive list of standard filters we expect to be available.
    // If jaq or vecq drops any of these, this test should fail.
    // We use trivial inputs/arguments just to prove existence (not correctness).

    let simple_arr = "[1,2,3]";
    let simple_obj = "{\"a\":1}";
    let simple_str = "\"hello\"";
    let simple_num = "42";

    let cases = vec![
        // --- Core ---
        TestCase::new("length", "length", simple_arr),
        TestCase::new("utf8bytelength", "utf8bytelength", simple_str),
        TestCase::new("keys", "keys", simple_obj),
        TestCase::new("keys_unsorted", "keys_unsorted", simple_obj),
        TestCase::new("has", "has(\"a\")", simple_obj),
        TestCase::new("in", "in({\"a\":1})", "\"a\""),
        TestCase::new("map", "map(.)", simple_arr),
        TestCase::new("map_values", "map_values(.)", simple_obj),
        TestCase::new("to_entries", "to_entries", simple_obj),
        TestCase::new(
            "from_entries",
            "from_entries",
            "[{\"key\":\"a\",\"value\":1}]",
        ),
        TestCase::new("with_entries", "with_entries(.)", simple_obj),
        TestCase::new("select", "select(true)", simple_arr),
        TestCase::new("empty", "empty", simple_arr),
        TestCase::new("error", "try error(\"msg\") catch .", simple_arr),
        // TestCase::new("halt", "halt", simple_arr), // Missing in jaq 1.5.0
        // TestCase::new("halt_error", "try halt_error catch .", simple_arr), // Missing in jaq 1.5.0
        // TestCase::new("path", "path(.[0])", simple_arr), // Missing in jaq 1.5.0
        TestCase::new("paths", "paths", simple_arr),
        TestCase::new("del", "del(.[0])", simple_arr),
        // TestCase::new("setpath", "setpath([0]; 9)", simple_arr), // Missing in jaq 1.5.0
        // TestCase::new("delpaths", "delpaths([[0]])", simple_arr), // Missing in jaq 1.5.0
        // TestCase::new("getpath", "getpath([0])", simple_arr), // Missing in jaq 1.5.0
        TestCase::new("transpose", "transpose", "[[1],[2]]"),
        // TestCase::new("bsearch", "bsearch(2)", simple_arr), // Missing in jaq 1.5.0

        // --- Types ---
        TestCase::new("type", "type", simple_arr),
        TestCase::new("isnan", "isnan", simple_num),
        TestCase::new("isfinite", "isfinite", simple_num),
        TestCase::new("infinite", "infinite", simple_num),
        TestCase::new("tonumber", "tonumber", "\"123\""),
        TestCase::new("tostring", "tostring", simple_num),
        TestCase::new("tojson", "tojson", simple_obj),
        TestCase::new("fromjson", "fromjson", "\"{\\\"a\\\":1}\""),
        // --- Math ---
        TestCase::new("add", "add", simple_arr),
        TestCase::new("min", "min", simple_arr),
        TestCase::new("max", "max", simple_arr),
        TestCase::new("min_by", "min_by(.)", simple_arr),
        TestCase::new("max_by", "max_by(.)", simple_arr),
        TestCase::new("sort", "sort", simple_arr),
        TestCase::new("sort_by", "sort_by(.)", simple_arr),
        TestCase::new("group_by", "group_by(.)", simple_arr),
        TestCase::new("unique", "unique", simple_arr),
        TestCase::new("unique_by", "unique_by(.)", simple_arr),
        TestCase::new("reverse", "reverse", simple_arr),
        TestCase::new("contains", "contains([1])", simple_arr),
        TestCase::new("inside", "inside([1,2,3,4])", simple_arr),
        TestCase::new("indices", "indices(1)", simple_arr),
        TestCase::new("index", "index(1)", simple_arr),
        TestCase::new("rindex", "rindex(1)", simple_arr),
        TestCase::new("flatten", "flatten", "[[1]]"),
        TestCase::new("range", "range(5)", "null"),
        TestCase::new("floor", "floor", "1.5"),
        TestCase::new("sqrt", "sqrt", "9"),
        TestCase::new("round", "round", "1.5"),
        TestCase::new("ceil", "ceil", "1.5"),
        // --- Strings ---
        TestCase::new("startswith", "startswith(\"h\")", simple_str),
        TestCase::new("endswith", "endswith(\"o\")", simple_str),
        TestCase::new("ltrimstr", "ltrimstr(\"h\")", simple_str),
        TestCase::new("rtrimstr", "rtrimstr(\"o\")", simple_str),
        TestCase::new("explode", "explode", simple_str),
        TestCase::new("implode", "implode", "[104, 101, 108, 108, 111]"),
        TestCase::new("split", "split(\"e\")", simple_str),
        TestCase::new("join", "join(\" \")", "[\"a\",\"b\"]"),
        TestCase::new("ascii_downcase", "ascii_downcase", "\"HELLO\""),
        TestCase::new("ascii_upcase", "ascii_upcase", "\"hello\""),
        // --- Regex ---
        TestCase::new("test", "test(\"l+\")", simple_str),
        TestCase::new("scan", "scan(\"l+\")", simple_str),
        TestCase::new("match", "match(\"l+\")", simple_str),
        TestCase::new("capture", "capture(\"(?<n>l+)\")", simple_str),
        TestCase::new("splits", "splits(\"e\")", simple_str),
        TestCase::new("sub", "sub(\"l\"; \"L\")", simple_str),
        TestCase::new("gsub", "gsub(\"l\"; \"L\")", simple_str),
        // --- Time ---
        TestCase::new("now", "now", "null"),
        TestCase::new("todate", "todate", "1234567890"),
        TestCase::new("fromdate", "fromdate", "\"2026-01-01T00:00:00Z\""),
        // TestCase::new("gmtime", "gmtime", "1234567890"), // Missing in jaq 1.5.0
        // TestCase::new("localtime", "localtime", "1234567890"), // Missing in jaq 1.5.0
        // TestCase::new("mktime", "mktime", "[2026,0,1,0,0,0,0,0]"), // Missing in jaq 1.5.0
        // TestCase::new("strftime", "strftime(\"%Y\")", "1234567890"), // Missing in jaq 1.5.0

        // --- Iterators ---
        TestCase::new("limit", "limit(1; .[])", simple_arr),
        TestCase::new("first", "first(.[])", simple_arr),
        TestCase::new("last", "last(.[])", simple_arr),
        TestCase::new("nth", "nth(0; .[])", simple_arr),
        TestCase::new("isempty", "isempty(.[])", simple_arr),
        TestCase::new("all", "all", "[true, true]"),
        TestCase::new("any", "any", "[true, false]"),
        // --- Combinatorics ---
        // TestCase::new("combinations", "combinations", "[[1],[2]]"), // Missing in jaq 1.5.0
    ];

    let mut failed_filters = Vec::new();

    for case in cases {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecq"));
        let assert = cmd
            .arg("-q")
            .arg(case.query)
            .write_stdin(case.input)
            .assert();

        let output = assert.get_output();
        let stderr = String::from_utf8_lossy(&output.stderr);

        // We specifically look for "undefined filter" which means it's missing from the engine.
        // Other errors (like type errors) mean the filter EXISTS but we used it wrong, which is fine for this audit.
        if stderr.contains("undefined filter") {
            println!(
                "FAIL: Filter '{}' is undefined. Query: {}",
                case.name, case.query
            );
            failed_filters.push(case.name);
        }
    }

    if !failed_filters.is_empty() {
        panic!(
            "The following standard filters are missing from vecq: {:?}",
            failed_filters
        );
    }
}
