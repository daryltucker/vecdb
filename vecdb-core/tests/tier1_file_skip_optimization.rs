use std::fs::File;
use std::io::Write;
use tempfile::tempdir;
use vecdb_core::state;

#[test]
fn test_compute_file_metadata_hash_changes() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test_file.txt");

    // Create initial file
    {
        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"Hello world").unwrap();
    }

    let hash1 = state::compute_file_metadata_hash(&file_path).unwrap();
    assert!(!hash1.is_empty(), "Hash should not be empty");

    // Wait a moment to ensure mtime changes (filesystems have resolution limits)
    std::thread::sleep(std::time::Duration::from_millis(1010));

    // Modify file
    {
        let mut file = File::create(&file_path).unwrap(); // Overwrite
        file.write_all(b"Hello changed world").unwrap();
    }

    let hash2 = state::compute_file_metadata_hash(&file_path).unwrap();
    assert_ne!(hash1, hash2, "Hash should verify file modification");
}

#[test]
fn test_compute_file_metadata_hash_stable() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("stable_file.txt");

    {
        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"Stable content").unwrap();
    }

    let hash1 = state::compute_file_metadata_hash(&file_path).unwrap();

    // Read file (shouldn't change mtime/size usually, but let's just re-compute)
    let hash2 = state::compute_file_metadata_hash(&file_path).unwrap();

    assert_eq!(hash1, hash2, "Hash should be stable if file unchanged");
}
