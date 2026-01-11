use ignore::WalkBuilder;
use crate::ingestion::IngestionOptions;

pub fn build_walker(options: &IngestionOptions) -> WalkBuilder {
    let mut builder = WalkBuilder::new(&options.path);
    builder
        .standard_filters(false) // Disable standard filters to control them manually
        .git_ignore(options.respect_gitignore) // Optional .gitignore
        .ignore(true)            // Always respect .ignore (and custom ignore files)
        .parents(true)           // Look in parent directories for ignore files
        .hidden(false)           // Allow hidden files
        .add_custom_ignore_filename(".vectorignore"); // Prioritize .vectorignore
    builder
}

pub fn count_files(builder: &WalkBuilder) -> u64 {
    let count_walker = builder.build();
    count_walker
        .filter_map(|r| r.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter(|e| !e.path().components().any(|c| c.as_os_str() == ".vecdb"))
        .count() as u64
}
