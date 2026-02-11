use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256, Sha512};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use tokio::sync::mpsc;

use crate::config::schema::{ToolConfig, WorkingDirMode};
use crate::error::{Error, Result};
use crate::events::Event;
use crate::rules::discovery::simple_glob_match;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct FilesystemTool {
    config: ToolConfig,
    schema: Value,
}

const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;
const MAX_HASH_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 50 * 1024 * 1024;
const MAX_EDIT_BYTES: u64 = 10 * 1024 * 1024;
const MAX_GLOB_DEPTH: usize = 20;
const MAX_GLOB_ENTRIES: usize = 50_000;
const MAX_COPY_DEPTH: usize = 32;
const MAX_COPY_ENTRIES: usize = 50_000;
const MAX_DELETE_DEPTH: usize = 32;
const MAX_DELETE_ENTRIES: usize = 50_000;
const MAX_MKDIR_DEPTH: usize = 32;

impl FilesystemTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "read", "write", "edit", "list", "mkdir", "delete", "copy", "move", "info", "glob", "hash"
                    ]
                },
                "path": { "type": "string" },
                "content": { "type": "string" },
                "source": { "type": "string" },
                "destination": { "type": "string" },
                "pattern": { "type": "string" }
            },
            "required": ["operation"]
        });

        Self { config, schema }
    }

    fn resolve_working_dir(&self, context: &ToolExecutionContext, args: &Value) -> Result<PathBuf> {
        let per_call_override = args
            .get("working_directory")
            .and_then(Value::as_str)
            .map(PathBuf::from);

        let base = &context.working_directory;
        let resolved = if let Some(override_path) = per_call_override {
            if override_path.is_absolute() {
                override_path
            } else {
                base.join(override_path)
            }
        } else {
            match self.config.working_dir {
                WorkingDirMode::Current | WorkingDirMode::ProjectRoot => base.clone(),
                WorkingDirMode::CustomPath => {
                    let custom = self.config.custom_working_dir.as_deref().ok_or_else(|| {
                        Error::Config(
                            "custom_working_dir is required when working_dir is 'custom_path'"
                                .to_owned(),
                        )
                    })?;
                    let path = Path::new(custom);
                    if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        base.join(path)
                    }
                }
            }
        };

        if !resolved.exists() {
            return Err(Error::Tool(format!(
                "working directory '{}' does not exist",
                resolved.display()
            )));
        }

        let metadata = fs::metadata(&resolved).map_err(|err| {
            Error::Tool(format!(
                "failed to read metadata for working directory '{}': {err}",
                resolved.display()
            ))
        })?;
        if !metadata.is_dir() {
            return Err(Error::Tool(format!(
                "working directory '{}' is not a directory",
                resolved.display()
            )));
        }

        fs::canonicalize(&resolved).map_err(|err| {
            Error::Tool(format!(
                "failed to canonicalize working directory '{}': {err}",
                resolved.display()
            ))
        })
    }

    fn normalize_path(path: &Path) -> PathBuf {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    let _ = normalized.pop();
                }
                _ => normalized.push(component.as_os_str()),
            }
        }
        normalized
    }

    fn resolve_candidate_path(raw_path: &str, working_dir: &Path) -> PathBuf {
        let raw = Path::new(raw_path);
        if raw.is_absolute() {
            Self::normalize_path(raw)
        } else {
            Self::normalize_path(&working_dir.join(raw))
        }
    }

    fn checked_path(&self, raw_path: &str, working_dir: &Path) -> Result<PathBuf> {
        if raw_path.trim().is_empty() {
            return Err(Error::Tool("path must be non-empty".to_owned()));
        }

        let candidate = Self::resolve_candidate_path(raw_path, working_dir);
        let effective = if candidate.exists() {
            fs::canonicalize(&candidate).map_err(|err| {
                Error::Tool(format!(
                    "failed to canonicalize path '{}': {err}",
                    candidate.display()
                ))
            })?
        } else if let Some(parent) = candidate.parent() {
            if parent.exists() {
                let canonical_parent = fs::canonicalize(parent).map_err(|err| {
                    Error::Tool(format!(
                        "failed to canonicalize parent '{}' for '{}': {err}",
                        parent.display(),
                        candidate.display()
                    ))
                })?;

                match candidate.file_name() {
                    Some(name) => canonical_parent.join(name),
                    None => canonical_parent,
                }
            } else {
                candidate.clone()
            }
        } else {
            candidate.clone()
        };

        Ok(effective)
    }

    fn required_string<'a>(&self, args: &'a Value, key: &str) -> Result<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| Error::Tool(format!("missing '{key}' argument")))
    }

    fn optional_bool(args: &Value, key: &str, default: bool) -> bool {
        args.get(key).and_then(Value::as_bool).unwrap_or(default)
    }

    fn optional_usize(args: &Value, key: &str, default: usize) -> Result<usize> {
        match args.get(key) {
            Some(value) => {
                let raw = value.as_u64().ok_or_else(|| {
                    Error::Tool(format!("argument '{key}' must be a non-negative integer"))
                })?;
                usize::try_from(raw)
                    .map_err(|_| Error::Tool(format!("argument '{key}' is too large")))
            }
            None => Ok(default),
        }
    }

    fn operation_result(payload: Value) -> ToolResult {
        ToolResult {
            success: true,
            exit_code: Some(0),
            output: payload.to_string(),
        }
    }

    fn enforce_agent_permission(
        &self,
        operation: &str,
        context: &ToolExecutionContext,
    ) -> Result<()> {
        if context.agent_permission_mode == crate::config::schema::AgentPermissionMode::ReadWrite {
            return Ok(());
        }

        let allowed = matches!(operation, "read" | "list" | "info" | "glob" | "hash");
        if allowed {
            return Ok(());
        }

        Err(Error::Tool(format!(
            "filesystem operation '{}' is not allowed in read_only agent mode",
            operation
        )))
    }

    fn hash_file(&self, path: &Path, algorithm: &str) -> Result<String> {
        let metadata = fs::metadata(path).map_err(|err| {
            Error::Tool(format!(
                "failed to read metadata '{}': {err}",
                path.display()
            ))
        })?;
        if metadata.len() > MAX_HASH_BYTES {
            return Err(Error::Tool(format!(
                "file '{}' is too large to hash safely ({} bytes, limit {} bytes)",
                path.display(),
                metadata.len(),
                MAX_HASH_BYTES
            )));
        }

        let mut file = fs::File::open(path)
            .map_err(|err| Error::Tool(format!("failed to open '{}': {err}", path.display())))?;
        let mut buffer = [0u8; 8192];

        let digest = match algorithm.to_ascii_lowercase().as_str() {
            "sha256" => {
                let mut hasher = Sha256::new();
                loop {
                    let read = file.read(&mut buffer).map_err(|err| {
                        Error::Tool(format!(
                            "failed to read '{}' for hashing: {err}",
                            path.display()
                        ))
                    })?;
                    if read == 0 {
                        break;
                    }
                    hasher.update(&buffer[..read]);
                }
                format!("{:x}", hasher.finalize())
            }
            "sha512" => {
                let mut hasher = Sha512::new();
                loop {
                    let read = file.read(&mut buffer).map_err(|err| {
                        Error::Tool(format!(
                            "failed to read '{}' for hashing: {err}",
                            path.display()
                        ))
                    })?;
                    if read == 0 {
                        break;
                    }
                    hasher.update(&buffer[..read]);
                }
                format!("{:x}", hasher.finalize())
            }
            other => {
                return Err(Error::Tool(format!(
                    "unsupported hash algorithm '{other}', supported: sha256, sha512"
                )));
            }
        };

        Ok(digest)
    }

    fn copy_directory_recursive(
        source: &Path,
        destination: &Path,
        depth: usize,
        copied_entries: &mut usize,
    ) -> Result<()> {
        if depth > MAX_COPY_DEPTH {
            return Err(Error::Tool(format!(
                "directory copy exceeded max depth ({MAX_COPY_DEPTH})"
            )));
        }

        if !destination.exists() {
            fs::create_dir_all(destination).map_err(|err| {
                Error::Tool(format!(
                    "failed to create destination directory '{}': {err}",
                    destination.display()
                ))
            })?;
        }

        for entry in fs::read_dir(source)
            .map_err(|err| Error::Tool(format!("failed to read '{}': {err}", source.display())))?
        {
            let entry = entry.map_err(|err| {
                Error::Tool(format!(
                    "failed to read directory entry in '{}': {err}",
                    source.display()
                ))
            })?;

            *copied_entries += 1;
            if *copied_entries > MAX_COPY_ENTRIES {
                return Err(Error::Tool(format!(
                    "directory copy exceeded max entries ({MAX_COPY_ENTRIES})"
                )));
            }

            let from = entry.path();
            let to = destination.join(entry.file_name());
            let symlink_metadata = fs::symlink_metadata(&from).map_err(|err| {
                Error::Tool(format!(
                    "failed to read symlink metadata for '{}': {err}",
                    from.display()
                ))
            })?;
            if symlink_metadata.file_type().is_symlink() {
                return Err(Error::Tool(format!(
                    "symlink entries are not supported for recursive copy: '{}'",
                    from.display()
                )));
            }
            let metadata = entry.metadata().map_err(|err| {
                Error::Tool(format!(
                    "failed to read metadata for '{}': {err}",
                    from.display()
                ))
            })?;

            if metadata.is_dir() {
                Self::copy_directory_recursive(&from, &to, depth + 1, copied_entries)?;
            } else {
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent).map_err(|err| {
                        Error::Tool(format!(
                            "failed to create parent directory '{}': {err}",
                            parent.display()
                        ))
                    })?;
                }
                fs::copy(&from, &to).map_err(|err| {
                    Error::Tool(format!(
                        "failed to copy '{}' -> '{}': {err}",
                        from.display(),
                        to.display()
                    ))
                })?;
            }
        }

        Ok(())
    }

    fn enforce_recursive_delete_limits(path: &Path) -> Result<()> {
        let mut queue = VecDeque::new();
        queue.push_back((path.to_path_buf(), 0usize));
        let mut entries = 0usize;

        while let Some((current, depth)) = queue.pop_front() {
            if depth > MAX_DELETE_DEPTH {
                return Err(Error::Tool(format!(
                    "recursive delete exceeded max depth ({MAX_DELETE_DEPTH})"
                )));
            }

            let metadata = fs::symlink_metadata(&current).map_err(|err| {
                Error::Tool(format!(
                    "failed to inspect '{}' during delete safety scan: {err}",
                    current.display()
                ))
            })?;
            let is_symlink = metadata.file_type().is_symlink();

            entries += 1;
            if entries > MAX_DELETE_ENTRIES {
                return Err(Error::Tool(format!(
                    "recursive delete exceeded max entries ({MAX_DELETE_ENTRIES})"
                )));
            }

            if metadata.is_dir() && !is_symlink {
                let iter = fs::read_dir(&current).map_err(|err| {
                    Error::Tool(format!(
                        "failed to read '{}' during delete safety scan: {err}",
                        current.display()
                    ))
                })?;
                for entry in iter {
                    let entry = entry.map_err(|err| {
                        Error::Tool(format!(
                            "failed to read entry in '{}' during delete safety scan: {err}",
                            current.display()
                        ))
                    })?;
                    queue.push_back((entry.path(), depth + 1));
                }
            }
        }

        Ok(())
    }

    fn execute_operation(&self, args: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let operation = self
            .required_string(&args, "operation")?
            .to_ascii_lowercase();
        self.enforce_agent_permission(&operation, context)?;
        let working_dir = self.resolve_working_dir(context, &args)?;

        match operation.as_str() {
            "read" => {
                let path = self.checked_path(self.required_string(&args, "path")?, &working_dir)?;
                let metadata = fs::metadata(&path).map_err(|err| {
                    Error::Tool(format!(
                        "failed to read metadata for '{}': {err}",
                        path.display()
                    ))
                })?;
                if metadata.len() > MAX_READ_BYTES {
                    return Err(Error::Tool(format!(
                        "file '{}' is too large to read safely ({} bytes, limit {} bytes)",
                        path.display(),
                        metadata.len(),
                        MAX_READ_BYTES
                    )));
                }

                let file = fs::File::open(&path).map_err(|err| {
                    Error::Tool(format!("failed to open '{}': {err}", path.display()))
                })?;
                let reader = BufReader::new(file);
                let offset = Self::optional_usize(&args, "offset", 0)?;
                let limit = Self::optional_usize(&args, "limit", 2000)?;
                let mut content_lines = Vec::new();
                for (index, line) in reader.lines().enumerate() {
                    if index < offset {
                        continue;
                    }
                    if content_lines.len() >= limit {
                        break;
                    }
                    let line = line.map_err(|err| {
                        Error::Tool(format!(
                            "failed to read line from '{}': {err}",
                            path.display()
                        ))
                    })?;
                    content_lines.push(line);
                }
                let slice = content_lines.join("\n");

                Ok(Self::operation_result(json!({
                    "path": path.to_string_lossy(),
                    "offset": offset,
                    "limit": limit,
                    "content": slice
                })))
            }
            "write" => {
                let path = self.checked_path(self.required_string(&args, "path")?, &working_dir)?;
                let content = self.required_string(&args, "content")?;
                if content.len() > MAX_WRITE_BYTES {
                    return Err(Error::Tool(format!(
                        "write payload is too large ({} bytes, limit {} bytes)",
                        content.len(),
                        MAX_WRITE_BYTES
                    )));
                }
                let overwrite = Self::optional_bool(&args, "overwrite", true);
                let create_dirs = Self::optional_bool(&args, "create_dirs", false);

                if path.exists() && !overwrite {
                    return Err(Error::Tool(format!(
                        "file '{}' already exists and overwrite=false",
                        path.display()
                    )));
                }

                if let Some(parent) = path.parent() {
                    if !parent.exists() {
                        if create_dirs {
                            fs::create_dir_all(parent).map_err(|err| {
                                Error::Tool(format!(
                                    "failed to create parent directories for '{}': {err}",
                                    path.display()
                                ))
                            })?;
                        } else {
                            return Err(Error::Tool(format!(
                                "parent directory '{}' does not exist",
                                parent.display()
                            )));
                        }
                    }
                }

                fs::write(&path, content).map_err(|err| {
                    Error::Tool(format!("failed to write '{}': {err}", path.display()))
                })?;

                Ok(Self::operation_result(json!({
                    "path": path.to_string_lossy(),
                    "bytes_written": content.len()
                })))
            }
            "edit" => {
                let path = self.checked_path(self.required_string(&args, "path")?, &working_dir)?;
                let old_text = self.required_string(&args, "old_text")?;
                let new_text = args
                    .get("new_text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let replace_all = Self::optional_bool(&args, "replace_all", false);

                let metadata = fs::metadata(&path).map_err(|err| {
                    Error::Tool(format!(
                        "failed to read metadata for '{}': {err}",
                        path.display()
                    ))
                })?;
                if metadata.len() > MAX_EDIT_BYTES {
                    return Err(Error::Tool(format!(
                        "file '{}' is too large to edit safely ({} bytes, limit {} bytes)",
                        path.display(),
                        metadata.len(),
                        MAX_EDIT_BYTES
                    )));
                }

                let mut content = fs::read_to_string(&path).map_err(|err| {
                    Error::Tool(format!(
                        "failed to read '{}' for edit: {err}",
                        path.display()
                    ))
                })?;

                if !content.contains(old_text) {
                    return Err(Error::Tool(format!(
                        "text to replace was not found in '{}'",
                        path.display()
                    )));
                }

                if replace_all {
                    content = content.replace(old_text, new_text);
                } else {
                    content = content.replacen(old_text, new_text, 1);
                }

                fs::write(&path, content.as_bytes()).map_err(|err| {
                    Error::Tool(format!(
                        "failed to write edited content to '{}': {err}",
                        path.display()
                    ))
                })?;

                Ok(Self::operation_result(json!({
                    "path": path.to_string_lossy(),
                    "replace_all": replace_all
                })))
            }
            "list" => {
                let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
                let path = self.checked_path(raw, &working_dir)?;
                let recursive = Self::optional_bool(&args, "recursive", false);
                let include_hidden = Self::optional_bool(&args, "include_hidden", false);

                let mut queue = VecDeque::new();
                queue.push_back(path.clone());
                let mut entries = Vec::new();

                while let Some(current_dir) = queue.pop_front() {
                    let iter = fs::read_dir(&current_dir).map_err(|err| {
                        Error::Tool(format!("failed to list '{}': {err}", current_dir.display()))
                    })?;
                    for entry in iter {
                        let entry = entry.map_err(|err| {
                            Error::Tool(format!(
                                "failed to read entry in '{}': {err}",
                                current_dir.display()
                            ))
                        })?;
                        let entry_path = entry.path();
                        let file_name = entry.file_name().to_string_lossy().into_owned();
                        if !include_hidden && file_name.starts_with('.') {
                            continue;
                        }

                        let symlink_metadata =
                            fs::symlink_metadata(&entry_path).map_err(|err| {
                                Error::Tool(format!(
                                    "failed to read symlink metadata for '{}': {err}",
                                    entry_path.display()
                                ))
                            })?;
                        let is_symlink = symlink_metadata.file_type().is_symlink();
                        let metadata = entry.metadata().map_err(|err| {
                            Error::Tool(format!(
                                "failed to read metadata for '{}': {err}",
                                entry_path.display()
                            ))
                        })?;
                        let is_dir = metadata.is_dir();

                        entries.push(json!({
                            "path": entry_path.to_string_lossy(),
                            "name": file_name,
                            "kind": if is_symlink { "symlink" } else if is_dir { "directory" } else { "file" },
                            "size": metadata.len()
                        }));

                        if recursive && is_dir && !is_symlink {
                            queue.push_back(entry_path);
                        }
                    }
                }

                Ok(Self::operation_result(json!({
                    "path": path.to_string_lossy(),
                    "recursive": recursive,
                    "entries": entries
                })))
            }
            "mkdir" => {
                let path = self.checked_path(self.required_string(&args, "path")?, &working_dir)?;
                let recursive = Self::optional_bool(&args, "recursive", true);

                if recursive {
                    let depth = path
                        .strip_prefix(&working_dir)
                        .map(|relative| relative.components().count())
                        .unwrap_or(usize::MAX / 2);
                    if depth > MAX_MKDIR_DEPTH {
                        return Err(Error::Tool(format!(
                            "directory creation exceeded max depth ({MAX_MKDIR_DEPTH})"
                        )));
                    }
                }

                if recursive {
                    fs::create_dir_all(&path).map_err(|err| {
                        Error::Tool(format!(
                            "failed to create directory '{}': {err}",
                            path.display()
                        ))
                    })?;
                } else {
                    fs::create_dir(&path).map_err(|err| {
                        Error::Tool(format!(
                            "failed to create directory '{}': {err}",
                            path.display()
                        ))
                    })?;
                }

                Ok(Self::operation_result(json!({
                    "path": path.to_string_lossy(),
                    "recursive": recursive
                })))
            }
            "delete" => {
                let path = self.checked_path(self.required_string(&args, "path")?, &working_dir)?;
                let recursive = Self::optional_bool(&args, "recursive", false);

                if path == working_dir {
                    return Err(Error::Tool(
                        "deleting working directory is not allowed".to_owned(),
                    ));
                }

                if !path.exists() {
                    return Err(Error::Tool(format!(
                        "path '{}' does not exist",
                        path.display()
                    )));
                }

                let metadata = fs::metadata(&path).map_err(|err| {
                    Error::Tool(format!(
                        "failed to read metadata for '{}': {err}",
                        path.display()
                    ))
                })?;
                if metadata.is_dir() {
                    if recursive {
                        Self::enforce_recursive_delete_limits(&path)?;
                        fs::remove_dir_all(&path).map_err(|err| {
                            Error::Tool(format!(
                                "failed to delete directory '{}': {err}",
                                path.display()
                            ))
                        })?;
                    } else {
                        fs::remove_dir(&path).map_err(|err| {
                            Error::Tool(format!(
                                "failed to delete directory '{}' (set recursive=true for non-empty directories): {err}",
                                path.display()
                            ))
                        })?;
                    }
                } else {
                    fs::remove_file(&path).map_err(|err| {
                        Error::Tool(format!("failed to delete file '{}': {err}", path.display()))
                    })?;
                }

                Ok(Self::operation_result(json!({
                    "path": path.to_string_lossy(),
                    "recursive": recursive
                })))
            }
            "copy" => {
                let source =
                    self.checked_path(self.required_string(&args, "source")?, &working_dir)?;
                let destination =
                    self.checked_path(self.required_string(&args, "destination")?, &working_dir)?;
                let recursive = Self::optional_bool(&args, "recursive", false);

                if !source.exists() {
                    return Err(Error::Tool(format!(
                        "source '{}' does not exist",
                        source.display()
                    )));
                }

                let metadata = fs::metadata(&source).map_err(|err| {
                    Error::Tool(format!(
                        "failed to read metadata for '{}': {err}",
                        source.display()
                    ))
                })?;

                if metadata.is_dir() {
                    if !recursive {
                        return Err(Error::Tool(
                            "copying a directory requires recursive=true".to_owned(),
                        ));
                    }
                    let mut copied_entries = 0usize;
                    Self::copy_directory_recursive(&source, &destination, 0, &mut copied_entries)?;
                } else {
                    if let Some(parent) = destination.parent() {
                        fs::create_dir_all(parent).map_err(|err| {
                            Error::Tool(format!(
                                "failed to create destination parent '{}': {err}",
                                parent.display()
                            ))
                        })?;
                    }

                    fs::copy(&source, &destination).map_err(|err| {
                        Error::Tool(format!(
                            "failed to copy '{}' -> '{}': {err}",
                            source.display(),
                            destination.display()
                        ))
                    })?;
                }

                Ok(Self::operation_result(json!({
                    "source": source.to_string_lossy(),
                    "destination": destination.to_string_lossy(),
                    "recursive": recursive
                })))
            }
            "move" => {
                let source =
                    self.checked_path(self.required_string(&args, "source")?, &working_dir)?;
                let destination =
                    self.checked_path(self.required_string(&args, "destination")?, &working_dir)?;

                if !source.exists() {
                    return Err(Error::Tool(format!(
                        "source '{}' does not exist",
                        source.display()
                    )));
                }

                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(|err| {
                        Error::Tool(format!(
                            "failed to create destination parent '{}': {err}",
                            parent.display()
                        ))
                    })?;
                }

                match fs::rename(&source, &destination) {
                    Ok(()) => {}
                    Err(_) => {
                        let metadata = fs::metadata(&source).map_err(|err| {
                            Error::Tool(format!(
                                "failed to read metadata for '{}' during move fallback: {err}",
                                source.display()
                            ))
                        })?;
                        if metadata.is_dir() {
                            let mut copied_entries = 0usize;
                            Self::copy_directory_recursive(
                                &source,
                                &destination,
                                0,
                                &mut copied_entries,
                            )?;
                            fs::remove_dir_all(&source).map_err(|err| {
                                Error::Tool(format!(
                                    "failed to remove source directory '{}' after copy: {err}",
                                    source.display()
                                ))
                            })?;
                        } else {
                            fs::copy(&source, &destination).map_err(|err| {
                                Error::Tool(format!(
                                    "failed to copy source '{}' during move fallback: {err}",
                                    source.display()
                                ))
                            })?;
                            fs::remove_file(&source).map_err(|err| {
                                Error::Tool(format!(
                                    "failed to remove source file '{}' after copy: {err}",
                                    source.display()
                                ))
                            })?;
                        }
                    }
                }

                Ok(Self::operation_result(json!({
                    "source": source.to_string_lossy(),
                    "destination": destination.to_string_lossy()
                })))
            }
            "info" => {
                let path = self.checked_path(self.required_string(&args, "path")?, &working_dir)?;
                let metadata = fs::metadata(&path).map_err(|err| {
                    Error::Tool(format!(
                        "failed to read metadata for '{}': {err}",
                        path.display()
                    ))
                })?;
                let include_hash = Self::optional_bool(&args, "include_hash", false);
                let algorithm = args
                    .get("hash_algorithm")
                    .and_then(Value::as_str)
                    .unwrap_or("sha256");

                let hash = if include_hash && metadata.is_file() {
                    Some(self.hash_file(&path, algorithm)?)
                } else {
                    None
                };

                Ok(Self::operation_result(json!({
                    "path": path.to_string_lossy(),
                    "is_file": metadata.is_file(),
                    "is_dir": metadata.is_dir(),
                    "size": metadata.len(),
                    "readonly": metadata.permissions().readonly(),
                    "hash_algorithm": if hash.is_some() { Some(algorithm) } else { None::<&str> },
                    "hash": hash
                })))
            }
            "glob" => {
                let pattern = self.required_string(&args, "pattern")?;
                let base_raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
                let base_path = self.checked_path(base_raw, &working_dir)?;
                let mut queue = VecDeque::new();
                queue.push_back((base_path.clone(), 0usize));
                let mut matches = Vec::new();
                let mut visited = HashSet::new();
                visited.insert(base_path.clone());
                let mut visited_entries = 0usize;

                while let Some((current_dir, depth)) = queue.pop_front() {
                    if depth > MAX_GLOB_DEPTH {
                        continue;
                    }
                    let iter = fs::read_dir(&current_dir).map_err(|err| {
                        Error::Tool(format!("failed to list '{}': {err}", current_dir.display()))
                    })?;
                    for entry in iter {
                        if visited_entries >= MAX_GLOB_ENTRIES {
                            return Err(Error::Tool(format!(
                                "glob search exceeded entry limit ({MAX_GLOB_ENTRIES})"
                            )));
                        }
                        visited_entries += 1;

                        let entry = entry.map_err(|err| {
                            Error::Tool(format!(
                                "failed to read entry in '{}': {err}",
                                current_dir.display()
                            ))
                        })?;
                        let entry_path = entry.path();
                        let symlink_metadata =
                            fs::symlink_metadata(&entry_path).map_err(|err| {
                                Error::Tool(format!(
                                    "failed to read symlink metadata for '{}': {err}",
                                    entry_path.display()
                                ))
                            })?;
                        let is_symlink = symlink_metadata.file_type().is_symlink();
                        let metadata = entry.metadata().map_err(|err| {
                            Error::Tool(format!(
                                "failed to read metadata for '{}': {err}",
                                entry_path.display()
                            ))
                        })?;
                        if metadata.is_dir() && !is_symlink {
                            let canonical = fs::canonicalize(&entry_path).map_err(|err| {
                                Error::Tool(format!(
                                    "failed to canonicalize '{}' during glob: {err}",
                                    entry_path.display()
                                ))
                            })?;
                            if canonical.starts_with(&base_path) && visited.insert(canonical) {
                                queue.push_back((entry_path.clone(), depth + 1));
                            }
                        }

                        let rel = entry_path
                            .strip_prefix(&base_path)
                            .unwrap_or(entry_path.as_path())
                            .components()
                            .map(|component| component.as_os_str().to_string_lossy().into_owned())
                            .collect::<Vec<String>>()
                            .join("/");

                        if simple_glob_match(pattern, &rel) {
                            matches.push(entry_path.to_string_lossy().into_owned());
                        }
                    }
                }

                Ok(Self::operation_result(json!({
                    "pattern": pattern,
                    "base_path": base_path.to_string_lossy(),
                    "matches": matches
                })))
            }
            "hash" => {
                let path = self.checked_path(self.required_string(&args, "path")?, &working_dir)?;
                if !path.is_file() {
                    return Err(Error::Tool(format!(
                        "path '{}' is not a file",
                        path.display()
                    )));
                }
                let algorithm = args
                    .get("algorithm")
                    .and_then(Value::as_str)
                    .unwrap_or("sha256");
                let digest = self.hash_file(&path, algorithm)?;
                Ok(Self::operation_result(json!({
                    "path": path.to_string_lossy(),
                    "algorithm": algorithm,
                    "digest": digest
                })))
            }
            other => Err(Error::Tool(format!(
                "unsupported filesystem operation '{other}'"
            ))),
        }
    }
}

#[async_trait]
impl Tool for FilesystemTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Filesystem operations with path guardrails"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        self.execute_operation(args, context)
    }

    async fn stream_execute(
        &self,
        args: Value,
        tx: mpsc::Sender<Event>,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let tool_name = self.name().to_owned();
        let _ = tx.try_send(Event::ToolStarted {
            tool: tool_name.clone(),
            args: args.clone(),
        });

        let result = self.execute_operation(args, context)?;

        let _ = tx.try_send(Event::ToolCompleted {
            tool: tool_name,
            exit_code: 0,
        });

        Ok(result)
    }
}
