/// Primary function for testing docstring extraction
/// 
/// # Purpose
/// This function exists to verify that RustParser correctly identifies
/// and extracts these lines as `docstring`.
pub fn primary_function() {
    println!("Hello");
}

/// Helper struct with documentation
pub struct Helper {
    /// Field documentation
    field: i32,
}
