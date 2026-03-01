use crate::ingestion::IngestionOptions;
use ignore::WalkBuilder;

pub fn build_walker(options: &IngestionOptions) -> WalkBuilder {
    let mut builder = WalkBuilder::new(&options.path);
    builder
        .git_ignore(options.respect_gitignore)
        .ignore(true)
        .hidden(false)
        .add_custom_ignore_filename(".vectorignore");
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
