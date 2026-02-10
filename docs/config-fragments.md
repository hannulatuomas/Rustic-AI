# Config Fragments

Rustic-AI supports split config files so you can manage providers, agents, tools, and permissions separately.

## Load Order

Effective config merge order is:

1. Base `config.json`
2. Global fragments (sorted by filename)
3. Project fragments (sorted by filename)
4. Environment variable overrides

Later sources override earlier values.

## Fragment Locations

- Global fragments:
  - `~/.rustic-ai/config/*.json`
  - Or `<storage.global_root_path>/config/*.json` if `storage.global_root_path` is configured
- Project fragments:
  - `<project>/<storage.default_root_dir_name>/config/*.json`
  - With defaults this is usually `.rustic-ai/config/*.json`

Each fragment file must contain a JSON object.

## Recommended Split Layout

Global:

```text
~/.rustic-ai/config/
  10-providers.json
  20-agents.json
  30-tools.json
  40-permissions.json
```

Project:

```text
<repo>/.rustic-ai/config/
  20-agents.json
  30-tools.json
  40-permissions.json
```

Use filename prefixes (`10-`, `20-`, etc.) to control deterministic merge precedence.

## Example Fragments

`40-permissions.json`

```json
{
  "permissions": {
    "globally_allowed_paths": ["~/dev", "/tmp"],
    "global_command_patterns": {
      "allow": ["git *", "cargo *"],
      "ask": ["sudo *"],
      "deny": ["rm -rf /", "dd *"]
    }
  }
}
```

`30-tools.json`

```json
{
  "tools": [
    {
      "name": "shell",
      "enabled": true,
      "permission_mode": "ask",
      "timeout_seconds": 30,
      "allowed_commands": [],
      "denied_commands": [],
      "working_dir": "project_root",
      "custom_working_dir": null,
      "env_passthrough": true,
      "stream_output": true
    }
  ]
}
```

Provider `settings` can include retry controls:

- `request_max_retries`
- `retry_base_delay_ms`
- `retry_max_delay_ms`
- `retry_jitter_ms`

## Agent Autonomy Limits

Agent autonomy can be tuned per agent with:

- `max_tool_rounds`
- `max_tools_per_round`
- `max_total_tool_calls_per_turn`
- `max_turn_duration_seconds`

For these fields:

- `null` (or omitted): use built-in defaults
- `0`: unlimited for that limit
- positive value: explicit cap

## REPL Permission Persistence

When using REPL commands:

- `/perm path add global <path>`
- `/perm cmd <allow|ask|deny> global <pattern>`
- `/perm path add project <path>`
- `/perm cmd <allow|ask|deny> project <pattern>`

Rustic-AI persists the change to `permissions.json` in the matching fragment directory and also applies it immediately in runtime.
