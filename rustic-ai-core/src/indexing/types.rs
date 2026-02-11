use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolType {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Type,
    Variable,
    Constant,
    Module,
}

impl SymbolType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Type => "type",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::Module => "module",
        }
    }

    pub fn from_storage_value(value: &str) -> Self {
        match value {
            "function" => Self::Function,
            "method" => Self::Method,
            "struct" => Self::Struct,
            "enum" => Self::Enum,
            "trait" => Self::Trait,
            "impl" => Self::Impl,
            "type" => Self::Type,
            "variable" => Self::Variable,
            "constant" => Self::Constant,
            "module" => Self::Module,
            _ => Self::Variable,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolIndex {
    pub name: String,
    pub symbol_type: SymbolType,
    pub file_path: String,
    pub line: usize,
    pub column: usize,
    pub docstring: Option<String>,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileIndex {
    pub path: String,
    pub language: String,
    pub functions: Vec<String>,
    pub classes: Vec<String>,
    pub imports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeIndex {
    pub workspace: String,
    pub files: Vec<FileIndex>,
    pub symbols: Vec<SymbolIndex>,
    pub dependencies: Vec<(String, String)>,
    pub call_edges: Vec<CallEdge>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    pub caller_symbol: String,
    pub callee_symbol: String,
    pub file_path: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedFileRecord {
    pub path: String,
    pub language: String,
    pub functions: Vec<String>,
    pub classes: Vec<String>,
    pub imports: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedSymbolRecord {
    pub name: String,
    pub symbol_type: SymbolType,
    pub file_path: String,
    pub line: usize,
    pub column: usize,
    pub docstring: Option<String>,
    pub signature: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedCallEdgeRecord {
    pub caller_symbol: String,
    pub callee_symbol: String,
    pub file_path: String,
    pub line: usize,
    pub column: usize,
    pub updated_at: DateTime<Utc>,
}
