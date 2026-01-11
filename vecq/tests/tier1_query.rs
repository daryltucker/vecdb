use vecq::{parse_file, convert_to_json, query_json, FileType};

#[tokio::test]
async fn test_basic_query_rust() {
    let content = r#"
    pub fn main() {
        println!("Hello");
    }
    
    fn private_helper() {}
    "#;
    let parsed = parse_file(content, FileType::Rust).await.unwrap();
    let json = convert_to_json(parsed).unwrap();
    println!("Rust JSON: {}", serde_json::to_string_pretty(&json).unwrap());
    
    // Test 1: Get all functions
    let result = query_json(&json, ".functions").unwrap();
    assert!(result.is_array());
    let funcs = result.as_array().unwrap();
    assert_eq!(funcs.len(), 2);
    
    // Test 2: Filter public functions
    let result = query_json(&json, ".functions[] | select(.attributes.visibility == \"pub\")").unwrap();
    println!("Filter Result: {:?}", result);
    if let Some(obj) = result.as_object() {
        println!("Keys: {:?}", obj.keys());
    }
    // Should be a single object since there's only one match
    assert!(result.is_object());
    assert_eq!(result.get("name").and_then(|v| v.as_str()), Some("main"));

    // Test 3: Get names of all functions
    let result = query_json(&json, ".functions[] | .name").unwrap();
    println!("Names Result: {:?}", result);
    assert!(result.is_array()); // Should be ["main", "private_helper"]

    let names: Vec<String> = result.as_array().unwrap().iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"main".to_string()));
    assert!(names.contains(&"private_helper".to_string()));
}

#[tokio::test]
async fn test_python_classes() {
    let content = r#"
class Dog:
    def bark(self): pass

class Cat:
    def meow(self): pass
    "#;
    let parsed = parse_file(content, FileType::Python).await.unwrap();
    let json = convert_to_json(parsed).unwrap();
    println!("Python JSON: {}", serde_json::to_string_pretty(&json).unwrap());

    // Debug: Check literal query
    let literal = query_json(&json, "1").unwrap();
    println!("Literal 1: {:?}", literal);
    assert_eq!(literal, serde_json::Value::Number(serde_json::Number::from(1)));

    // Debug: Check identity
    let identity = query_json(&json, ".").unwrap();
    assert!(identity.is_object());

    // Query class names using .classes (plural) as seen in JSON output
    let result = query_json(&json, ".classes[] | .name").unwrap();
    println!("Python Names Result: {:?}", result);
    // Expecting ["Dog", "Cat"]
    assert!(result.is_array()); 
    let names: Vec<&str> = result.as_array().unwrap().iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(names.contains(&"Dog"));
    assert!(names.contains(&"Cat"));
}

#[tokio::test]
async fn test_empty_query() {
    let content = "fn foo() {}";
    let parsed = parse_file(content, FileType::Rust).await.unwrap();
    let json = convert_to_json(parsed).unwrap();

    let result = query_json(&json, ".").unwrap();
    assert!(result.is_object());
    assert!(result.get("functions").is_some());
}
