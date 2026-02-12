# Domain Tooling Implementation Plan

Last updated: 2026-02-12

## Purpose

This plan translates requirements coverage into an implementation roadmap for tools, agent presets, and safety controls. It is designed to be executed incrementally without losing quality or governance.

## Scope

Covered requirement areas:

- Programming languages
- Frameworks
- Linux administration/setup/maintenance (multi-distro)
- Databases and data analysis
- API development/maintenance
- Cyber security (pentest planning, malware triage, OSINT)
- Microsoft and Azure
- Windows administration/maintenance
- DevOps and SecDevOps
- AI/ML and prompt engineering
- Game development
- Infrastructure as Code
- Containers
- Cloud
- Servers
- VMs
- Networking

## Implementation Model

- Tier 0: Universal baseline tools for all agents
- Tier 1: Domain core tools required for primary tasks
- Tier 2: Domain advanced tools for deeper operations and diagnostics
- Tier 3: Governance/safety tools for production-grade operation

## Tier 0 (Universal Baseline)

Use these as default across most domain agents:

- `shell`
- `filesystem`
- `grep`
- `code_search`
- `git`
- `http`
- `web_fetch`
- `download`
- `process`
- `watch`
- `regex`
- `format`
- `encoding`
- `convert`

Baseline policy:

- Audit/discovery/security agents: `read_only`
- Build/ops/implementation agents: `read_write`
- Privileged actions require explicit policy checks and path/command allowlists

## Domain Tool Matrix

### Programming Languages

- Tier 1: `lsp`, `git`, `code_search`, `grep`, `filesystem`, `shell`
- Tier 2: language package/build wrappers
  - JS/TS: npm/pnpm/yarn
  - Python: uv/pip/poetry
  - .NET: dotnet
  - Go: go
  - Rust: cargo
  - Java: gradle/maven
  - C/C++: cmake/ninja/make
- Tier 3: dependency vulnerability auditing wrappers

### Frameworks

- Tier 1: `lsp`, `http`, `code_search`, `grep`, `filesystem`, `shell`
- Tier 2: framework diagnostics (route/config/spec checks)
- Tier 3: framework hardening checks (auth/CORS/headers/session defaults)

### Linux Administration (Multi-Distro)

- Tier 1: `shell`, `process`, `ssh`, `watch`, `filesystem`
- Tier 2: package/service/network/storage wrappers
  - apt/dnf/yum/pacman/apk/zypper/nix
  - systemctl/journalctl
  - ip/ss/nft/iptables
- Tier 3: baseline hardening and patch posture checks

### Databases and Data Analysis

- Tier 1: `database`, `format`, `convert`, `code_search`, `http`
- Tier 2: adapters for SQL and NoSQL families
- Tier 3: query plan/index advisor, migration diff, backup/restore verification

### API Development and Maintenance

- Tier 1: `http`, `web_fetch`, `format`, `convert`, `code_search`, `grep`
- Tier 2: OpenAPI/GraphQL/gRPC/SOAP tooling
- Tier 3: contract drift and auth/rate-limit regression checks

### Cyber Security

- Tier 1: `code_search`, `grep`, `http`, `web_fetch`, `web_search`, `filesystem`
- Tier 2: secret scanning, SAST/dependency scan, IOC utilities, malware static triage, authorized OSINT helpers
- Tier 3: strict scope controls, redaction, evidence logging, safe defaults

### Microsoft and Azure

- Tier 1: `http`, `shell`, `process`, `filesystem`
- Tier 2: Azure/Entra/M365/Intune/AzDO wrappers
- Tier 3: RBAC least-privilege, policy drift, cost/security posture checks

### Windows Administration and Maintenance

- Tier 1: `shell` (PowerShell/cmd), `process`, `filesystem`, `watch`
- Tier 2: event logs, services/tasks, Defender/firewall, WinRM diagnostics
- Tier 3: hardening baseline and remediation planning

### DevOps and SecDevOps

- Tier 1: `git`, `shell`, `process`, `http`, `watch`, `filesystem`
- Tier 2: CI/CD status/lint/release wrappers
- Tier 3: SBOM/provenance/pinning/supply-chain checks

### AI/ML and Prompt Engineering

- Tier 1: `filesystem`, `database`, `http`, `format`, `convert`, `code_search`
- Tier 2: eval harnesses, drift checks, MLOps validations
- Tier 3: privacy/PII checks, prompt regression gates, reproducibility checks

### Game Development

- Tier 1: `filesystem`, `code_search`, `git`, `image`, `shell`
- Tier 2: Unity/Godot/Unreal/Bevy build/export wrappers
- Tier 3: performance budget + asset hygiene checks

### Infrastructure as Code

- Tier 1: `filesystem`, `code_search`, `grep`, `shell`, `git`
- Tier 2: validate/plan wrappers for Terraform/OpenTofu/Bicep/Ansible
- Tier 3: policy-as-code and drift checks

### Containers

- Tier 1: `docker`, `process`, `filesystem`, `shell`, `http`
- Tier 2: compose/k8s diagnostics
- Tier 3: runtime hardening and least-privilege policy checks

### Cloud

- Tier 1: `http`, `shell`, `filesystem`, `process`
- Tier 2: Azure/AWS/GCP inventory and IAM wrappers
- Tier 3: posture and cost controls

### Servers and VMs

- Tier 1: `ssh`, `shell`, `process`, `watch`, `filesystem`
- Tier 2: hypervisor/host diagnostics wrappers
- Tier 3: patch, backup, restore, and runbook verification

### Networking

- Tier 1: `shell`, `http`, `process`, `watch`
- Tier 2: DNS/TLS/connectivity diagnostics wrappers
- Tier 3: firewall/rule audit and controlled change planning

## Priority Tool Backlog

P0 (highest impact):

1. `openapi_tool` (lint/diff/mock/contract checks)
2. `k8s_tool` (read-first diagnostics)
3. `iac_tool` (validate/plan for Terraform/OpenTofu/Bicep/Ansible)
4. `secrets_scan_tool` (redacted detection)
5. `sbom_tool` and `supply_chain_tool`

P1:

1. `windows_events_tool`
2. `systemd_tool`
3. `db_explain_tool`
4. `api_contract_tool`
5. `cloud_inventory_tool` (provider-neutral abstraction)

P2:

1. `malware_triage_tool` (static-only baseline)
2. `osint_tool` (authorized, source-tracked)
3. `perf_budget_tool` for app/game loops

## Agent Preset Strategy

For each domain preset:

- assign Tier 0 + domain Tier 1 tools
- include Tier 2 selectively when safe/needed
- keep Tier 3 checks available to audit/release agents
- use taxonomy membership for discoverability and router weighting

## Safety and Governance Requirements

- No destructive actions by default in infra/security domains
- Read-only defaults for audit/security/discovery roles
- Explicit escalation for write/privileged actions
- Per-tool command/path policy enforcement
- Always emit actionable error context without leaking secrets

## Execution Phases

Phase A: Core parity

- Ensure every requirement domain has at least one preset agent with correct Tier 1 tools
- Ensure routing keywords include each requirement domain

Phase B: P0 tooling

- Implement P0 tools and wire into manager + config schema + example config
- Add safety checks and permission enforcement

Phase C: P1 tooling and operability

- Add P1 tools
- Add command examples and operational docs

Phase D: P2 and quality gates

- Add specialized P2 tools
- Add regression checks and stricter validation gates

## Definition of Done

- Domain-to-agent mapping covers all requirement categories
- Tool manager supports all planned P0 tools
- Config example validates with strict schema/runtime checks
- Workspace builds and lints pass
- README and TODO reflect actual implementation state

## Verification Commands

- `python3 -m json.tool config.example.json`
- `cargo fmt --all`
- `cargo build --workspace`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo run -p rustic-ai-cli -- --config config.example.json validate-config --strict`
