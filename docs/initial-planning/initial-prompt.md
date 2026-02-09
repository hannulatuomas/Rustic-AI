I want to build a perfect Agentic AI system (like claude code, opencode, claude cowork, codex, etc).

* I want to write it in Rust
* I want it to be well structured, extendable, easy to maintain and expand, clean (no bloat), feature rich (no MVP), etc.
* Everything needs to be done according to best practices
* I will skip testing and commit hooks at first and focus purely to get the core running
* I will focus only to backend first and frontend facing things will be done later
* It must be able to support ALL the features Claude Code, OpenCode etc. have and more
* It needs to be customizable, everything should be easily configurable

* Must have features to support:
  * Agents, sub-agents, multi-agent systems
  * Rules (.cursorrules, .windsurfrules, etc), context files (AGENTS.md, CLAUDE.md, etc.)
  * Workflows, slash commands, saved prompts, agents, sub-agents, SKILLS, tools...
  * Context management and memory (important for long-running conversations, make efficient)
  * State management (important for long-running conversations, make efficient)
  * Conversation history and tracking (sessions)
  * Tool integration and execution
  * Model provider support (OpenAI, Anthropic, Z.ai, Grok, Google, etc.)
  * Allow the use of existing subscriptions for model providers (OpenAI, Anthropic, etc.)
  * Local models (Ollama, llama-cpp, etc.)
  * Streaming and async operations
  * Error handling and retries
  * Graceful degradation
  * Easy to add providers, tools, agents
  * Workflows, commands, agents, skills, tools
  * Remote execution capabilities (ssh to remote machines, execute tools there and get results back)
  * Progress tracking and status updates
  * Handle multi-agent systems (multiple agents working together) efficiently, share only necessary context to each agent to save context size
  * Parallel agent execution

* Core Principles:
  * **Performance First**: Zero-cost abstractions, async I/O, minimal allocations
  * **Correctness**: Type safety, error handling
  * **Feature Rich**: Every capability OpenCode has + more
  * **Extensible**: Easy to add providers, tools, agents

* Agent Use Cases:
  * Programming and Scripting
  * Linux and Windows Administration
  * Database Maintenance and Data Analysis
  * API Development and Maintenance
  * Cyber Security (ethical hacking, penetration testing, CTF, Malware analysis, etc.)
  * Microsoft / Azure Development
  * DevOps
  * AI / ML designing and development
  * Game Development
  * Infrastructure as Code
  * General IT, coding, development, maintaining, designing
  * Docker/Podman
  * Container orchestration
  * Cloud infrastructure
  * Servers
  * Virtual Machines
  * Networking
  * Microsoft Ecosystem (administrator)

 
I want you to carefully plan this project for me. I need a detailed and comprehensive plan for this project. I'm using AI (OpenCode) to implement this, so make the plan AI friendly (give clear tasks, phases, todos, etc.)
 
Please take your time and make me the BEST plan possible. Act as an experienced (>20 years experience) senior Software and AI engineer.

