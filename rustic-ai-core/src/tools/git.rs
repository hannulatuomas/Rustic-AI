use async_trait::async_trait;
use git2::{
    BranchType, DiffFormat, DiffOptions, FetchOptions, IndexAddOption, ObjectType, Oid, Repository,
    StatusOptions,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use crate::config::schema::{AgentPermissionMode, ToolConfig};
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct GitTool {
    config: ToolConfig,
    schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitCommand {
    Clone,
    Pull,
    Push,
    Commit,
    Status,
    Diff,
    Branch,
    Tag,
    Log,
    Checkout,
}

impl GitCommand {
    fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "clone" => Ok(Self::Clone),
            "pull" => Ok(Self::Pull),
            "push" => Ok(Self::Push),
            "commit" => Ok(Self::Commit),
            "status" => Ok(Self::Status),
            "diff" => Ok(Self::Diff),
            "branch" => Ok(Self::Branch),
            "tag" => Ok(Self::Tag),
            "log" => Ok(Self::Log),
            "checkout" => Ok(Self::Checkout),
            other => Err(Error::Tool(format!(
                "unsupported git command '{other}' (expected clone|pull|push|commit|status|diff|branch|tag|log|checkout)"
            ))),
        }
    }

    fn is_write(self) -> bool {
        !matches!(self, Self::Status | Self::Diff | Self::Log)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Clone => "clone",
            Self::Pull => "pull",
            Self::Push => "push",
            Self::Commit => "commit",
            Self::Status => "status",
            Self::Diff => "diff",
            Self::Branch => "branch",
            Self::Tag => "tag",
            Self::Log => "log",
            Self::Checkout => "checkout",
        }
    }
}

impl GitTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "enum": ["clone", "pull", "push", "commit", "status", "diff", "branch", "tag", "log", "checkout"]
                },
                "repo_path": { "type": "string" },
                "url": { "type": "string" },
                "target_path": { "type": "string" },
                "remote": { "type": "string" },
                "branch": { "type": "string" },
                "message": { "type": "string" },
                "author_name": { "type": "string" },
                "author_email": { "type": "string" },
                "all": { "type": "boolean" },
                "paths": { "type": "array", "items": { "type": "string" } },
                "staged": { "type": "boolean" },
                "max_count": { "type": "integer", "minimum": 1, "maximum": 500 },
                "action": { "type": "string" },
                "name": { "type": "string" },
                "start_point": { "type": "string" },
                "create_branch": { "type": "boolean" },
                "force": { "type": "boolean" },
                "annotated": { "type": "boolean" },
                "target": { "type": "string" }
            },
            "required": ["command"]
        });

        Self { config, schema }
    }

    fn command_from_args(args: &Value) -> Result<GitCommand> {
        let command = args
            .get("command")
            .or_else(|| args.get("operation"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool("missing 'command' argument".to_owned()))?;
        GitCommand::parse(command)
    }

    fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Tool(format!("missing '{key}' argument")))
    }

    fn optional_string<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }

    fn optional_paths(args: &Value, key: &str) -> Result<Vec<String>> {
        let Some(raw) = args.get(key) else {
            return Ok(Vec::new());
        };
        let Some(items) = raw.as_array() else {
            return Err(Error::Tool(format!("'{key}' must be an array of strings")));
        };

        let mut paths = Vec::with_capacity(items.len());
        for item in items {
            let Some(path) = item.as_str() else {
                return Err(Error::Tool(format!("'{key}' must be an array of strings")));
            };
            let trimmed = path.trim();
            if trimmed.is_empty() {
                continue;
            }
            paths.push(trimmed.to_owned());
        }
        Ok(paths)
    }

    fn canonicalize_for_guard(path: &Path) -> std::io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn resolve_path_within_workspace(
        &self,
        context: &ToolExecutionContext,
        raw: Option<&str>,
        allow_missing_leaf: bool,
    ) -> Result<PathBuf> {
        let base = Self::canonicalize_for_guard(&context.working_directory).map_err(|err| {
            Error::Tool(format!(
                "failed to resolve path '{}': {err}",
                context.working_directory.display()
            ))
        })?;
        let requested = match raw {
            Some(text) => {
                let candidate = PathBuf::from(text);
                if candidate.is_absolute() {
                    candidate
                } else {
                    base.join(candidate)
                }
            }
            None => base.clone(),
        };

        match Self::canonicalize_for_guard(&requested) {
            Ok(resolved) => {
                if !resolved.starts_with(&base) {
                    return Err(Error::Tool(format!(
                        "path '{}' is outside tool working directory '{}'; use a path within the workspace",
                        resolved.display(),
                        base.display()
                    )));
                }
                return Ok(resolved);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(Error::Tool(format!(
                    "failed to resolve path '{}': {err}",
                    requested.display()
                )));
            }
        }

        if allow_missing_leaf {
            let parent = requested.parent().ok_or_else(|| {
                Error::Tool(format!(
                    "path '{}' has no parent directory",
                    requested.display()
                ))
            })?;
            let parent_resolved = Self::canonicalize_for_guard(parent).map_err(|err| {
                Error::Tool(format!(
                    "failed to resolve path '{}': {err}",
                    parent.display()
                ))
            })?;
            if !parent_resolved.starts_with(&base) {
                return Err(Error::Tool(format!(
                    "path '{}' is outside tool working directory '{}'; use a path within the workspace",
                    requested.display(),
                    base.display()
                )));
            }
            return Ok(requested);
        }

        Err(Error::Tool(format!(
            "path '{}' does not exist",
            requested.display()
        )))
    }

    fn discover_repo(path: &Path) -> Result<Repository> {
        Repository::discover(path).map_err(|err| {
            Error::Tool(format!(
                "failed to open git repository from '{}': {err}",
                path.display()
            ))
        })
    }

    fn ensure_read_write_allowed(
        command: GitCommand,
        context: &ToolExecutionContext,
    ) -> Result<()> {
        if command.is_write() && context.agent_permission_mode == AgentPermissionMode::ReadOnly {
            return Err(Error::Tool(format!(
                "git command '{}' is blocked in read-only agent mode",
                command.as_str()
            )));
        }
        Ok(())
    }

    fn oid_to_short(repo: &Repository, oid: Oid) -> String {
        repo.find_object(oid, Some(ObjectType::Commit))
            .ok()
            .and_then(|obj| obj.short_id().ok())
            .and_then(|buf| buf.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| oid.to_string())
    }

    fn run_clone(&self, args: &Value, context: &ToolExecutionContext) -> Result<Value> {
        let url = Self::required_string(args, "url")?;
        let target_path = self.resolve_path_within_workspace(
            context,
            Self::optional_string(args, "target_path"),
            true,
        )?;
        let repo = Repository::clone(url, &target_path).map_err(|err| {
            Error::Tool(format!(
                "failed to clone '{}' into '{}': {err}",
                url,
                target_path.display()
            ))
        })?;
        let head = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| oid.to_string())
            .unwrap_or_default();
        Ok(json!({
            "command": "clone",
            "url": url,
            "target_path": target_path.to_string_lossy(),
            "head": head,
        }))
    }

    fn run_status(&self, args: &Value, context: &ToolExecutionContext) -> Result<Value> {
        let repo_path = self.resolve_path_within_workspace(
            context,
            Self::optional_string(args, "repo_path"),
            false,
        )?;
        let repo = Self::discover_repo(&repo_path)?;

        let include_untracked = args
            .get("include_untracked")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let mut options = StatusOptions::new();
        options
            .include_untracked(include_untracked)
            .renames_head_to_index(true)
            .renames_index_to_workdir(true)
            .include_unmodified(false);

        let statuses = repo
            .statuses(Some(&mut options))
            .map_err(|err| Error::Tool(format!("failed to read git status: {err}")))?;

        let mut entries = Vec::new();
        for entry in statuses.iter() {
            let path = entry.path().unwrap_or_default().to_owned();
            let status = format!("{:?}", entry.status());
            entries.push(json!({ "path": path, "status": status }));
        }

        Ok(json!({
            "command": "status",
            "repo": repo.path().display().to_string(),
            "count": entries.len(),
            "entries": entries,
        }))
    }

    fn run_diff(&self, args: &Value, context: &ToolExecutionContext) -> Result<Value> {
        let repo_path = self.resolve_path_within_workspace(
            context,
            Self::optional_string(args, "repo_path"),
            false,
        )?;
        let repo = Self::discover_repo(&repo_path)?;
        let staged = args.get("staged").and_then(Value::as_bool).unwrap_or(false);

        let mut diff_options = DiffOptions::new();
        let diff = if staged {
            let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
            repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut diff_options))
                .map_err(|err| Error::Tool(format!("failed to compute staged diff: {err}")))?
        } else {
            repo.diff_index_to_workdir(None, Some(&mut diff_options))
                .map_err(|err| Error::Tool(format!("failed to compute working-tree diff: {err}")))?
        };

        let mut patch = String::new();
        diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
            patch.push_str(&String::from_utf8_lossy(line.content()));
            true
        })
        .map_err(|err| Error::Tool(format!("failed to format diff output: {err}")))?;

        Ok(json!({
            "command": "diff",
            "staged": staged,
            "patch": patch,
        }))
    }

    fn run_log(&self, args: &Value, context: &ToolExecutionContext) -> Result<Value> {
        let repo_path = self.resolve_path_within_workspace(
            context,
            Self::optional_string(args, "repo_path"),
            false,
        )?;
        let repo = Self::discover_repo(&repo_path)?;
        let max_count = args
            .get("max_count")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 500) as usize;

        let mut revwalk = repo
            .revwalk()
            .map_err(|err| Error::Tool(format!("failed to initialize revwalk: {err}")))?;
        revwalk
            .push_head()
            .map_err(|err| Error::Tool(format!("failed to walk HEAD: {err}")))?;

        let mut commits = Vec::new();
        for oid in revwalk.take(max_count).flatten() {
            let commit = repo
                .find_commit(oid)
                .map_err(|err| Error::Tool(format!("failed to load commit {oid}: {err}")))?;
            commits.push(json!({
                "id": oid.to_string(),
                "short_id": Self::oid_to_short(&repo, oid),
                "summary": commit.summary().unwrap_or_default(),
                "author": commit.author().name().unwrap_or_default(),
                "time": commit.time().seconds(),
            }));
        }

        Ok(json!({
            "command": "log",
            "count": commits.len(),
            "commits": commits,
        }))
    }

    fn run_commit(&self, args: &Value, context: &ToolExecutionContext) -> Result<Value> {
        let repo_path = self.resolve_path_within_workspace(
            context,
            Self::optional_string(args, "repo_path"),
            false,
        )?;
        let repo = Self::discover_repo(&repo_path)?;
        let message = Self::required_string(args, "message")?;
        let stage_all = args.get("all").and_then(Value::as_bool).unwrap_or(true);
        let explicit_paths = Self::optional_paths(args, "paths")?;

        let mut index = repo
            .index()
            .map_err(|err| Error::Tool(format!("failed to read index: {err}")))?;
        if stage_all {
            index
                .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
                .map_err(|err| Error::Tool(format!("failed to stage files: {err}")))?;
        } else {
            for path in explicit_paths {
                index
                    .add_path(Path::new(&path))
                    .map_err(|err| Error::Tool(format!("failed to stage '{path}': {err}")))?;
            }
        }

        index
            .write()
            .map_err(|err| Error::Tool(format!("failed to write index: {err}")))?;
        let tree_id = index
            .write_tree()
            .map_err(|err| Error::Tool(format!("failed to write tree: {err}")))?;
        let tree = repo
            .find_tree(tree_id)
            .map_err(|err| Error::Tool(format!("failed to load tree: {err}")))?;

        let author_name = Self::optional_string(args, "author_name").unwrap_or("Rustic AI");
        let author_email = Self::optional_string(args, "author_email").unwrap_or("rustic-ai@local");
        let signature = git2::Signature::now(author_name, author_email)
            .map_err(|err| Error::Tool(format!("failed to build commit signature: {err}")))?;

        let mut parent_commits = Vec::new();
        if let Ok(head) = repo.head() {
            if let Some(parent_oid) = head.target() {
                let parent = repo.find_commit(parent_oid).map_err(|err| {
                    Error::Tool(format!("failed to load parent commit {parent_oid}: {err}"))
                })?;
                parent_commits.push(parent);
            }
        }
        let parent_refs = parent_commits.iter().collect::<Vec<_>>();

        let commit_id = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parent_refs,
            )
            .map_err(|err| Error::Tool(format!("failed to create commit: {err}")))?;

        Ok(json!({
            "command": "commit",
            "id": commit_id.to_string(),
            "short_id": Self::oid_to_short(&repo, commit_id),
            "message": message,
        }))
    }

    fn run_checkout(&self, args: &Value, context: &ToolExecutionContext) -> Result<Value> {
        let repo_path = self.resolve_path_within_workspace(
            context,
            Self::optional_string(args, "repo_path"),
            false,
        )?;
        let repo = Self::discover_repo(&repo_path)?;
        let branch = Self::required_string(args, "branch")?;
        let create_branch = args
            .get("create_branch")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if create_branch {
            let start_ref = Self::optional_string(args, "start_point").unwrap_or("HEAD");
            let target_obj = repo.revparse_single(start_ref).map_err(|err| {
                Error::Tool(format!(
                    "failed to resolve start_point '{start_ref}': {err}"
                ))
            })?;
            let target_commit = target_obj.peel_to_commit().map_err(|err| {
                Error::Tool(format!("start_point '{start_ref}' is not a commit: {err}"))
            })?;
            repo.branch(branch, &target_commit, false)
                .map_err(|err| Error::Tool(format!("failed to create branch '{branch}': {err}")))?;
        }

        let local_ref = format!("refs/heads/{branch}");
        repo.set_head(&local_ref)
            .map_err(|err| Error::Tool(format!("failed to set HEAD to '{local_ref}': {err}")))?;
        repo.checkout_head(None)
            .map_err(|err| Error::Tool(format!("failed to checkout '{branch}': {err}")))?;

        Ok(json!({
            "command": "checkout",
            "branch": branch,
            "created": create_branch,
        }))
    }

    fn run_branch(&self, args: &Value, context: &ToolExecutionContext) -> Result<Value> {
        let repo_path = self.resolve_path_within_workspace(
            context,
            Self::optional_string(args, "repo_path"),
            false,
        )?;
        let repo = Self::discover_repo(&repo_path)?;
        let action = Self::optional_string(args, "action").unwrap_or("list");

        match action {
            "list" => {
                let mut branches = Vec::new();
                for branch in repo
                    .branches(Some(BranchType::Local))
                    .map_err(|err| Error::Tool(format!("failed to list branches: {err}")))?
                {
                    let (branch, _) = branch.map_err(|err| {
                        Error::Tool(format!("failed to read branch entry: {err}"))
                    })?;
                    let name = branch.name().ok().flatten().unwrap_or_default().to_owned();
                    let is_head = branch.is_head();
                    branches.push(json!({"name": name, "is_head": is_head}));
                }
                Ok(json!({"command": "branch", "action": "list", "branches": branches}))
            }
            "current" => {
                let current = repo
                    .head()
                    .ok()
                    .and_then(|head| head.shorthand().map(ToOwned::to_owned))
                    .unwrap_or_default();
                Ok(json!({"command": "branch", "action": "current", "name": current}))
            }
            "create" => {
                let name = Self::required_string(args, "name")?;
                let start_point = Self::optional_string(args, "start_point").unwrap_or("HEAD");
                let target = repo.revparse_single(start_point).map_err(|err| {
                    Error::Tool(format!(
                        "failed to resolve start_point '{start_point}': {err}"
                    ))
                })?;
                let commit = target.peel_to_commit().map_err(|err| {
                    Error::Tool(format!(
                        "start_point '{start_point}' is not a commit: {err}"
                    ))
                })?;
                repo.branch(name, &commit, false).map_err(|err| {
                    Error::Tool(format!("failed to create branch '{name}': {err}"))
                })?;
                Ok(
                    json!({"command": "branch", "action": "create", "name": name, "start_point": start_point}),
                )
            }
            "delete" => {
                let name = Self::required_string(args, "name")?;
                let mut branch = repo
                    .find_branch(name, BranchType::Local)
                    .map_err(|err| Error::Tool(format!("failed to find branch '{name}': {err}")))?;
                branch.delete().map_err(|err| {
                    Error::Tool(format!("failed to delete branch '{name}': {err}"))
                })?;
                Ok(json!({"command": "branch", "action": "delete", "name": name}))
            }
            other => Err(Error::Tool(format!(
                "unsupported branch action '{other}' (expected list|current|create|delete)"
            ))),
        }
    }

    fn run_tag(&self, args: &Value, context: &ToolExecutionContext) -> Result<Value> {
        let repo_path = self.resolve_path_within_workspace(
            context,
            Self::optional_string(args, "repo_path"),
            false,
        )?;
        let repo = Self::discover_repo(&repo_path)?;
        let action = Self::optional_string(args, "action").unwrap_or("list");

        match action {
            "list" => {
                let names = repo
                    .tag_names(None)
                    .map_err(|err| Error::Tool(format!("failed to list tags: {err}")))?
                    .iter()
                    .flatten()
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                Ok(json!({"command": "tag", "action": "list", "tags": names}))
            }
            "create" => {
                let name = Self::required_string(args, "name")?;
                let target_ref = Self::optional_string(args, "target").unwrap_or("HEAD");
                let target = repo.revparse_single(target_ref).map_err(|err| {
                    Error::Tool(format!("failed to resolve target '{target_ref}': {err}"))
                })?;
                let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
                let annotated = args
                    .get("annotated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);

                if annotated {
                    let message = Self::optional_string(args, "message").unwrap_or(name);
                    let signature = repo
                        .signature()
                        .or_else(|_| git2::Signature::now("Rustic AI", "rustic-ai@local"))
                        .map_err(|err| {
                            Error::Tool(format!("failed to build tag signature: {err}"))
                        })?;
                    repo.tag(name, &target, &signature, message, force)
                        .map_err(|err| {
                            Error::Tool(format!("failed to create tag '{name}': {err}"))
                        })?;
                } else {
                    repo.tag_lightweight(name, &target, force).map_err(|err| {
                        Error::Tool(format!("failed to create tag '{name}': {err}"))
                    })?;
                }

                Ok(
                    json!({"command": "tag", "action": "create", "name": name, "target": target_ref, "annotated": annotated}),
                )
            }
            "delete" => {
                let name = Self::required_string(args, "name")?;
                repo.tag_delete(name)
                    .map_err(|err| Error::Tool(format!("failed to delete tag '{name}': {err}")))?;
                Ok(json!({"command": "tag", "action": "delete", "name": name}))
            }
            other => Err(Error::Tool(format!(
                "unsupported tag action '{other}' (expected list|create|delete)"
            ))),
        }
    }

    fn fast_forward_branch(repo: &Repository, branch: &str, target_oid: Oid) -> Result<()> {
        let branch_ref = format!("refs/heads/{branch}");
        if let Ok(mut local) = repo.find_reference(&branch_ref) {
            local
                .set_target(target_oid, "fast-forward")
                .map_err(|err| Error::Tool(format!("failed to fast-forward '{branch}': {err}")))?;
        } else {
            repo.reference(
                &branch_ref,
                target_oid,
                true,
                "set branch to fetched commit",
            )
            .map_err(|err| {
                Error::Tool(format!("failed to create local branch '{branch}': {err}"))
            })?;
        }
        repo.set_head(&branch_ref)
            .map_err(|err| Error::Tool(format!("failed to set HEAD to '{branch_ref}': {err}")))?;
        repo.checkout_head(None).map_err(|err| {
            Error::Tool(format!(
                "failed to checkout updated branch '{branch}': {err}"
            ))
        })?;
        Ok(())
    }

    fn run_pull(&self, args: &Value, context: &ToolExecutionContext) -> Result<Value> {
        let repo_path = self.resolve_path_within_workspace(
            context,
            Self::optional_string(args, "repo_path"),
            false,
        )?;
        let repo = Self::discover_repo(&repo_path)?;
        let remote_name = Self::optional_string(args, "remote").unwrap_or("origin");

        let branch = if let Some(branch) = Self::optional_string(args, "branch") {
            branch.to_owned()
        } else {
            repo.head()
                .ok()
                .and_then(|head| head.shorthand().map(ToOwned::to_owned))
                .ok_or_else(|| Error::Tool("unable to infer current branch for pull".to_owned()))?
        };

        let mut remote = repo
            .find_remote(remote_name)
            .map_err(|err| Error::Tool(format!("failed to find remote '{remote_name}': {err}")))?;
        remote
            .fetch(&[&branch], Some(&mut FetchOptions::new()), None)
            .map_err(|err| {
                Error::Tool(format!("failed to fetch '{remote_name}/{branch}': {err}"))
            })?;

        let fetch_ref = format!("refs/remotes/{remote_name}/{branch}");
        let fetch_head = repo.find_reference(&fetch_ref).map_err(|err| {
            Error::Tool(format!("failed to find fetched ref '{fetch_ref}': {err}"))
        })?;
        let fetch_annotated = repo
            .reference_to_annotated_commit(&fetch_head)
            .map_err(|err| Error::Tool(format!("failed to read fetched commit: {err}")))?;
        let (analysis, _) = repo
            .merge_analysis(&[&fetch_annotated])
            .map_err(|err| Error::Tool(format!("failed merge analysis: {err}")))?;

        if analysis.is_up_to_date() {
            return Ok(json!({
                "command": "pull",
                "remote": remote_name,
                "branch": branch,
                "status": "up_to_date",
            }));
        }

        if analysis.is_fast_forward() {
            let target_oid = fetch_head.target().ok_or_else(|| {
                Error::Tool(format!("fetched ref '{fetch_ref}' has no target oid"))
            })?;
            Self::fast_forward_branch(&repo, &branch, target_oid)?;
            return Ok(json!({
                "command": "pull",
                "remote": remote_name,
                "branch": branch,
                "status": "fast_forward",
                "target": target_oid.to_string(),
            }));
        }

        Err(Error::Tool(
            "pull requires merge/rebase; only fast-forward pull is supported currently".to_owned(),
        ))
    }

    fn run_push(&self, args: &Value, context: &ToolExecutionContext) -> Result<Value> {
        let repo_path = self.resolve_path_within_workspace(
            context,
            Self::optional_string(args, "repo_path"),
            false,
        )?;
        let repo = Self::discover_repo(&repo_path)?;
        let remote_name = Self::optional_string(args, "remote").unwrap_or("origin");
        let branch = if let Some(branch) = Self::optional_string(args, "branch") {
            branch.to_owned()
        } else {
            repo.head()
                .ok()
                .and_then(|head| head.shorthand().map(ToOwned::to_owned))
                .ok_or_else(|| Error::Tool("unable to infer current branch for push".to_owned()))?
        };

        let mut remote = repo
            .find_remote(remote_name)
            .map_err(|err| Error::Tool(format!("failed to find remote '{remote_name}': {err}")))?;
        let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
        remote
            .push(&[&refspec], None)
            .map_err(|err| Error::Tool(format!("failed to push '{refspec}': {err}")))?;

        Ok(json!({
            "command": "push",
            "remote": remote_name,
            "branch": branch,
            "refspec": refspec,
        }))
    }

    fn run_command(&self, args: &Value, context: &ToolExecutionContext) -> Result<Value> {
        let command = Self::command_from_args(args)?;
        Self::ensure_read_write_allowed(command, context)?;
        match command {
            GitCommand::Clone => self.run_clone(args, context),
            GitCommand::Pull => self.run_pull(args, context),
            GitCommand::Push => self.run_push(args, context),
            GitCommand::Commit => self.run_commit(args, context),
            GitCommand::Status => self.run_status(args, context),
            GitCommand::Diff => self.run_diff(args, context),
            GitCommand::Branch => self.run_branch(args, context),
            GitCommand::Tag => self.run_tag(args, context),
            GitCommand::Log => self.run_log(args, context),
            GitCommand::Checkout => self.run_checkout(args, context),
        }
    }

    async fn run_async(&self, args: Value, context: ToolExecutionContext) -> Result<ToolResult> {
        let this = self.clone();
        let payload = tokio::task::spawn_blocking(move || this.run_command(&args, &context))
            .await
            .map_err(|err| Error::Tool(format!("git tool task join error: {err}")))??;

        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: serde_json::to_string(&payload).map_err(|err| {
                Error::Tool(format!("failed to serialize git tool output: {err}"))
            })?,
        })
    }
}

#[async_trait]
impl Tool for GitTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Git repository operations backed by libgit2"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        self.run_async(args, context.clone()).await
    }

    async fn stream_execute(
        &self,
        args: Value,
        tx: mpsc::Sender<Event>,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let tool_name = self.name().to_owned();
        let command = Self::command_from_args(&args)
            .map(GitCommand::as_str)
            .unwrap_or("unknown")
            .to_owned();

        let _ = tx.try_send(Event::ToolStarted {
            tool: tool_name.clone(),
            args: args.clone(),
        });
        let _ = tx.try_send(Event::ToolOutput {
            tool: tool_name.clone(),
            stdout_chunk: format!("running git command '{command}'\n"),
            stderr_chunk: String::new(),
        });

        let result = self.run_async(args, context.clone()).await;

        let _ = tx.try_send(Event::ToolCompleted {
            tool: tool_name,
            exit_code: if result.is_ok() { 0 } else { 1 },
        });

        result
    }
}
