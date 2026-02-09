# Rustic-AI

Rustic-AI is a Rust-first, library-centric AI orchestration engine with a CLI frontend.

The core engine is UI-agnostic (`rustic-ai-core`), and user interfaces live under `frontend/`.

## Project Status

This repository is currently in scaffold + foundation stage.

- Workspace and crate layout are in place
- Core module boundaries are defined
- Planning and architecture docs are established
- Build and lint are green for the current scaffold

## Repository Layout

```text
.
├── rustic-ai-core/            # UI-agnostic engine library
├── frontend/
│   └── rustic-ai-cli/         # First consumer frontend (CLI)
├── docs/
│   ├── DESIGN_GUIDE.md
│   ├── DECISIONS.md
│   └── initial-planning/
├── IMPLEMENTATION_PLAN.md
└── AGENTS.md
```

## Architecture Principles

- Library-first: core logic stays in `rustic-ai-core`
- Frontend isolation: CLI/TUI/API/GUI consumers live under `frontend/`
- SQLite-first persistence with abstraction for future backends
- Strong typing and explicit boundaries over stringly protocols
- Configurable, guarded tool execution and extensibility

## Prerequisites

- Linux/macOS shell
- Rust toolchain (recommended via `rustup`)
- C build tools for native dependencies

Debian/Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev
```

Install Rust:

```bash
curl https://sh.rustup.rs -sSf | sh
```

If `cargo` is not in your PATH yet:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Build, Lint, Test

From repo root:

```bash
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

## Documentation You Should Read First

- `docs/DESIGN_GUIDE.md` - required workflow and definition of done
- `docs/DECISIONS.md` - architecture decisions (ADR-lite)
- `docs/initial-planning/big-picture.md` - product north star
- `docs/initial-planning/integration-plan.md` - subsystem boundaries and flow
- `docs/initial-planning/REQUIREMENTS.md` - capability and quality baseline
- `docs/initial-planning/tools.md` - target tools baseline

## Contributing

1. Follow the quality gate in `docs/DESIGN_GUIDE.md`.
2. Keep `rustic-ai-core` independent from frontend concerns.
3. Update docs and decisions when changing architecture, config, or boundaries.
4. Run build/lint/tests before opening a PR.

## License

TBD.
