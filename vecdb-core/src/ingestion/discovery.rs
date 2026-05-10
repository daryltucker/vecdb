use crate::ingestion::IngestionOptions;
use ignore::WalkBuilder;

pub fn build_walker(options: &IngestionOptions) -> WalkBuilder {
    let mut builder = WalkBuilder::new(&options.path);
    builder
        .git_ignore(options.respect_gitignore)
        .ignore(options.respect_gitignore)
        .hidden(false);

    if !options.ignore_vectorignore {
        builder.add_custom_ignore_filename(".vectorignore");
    }

    // Always exclude configuration/ignore files — they are not ingestable content
    builder.filter_entry(move |entry| {
        let name = entry.file_name();
        name != ".vecdbrc" && name != ".vectorignore" && name != ".gitignore"
    });

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
