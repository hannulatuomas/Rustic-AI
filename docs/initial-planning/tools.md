## Tool Inventory (AgentNexus-tools.txt)

File tools:

- read (offset/limit, binary support)
- write (overwrite protection, mkdir)
- edit (exact matching, replace_all)
- glob
- watch (file watching + events)
- delete (safety checks)
- list (metadata, recursive)
- copy (recursive, preserve metadata)
- move (rename, cross-filesystem)
- mkdir (nested, permissions)
- info (metadata + optional hashes)
- diff (unified diff)
- archive (zip/tar/tar.gz, list/extract)
- hash/checksum (md5/sha1/sha256/sha512)

Shell and process tools:

- bash (timeout, output capture)
- pty (interactive commands)
- background process management (spawn, status, terminate)
- command sandboxing (validation, path restrictions, whitelist/denylist)

Remote tools:

- ssh (command execution)
- remote tool execution (streaming output)
- remote sessions
- remote file ops (read/write/edit)
- remote execution security policies + audit
- remote - host file ops (move/copy files from/to remote, use scp)

Search and web tools:

- grep
- code_search (semantic-like similarity + ranking)
- web_fetch (http get, redirects, content types)
- http_api (post/put/delete, json body, upload)
- web_search
- download (progress, resume, chunking)
- crawler (html parsing, link extraction, robots)

Code intelligence:

- lsp (workspace/document symbol search)

Text processing:

- regex (multiline, groups, backrefs)
- format (json/xml, minify)
- encoding (base64/url/html entities, utf-8 validation)
- convert (md<->html, csv<->json, xml<->json, yaml<->json)

Integration tools:

- git (clone, pull/push, commit/status/diff, branches/tags)
- database (sqlite/postgres/mysql)
- image (resize/crop/rotate/convert/metadata)

Bracket validation tool:

- Validate (), {}, [], <> with nesting
- Ignore brackets inside comments and/or string literals
- Language rules:
  - comment prefixes (e.g. //, #, --)
  - block comment delimiters (e.g. /* */ with optional nesting)
  - string delimiters (single, double, backticks, raw forms)
- Output:
  - summary or detailed mismatch list
  - line/column positions
  - suggestions
  - supports max error cap
- Optional formatting targets:
  - CI annotations
  - LSP diagnostics

