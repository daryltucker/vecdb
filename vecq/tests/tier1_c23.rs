// Test that C23 language features parse without errors
use vecq::{parse_file, FileType};

#[tokio::test]
async fn test_c23_typeof() {
    // C23 typeof feature
    let content = r#"
#include <stdio.h>

int main() {
    int x = 42;
    typeof(x) y = x;  // C23 typeof
    printf("%d\n", y);
    return 0;
}
"#;
    let result = parse_file(content, FileType::C).await;
    assert!(result.is_ok(), "C23 typeof should parse without errors");
}

#[tokio::test]
async fn test_c23_auto() {
    // C23 auto type inference
    let content = r#"
int main() {
    auto x = 42;  // C23 auto
    return 0;
}
"#;
    let result = parse_file(content, FileType::C).await;
    assert!(result.is_ok(), "C23 auto should parse without errors");
}
