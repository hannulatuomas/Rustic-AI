pub mod ast;
pub mod graph;
pub mod parser;
pub mod symbols;
pub mod types;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use ignore::WalkBuilder;

use crate::error::{Error, Result};
use crate::storage::StorageBackend;
use crate::vector::{create_embedding_provider, Embedding, VectorDb};

pub use graph::{analyze_impact, build_code_graph, render_dot, CodeGraph, ImpactReport};
use parser::{build_file_and_symbols, detect_language};
pub use types::{
    CallEdge, CodeIndex, FileIndex, IndexedCallEdgeRecord, IndexedFileRecord, IndexedSymbolRecord,
    SymbolIndex, SymbolType,
};

pub struct CodeIndexer {
    storage: Arc<dyn StorageBackend>,
    workspace: PathBuf,
    indexing_enabled: bool,
    vector_enabled: bool,
    retrieval_config: crate::config::schema::RetrievalConfig,
}

impl CodeIndexer {
    pub fn new(
        storage: Arc<dyn StorageBackend>,
        workspace: PathBuf,
        indexing_enabled: bool,
        vector_enabled: bool,
        retrieval_config: crate::config::schema::RetrievalConfig,
    ) -> Self {
        Self {
            storage,
            workspace,
            indexing_enabled,
            vector_enabled,
            retrieval_config,
        }
    }

    pub async fn build_index(&self) -> Result<CodeIndex> {
        if !self.indexing_enabled {
            return Ok(CodeIndex {
                workspace: self.workspace.to_string_lossy().to_string(),
                files: Vec::new(),
                symbols: Vec::new(),
                dependencies: Vec::new(),
                call_edges: Vec::new(),
                updated_at: Utc::now(),
            });
        }

        let workspace_string = self.workspace.to_string_lossy().to_string();
        let mut files = Vec::new();
        let mut symbols = Vec::new();
        let mut dependencies = Vec::new();
        let mut call_edges = Vec::new();
        let vector_db = if self.vector_enabled {
            Some(VectorDb::new(
                self.storage.clone(),
                workspace_string.clone(),
            ))
        } else {
            None
        };
        let embedder = if self.vector_enabled {
            Some(create_embedding_provider(&self.retrieval_config)?)
        } else {
            None
        };

        let mut walker = WalkBuilder::new(&self.workspace);
        walker.hidden(false);
        walker.git_ignore(true);
        walker.git_exclude(true);
        walker.parents(true);

        for entry in walker.build() {
            let entry = entry.map_err(|err| Error::Io(std::io::Error::other(err.to_string())))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let Some(language) = detect_language(path) else {
                continue;
            };

            let content = match tokio::fs::read_to_string(path).await {
                Ok(content) => content,
                Err(_) => continue,
            };

            let relative_path = relative_path(&self.workspace, path);
            let (file_index, file_symbols, file_call_edges) =
                build_file_and_symbols(&relative_path, language, &content);

            for import in &file_index.imports {
                dependencies.push((relative_path.clone(), import.clone()));
            }

            self.storage
                .upsert_code_file_index(
                    &workspace_string,
                    &relative_path,
                    language,
                    &file_index.functions,
                    &file_index.classes,
                    &file_index.imports,
                )
                .await?;
            self.storage
                .replace_code_symbols_for_file(&workspace_string, &relative_path, &file_symbols)
                .await?;
            self.storage
                .replace_code_call_edges_for_file(
                    &workspace_string,
                    &relative_path,
                    &file_call_edges,
                )
                .await?;

            if let (Some(vector_db), Some(embedder)) = (vector_db.as_ref(), embedder.as_ref()) {
                let embedding_text =
                    build_embedding_text(&file_index, &file_symbols, &file_call_edges);
                let vector = embedder.embed(&embedding_text).await?;
                let embedding = Embedding {
                    id: format!("file:{relative_path}"),
                    vector,
                    metadata: serde_json::json!({
                        "path": relative_path,
                        "language": file_index.language,
                        "text": embedding_text,
                    }),
                };
                vector_db.upsert_embedding(&embedding).await?;
            }

            files.push(file_index);
            symbols.extend(file_symbols);
            call_edges.extend(file_call_edges);
        }

        self.storage
            .upsert_code_index_metadata(&workspace_string, Utc::now())
            .await?;

        Ok(CodeIndex {
            workspace: workspace_string,
            files,
            symbols,
            dependencies,
            call_edges,
            updated_at: Utc::now(),
        })
    }

    pub async fn rebuild_file(&self, file_path: &Path) -> Result<()> {
        if !self.indexing_enabled {
            return Ok(());
        }

        let Some(language) = detect_language(file_path) else {
            return Ok(());
        };
        let content = tokio::fs::read_to_string(file_path)
            .await
            .map_err(|err| Error::Io(std::io::Error::other(err.to_string())))?;
        let workspace_string = self.workspace.to_string_lossy().to_string();
        let relative_path = relative_path(&self.workspace, file_path);
        let (file_index, symbols, call_edges) =
            build_file_and_symbols(&relative_path, language, &content);

        self.storage
            .upsert_code_file_index(
                &workspace_string,
                &relative_path,
                language,
                &file_index.functions,
                &file_index.classes,
                &file_index.imports,
            )
            .await?;
        self.storage
            .replace_code_symbols_for_file(&workspace_string, &relative_path, &symbols)
            .await?;
        self.storage
            .replace_code_call_edges_for_file(&workspace_string, &relative_path, &call_edges)
            .await?;

        if self.vector_enabled {
            let embedder = create_embedding_provider(&self.retrieval_config)?;
            let vector_db = VectorDb::new(self.storage.clone(), workspace_string.clone());
            let embedding_text = build_embedding_text(&file_index, &symbols, &call_edges);
            let vector = embedder.embed(&embedding_text).await?;
            let embedding = Embedding {
                id: format!("file:{relative_path}"),
                vector,
                metadata: serde_json::json!({
                    "path": relative_path,
                    "language": file_index.language,
                    "text": embedding_text,
                }),
            };
            vector_db.upsert_embedding(&embedding).await?;
        }

        self.storage
            .upsert_code_index_metadata(&workspace_string, Utc::now())
            .await?;
        Ok(())
    }

    pub async fn search_symbols(&self, query: &str, limit: usize) -> Result<Vec<SymbolIndex>> {
        if !self.indexing_enabled {
            return Ok(Vec::new());
        }
        let workspace_string = self.workspace.to_string_lossy().to_string();
        self.storage
            .search_code_symbols(&workspace_string, query, limit)
            .await
    }

    pub async fn load_index_snapshot(&self) -> Result<CodeIndex> {
        if !self.indexing_enabled {
            return Ok(CodeIndex {
                workspace: self.workspace.to_string_lossy().to_string(),
                files: Vec::new(),
                symbols: Vec::new(),
                dependencies: Vec::new(),
                call_edges: Vec::new(),
                updated_at: Utc::now(),
            });
        }

        let workspace_string = self.workspace.to_string_lossy().to_string();
        let file_records = self
            .storage
            .list_code_file_indexes(&workspace_string)
            .await?;
        let symbol_records = self.storage.list_code_symbols(&workspace_string).await?;
        let call_edge_records = self.storage.list_code_call_edges(&workspace_string).await?;
        let dependencies = file_records
            .iter()
            .flat_map(|record| {
                record
                    .imports
                    .iter()
                    .map(|import| (record.path.clone(), import.clone()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let files = file_records
            .iter()
            .map(|record| FileIndex {
                path: record.path.clone(),
                language: record.language.clone(),
                functions: record.functions.clone(),
                classes: record.classes.clone(),
                imports: record.imports.clone(),
            })
            .collect::<Vec<_>>();
        let symbols = symbol_records
            .iter()
            .map(|record| SymbolIndex {
                name: record.name.clone(),
                symbol_type: record.symbol_type,
                file_path: record.file_path.clone(),
                line: record.line,
                column: record.column,
                docstring: record.docstring.clone(),
                signature: record.signature.clone(),
            })
            .collect::<Vec<_>>();
        let call_edges = call_edge_records
            .iter()
            .map(|record| CallEdge {
                caller_symbol: record.caller_symbol.clone(),
                callee_symbol: record.callee_symbol.clone(),
                file_path: record.file_path.clone(),
                line: record.line,
                column: record.column,
            })
            .collect::<Vec<_>>();

        let updated_at = self
            .storage
            .get_code_index_metadata(&workspace_string)
            .await?
            .unwrap_or_else(Utc::now);

        Ok(CodeIndex {
            workspace: workspace_string,
            files,
            symbols,
            dependencies,
            call_edges,
            updated_at,
        })
    }
}

fn build_embedding_text(file: &FileIndex, symbols: &[SymbolIndex], calls: &[CallEdge]) -> String {
    let symbol_names = symbols
        .iter()
        .take(48)
        .map(|symbol| symbol.name.clone())
        .collect::<Vec<_>>()
        .join(" ");
    let call_names = calls
        .iter()
        .take(48)
        .map(|edge| format!("{}->{}", edge.caller_symbol, edge.callee_symbol))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        "path={} language={} functions={} classes={} imports={} symbols={} calls={}",
        file.path,
        file.language,
        file.functions.join(" "),
        file.classes.join(" "),
        file.imports.join(" "),
        symbol_names,
        call_names
    )
}

fn relative_path(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}
