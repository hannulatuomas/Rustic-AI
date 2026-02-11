use std::path::Path;

use regex::Regex;

use super::ast::extract_symbols_and_calls;
use super::symbols::extract_symbols;
use super::types::{CallEdge, FileIndex, SymbolIndex};

pub fn detect_language(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match extension.as_str() {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" | "jsx" => Some("javascript"),
        "ts" | "tsx" => Some("typescript"),
        "go" => Some("go"),
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
        _ => None,
    }
}

pub fn build_file_and_symbols(
    path: &str,
    language: &str,
    source: &str,
) -> (FileIndex, Vec<SymbolIndex>, Vec<CallEdge>) {
    let imports = extract_imports(language, source);
    let (symbols, call_edges) =
        if let Some((symbols, call_edges)) = extract_symbols_and_calls(language, path, source) {
            (symbols, call_edges)
        } else {
            (extract_symbols(language, path, source), Vec::new())
        };

    let mut functions = Vec::new();
    let mut classes = Vec::new();
    for symbol in &symbols {
        match symbol.symbol_type {
            super::types::SymbolType::Function | super::types::SymbolType::Method => {
                if !functions.iter().any(|name| name == &symbol.name) {
                    functions.push(symbol.name.clone());
                }
            }
            super::types::SymbolType::Struct
            | super::types::SymbolType::Enum
            | super::types::SymbolType::Trait
            | super::types::SymbolType::Type => {
                if !classes.iter().any(|name| name == &symbol.name) {
                    classes.push(symbol.name.clone());
                }
            }
            _ => {}
        }
    }

    let file_index = FileIndex {
        path: path.to_owned(),
        language: language.to_owned(),
        functions,
        classes,
        imports,
    };

    (file_index, symbols, call_edges)
}

fn extract_imports(language: &str, source: &str) -> Vec<String> {
    let mut imports = Vec::new();

    let regexes = match language {
        "rust" => vec![
            Regex::new(r"^\s*use\s+([^;]+);").unwrap(),
            Regex::new(r"^\s*mod\s+([A-Za-z_][A-Za-z0-9_]*);?").unwrap(),
        ],
        "python" => vec![
            Regex::new(r"^\s*import\s+(.+)$").unwrap(),
            Regex::new(r"^\s*from\s+([A-Za-z0-9_\.]+)\s+import\s+(.+)$").unwrap(),
        ],
        "javascript" | "typescript" => vec![
            Regex::new(r#"^\s*import\s+.+\s+from\s+['\"]([^'\"]+)['\"]"#).unwrap(),
            Regex::new(r#"^\s*const\s+.+\s*=\s*require\(['\"]([^'\"]+)['\"]\)"#).unwrap(),
        ],
        "go" => vec![
            Regex::new(r#"^\s*import\s+\"([^\"]+)\""#).unwrap(),
            Regex::new(r#"^\s*\"([^\"]+)\""#).unwrap(),
        ],
        "c" | "cpp" => vec![Regex::new(r#"^\s*#include\s+[<"]([^>"]+)[>"]"#).unwrap()],
        _ => Vec::new(),
    };

    for line in source.lines() {
        for regex in &regexes {
            if let Some(captures) = regex.captures(line) {
                if let Some(import_match) = captures.get(1) {
                    let import_name = import_match.as_str().trim().to_owned();
                    if !imports.iter().any(|value| value == &import_name) {
                        imports.push(import_name);
                    }
                }
            }
        }
    }

    imports
}
