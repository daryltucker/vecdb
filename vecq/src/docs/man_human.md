# VECQ(1) - jq for source code

## SYNOPSIS
    vecq [OPTIONS] <INPUT> [QUERY]
    vecq --convert <INPUT>
    vecq map <PATH> [OPTIONS]
    vecq man [COMMAND] [--agent]

## DESCRIPTION
    **vecq** converts any structured document (source code, markdown, etc.) into
    queryable JSON and enables jq-like querying with natural language support.
    
    Think "jq for source code" - making programmatic document interaction as
    natural as working with JSON APIs.

## COMMANDS

### Essential
    * **query** <INPUT> [QUERY]
        Query a file or directory. The query is optional if you just want to see the structure.
        Example: `vecq src/main.rs '.functions[]'`

    * **map** <PATH> [-d DEPTH] [-o FORMAT] [-s]
        Analyze a directory and produce a structural graph of files, modules, and symbols.
        Output formats: mermaid (default), json, graph, human.
        Example: `vecq map ./src -o mermaid > architecture.md`
        Example: `vecq map ./src -s` (show statistics)

    * **convert** --convert <INPUT>
        Convert file to JSON directly without querying.
        Example: `vecq --convert README.md`

    * **syntax** <INPUT> [-l LANGUAGE]
        Display syntax highlighted file or stdin.
        Example: `vecq syntax src/main.rs` or `cat file.md | vecq syntax -l md`

    * **man** [COMMAND] [--agent]
        Display this manual.
        Use `--agent` to see the Agent Context Specification.
        Use `man <command>` to see help for a specific command.

    * **doc** <INPUT>
        Generate Markdown documentation from source.
        Example: `vecq doc src/lib.rs`
        Uses the embedded `doc.jq` standard library.

### Options
    * **--grep-format**
        Output in `filename:line:content` format for piping to grep.
    
    * **--recursive** (-R)
        Process directories recursively.

    * **--raw-output** (-r)
        Output raw strings, not JSON texts.

    * **--format** [json|grep|human]
        Choose explicitly between JSON, Grep, or Human-readable tables.

## EXAMPLES
    Query public functions in a Rust file:
    `vecq src/main.rs '.functions[] | select(.visibility == "pub")'`

    Pipeline integration:
    `find . -name "*.rs" | xargs vecq --grep-format | grep "pub fn"`

    Generate architecture diagram:
    `vecq map ./src -o mermaid > architecture.md`

## SUPPORTED FILE TYPES
    Rust (.rs), Python (.py), Markdown (.md), C (.c, .h), 
    C++ (.cpp, .hpp), CUDA (.cu), Go (.go), Bash (.sh)
