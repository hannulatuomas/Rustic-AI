use std::path::Path;

use tree_sitter::{Language, Node, Parser, Tree};

use super::types::{CallEdge, SymbolIndex, SymbolType};

pub fn extract_symbols_and_calls(
    language_name: &str,
    file_path: &str,
    source: &str,
) -> Option<(Vec<SymbolIndex>, Vec<CallEdge>)> {
    let language = language_for(language_name, file_path)?;
    let tree = parse(language, source)?;

    let root = tree.root_node();
    let mut symbols = Vec::new();
    let mut calls = Vec::new();
    visit(
        root,
        source.as_bytes(),
        file_path,
        None,
        &mut symbols,
        &mut calls,
    );
    Some((symbols, calls))
}

fn parse(language: Language, source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    parser.parse(source, None)
}

fn language_for(language_name: &str, file_path: &str) -> Option<Language> {
    match language_name {
        "rust" => Some(tree_sitter_rust::language()),
        "python" => Some(tree_sitter_python::language()),
        "javascript" => Some(tree_sitter_javascript::language()),
        "typescript" => {
            if Path::new(file_path)
                .extension()
                .map(|ext| ext.eq_ignore_ascii_case("tsx"))
                .unwrap_or(false)
            {
                Some(tree_sitter_typescript::language_tsx())
            } else {
                Some(tree_sitter_typescript::language_typescript())
            }
        }
        "go" => Some(tree_sitter_go::language()),
        "c" => Some(tree_sitter_c::language()),
        "cpp" => Some(tree_sitter_cpp::language()),
        _ => None,
    }
}

fn visit(
    node: Node<'_>,
    source: &[u8],
    file_path: &str,
    current_scope: Option<String>,
    symbols: &mut Vec<SymbolIndex>,
    calls: &mut Vec<CallEdge>,
) {
    let mut scope = current_scope;

    if let Some(symbol_type) = map_symbol_kind(node.kind()) {
        if let Some(name_node) = find_name_node(node) {
            if let Ok(name) = name_node.utf8_text(source) {
                let clean_name = name.trim().to_owned();
                if !clean_name.is_empty() {
                    let start = name_node.start_position();
                    symbols.push(SymbolIndex {
                        name: clean_name.clone(),
                        symbol_type,
                        file_path: file_path.to_owned(),
                        line: start.row + 1,
                        column: start.column + 1,
                        docstring: None,
                        signature: node
                            .utf8_text(source)
                            .ok()
                            .map(str::trim)
                            .map(str::to_owned),
                    });

                    if matches!(
                        symbol_type,
                        SymbolType::Function | SymbolType::Method | SymbolType::Impl
                    ) {
                        scope = Some(clean_name);
                    }
                }
            }
        }
    }

    if is_call_node(node.kind()) {
        if let Some(callee) = extract_call_name(node, source) {
            if let Some(caller) = scope.clone() {
                let start = node.start_position();
                calls.push(CallEdge {
                    caller_symbol: caller,
                    callee_symbol: callee,
                    file_path: file_path.to_owned(),
                    line: start.row + 1,
                    column: start.column + 1,
                });
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(child, source, file_path, scope.clone(), symbols, calls);
    }
}

fn map_symbol_kind(kind: &str) -> Option<SymbolType> {
    match kind {
        "function_item" | "function_definition" | "function_declaration" => {
            Some(SymbolType::Function)
        }
        "method_definition" | "method_declaration" => Some(SymbolType::Method),
        "struct_item" | "struct_specifier" | "class_definition" | "class_declaration" => {
            Some(SymbolType::Struct)
        }
        "enum_item" | "enum_specifier" => Some(SymbolType::Enum),
        "trait_item" => Some(SymbolType::Trait),
        "impl_item" => Some(SymbolType::Impl),
        "type_item" | "type_alias_declaration" | "type_definition" | "type_declaration" => {
            Some(SymbolType::Type)
        }
        "const_item" | "const_declaration" => Some(SymbolType::Constant),
        "mod_item" | "module" => Some(SymbolType::Module),
        "lexical_declaration" | "variable_declaration" | "var_declaration" => {
            Some(SymbolType::Variable)
        }
        _ => None,
    }
}

fn is_call_node(kind: &str) -> bool {
    matches!(kind, "call_expression" | "call" | "invocation_expression")
}

fn find_name_node(node: Node<'_>) -> Option<Node<'_>> {
    for field in [
        "name",
        "declarator",
        "type",
        "value",
        "identifier",
        "function",
    ] {
        if let Some(named) = node.child_by_field_name(field) {
            if named.is_named() {
                return first_identifier(named).or(Some(named));
            }
        }
    }
    first_identifier(node)
}

fn first_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "identifier" || node.kind() == "type_identifier" {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = first_identifier(child) {
            return Some(found);
        }
    }
    None
}

fn extract_call_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(function_node) = node.child_by_field_name("function") {
        return first_identifier(function_node)
            .or(Some(function_node))
            .and_then(|value| {
                value
                    .utf8_text(source)
                    .ok()
                    .map(str::trim)
                    .map(str::to_owned)
            })
            .filter(|value| !value.is_empty());
    }

    first_identifier(node)
        .and_then(|value| {
            value
                .utf8_text(source)
                .ok()
                .map(str::trim)
                .map(str::to_owned)
        })
        .filter(|value| !value.is_empty())
}
