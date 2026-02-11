use regex::Regex;

use crate::indexing::types::{SymbolIndex, SymbolType};

pub fn extract_symbols(language: &str, file_path: &str, source: &str) -> Vec<SymbolIndex> {
    match language {
        "rust" => extract_rust_symbols(file_path, source),
        "python" => extract_python_symbols(file_path, source),
        "javascript" | "typescript" => extract_js_ts_symbols(file_path, source),
        "go" => extract_go_symbols(file_path, source),
        "cpp" | "c" => extract_c_family_symbols(file_path, source),
        _ => Vec::new(),
    }
}

fn extract_rust_symbols(file_path: &str, source: &str) -> Vec<SymbolIndex> {
    let mut symbols = Vec::new();
    let fn_re = Regex::new(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let struct_re = Regex::new(r"^\s*(?:pub\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let enum_re = Regex::new(r"^\s*(?:pub\s+)?enum\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let trait_re = Regex::new(r"^\s*(?:pub\s+)?trait\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let mod_re = Regex::new(r"^\s*(?:pub\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();

    for (line_index, line) in source.lines().enumerate() {
        push_if_match(
            &mut symbols,
            &fn_re,
            SymbolType::Function,
            file_path,
            line,
            line_index,
        );
        push_if_match(
            &mut symbols,
            &struct_re,
            SymbolType::Struct,
            file_path,
            line,
            line_index,
        );
        push_if_match(
            &mut symbols,
            &enum_re,
            SymbolType::Enum,
            file_path,
            line,
            line_index,
        );
        push_if_match(
            &mut symbols,
            &trait_re,
            SymbolType::Trait,
            file_path,
            line,
            line_index,
        );
        push_if_match(
            &mut symbols,
            &mod_re,
            SymbolType::Module,
            file_path,
            line,
            line_index,
        );
    }

    symbols
}

fn extract_python_symbols(file_path: &str, source: &str) -> Vec<SymbolIndex> {
    let mut symbols = Vec::new();
    let class_re = Regex::new(r"^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let fn_re = Regex::new(r"^\s*def\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();

    for (line_index, line) in source.lines().enumerate() {
        push_if_match(
            &mut symbols,
            &class_re,
            SymbolType::Struct,
            file_path,
            line,
            line_index,
        );
        push_if_match(
            &mut symbols,
            &fn_re,
            SymbolType::Function,
            file_path,
            line,
            line_index,
        );
    }

    symbols
}

fn extract_js_ts_symbols(file_path: &str, source: &str) -> Vec<SymbolIndex> {
    let mut symbols = Vec::new();
    let fn_re = Regex::new(r"^\s*(?:export\s+)?function\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let class_re = Regex::new(r"^\s*(?:export\s+)?class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let const_re = Regex::new(r"^\s*(?:export\s+)?const\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();

    for (line_index, line) in source.lines().enumerate() {
        push_if_match(
            &mut symbols,
            &fn_re,
            SymbolType::Function,
            file_path,
            line,
            line_index,
        );
        push_if_match(
            &mut symbols,
            &class_re,
            SymbolType::Struct,
            file_path,
            line,
            line_index,
        );
        push_if_match(
            &mut symbols,
            &const_re,
            SymbolType::Variable,
            file_path,
            line,
            line_index,
        );
    }

    symbols
}

fn extract_go_symbols(file_path: &str, source: &str) -> Vec<SymbolIndex> {
    let mut symbols = Vec::new();
    let fn_re = Regex::new(r"^\s*func\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let type_re = Regex::new(r"^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();

    for (line_index, line) in source.lines().enumerate() {
        push_if_match(
            &mut symbols,
            &fn_re,
            SymbolType::Function,
            file_path,
            line,
            line_index,
        );
        push_if_match(
            &mut symbols,
            &type_re,
            SymbolType::Type,
            file_path,
            line,
            line_index,
        );
    }

    symbols
}

fn extract_c_family_symbols(file_path: &str, source: &str) -> Vec<SymbolIndex> {
    let mut symbols = Vec::new();
    let func_re =
        Regex::new(r"^\s*[A-Za-z_][A-Za-z0-9_\s\*]+\s+([A-Za-z_][A-Za-z0-9_]*)\s*\([^;]*\)\s*\{")
            .unwrap();
    let struct_re = Regex::new(r"^\s*(?:typedef\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let enum_re = Regex::new(r"^\s*(?:typedef\s+)?enum\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();

    for (line_index, line) in source.lines().enumerate() {
        push_if_match(
            &mut symbols,
            &func_re,
            SymbolType::Function,
            file_path,
            line,
            line_index,
        );
        push_if_match(
            &mut symbols,
            &struct_re,
            SymbolType::Struct,
            file_path,
            line,
            line_index,
        );
        push_if_match(
            &mut symbols,
            &enum_re,
            SymbolType::Enum,
            file_path,
            line,
            line_index,
        );
    }

    symbols
}

fn push_if_match(
    target: &mut Vec<SymbolIndex>,
    regex: &Regex,
    symbol_type: SymbolType,
    file_path: &str,
    line: &str,
    line_index: usize,
) {
    if let Some(captures) = regex.captures(line) {
        if let Some(name_match) = captures.get(1) {
            target.push(SymbolIndex {
                name: name_match.as_str().to_owned(),
                symbol_type,
                file_path: file_path.to_owned(),
                line: line_index + 1,
                column: name_match.start() + 1,
                docstring: None,
                signature: Some(line.trim().to_owned()),
            });
        }
    }
}
