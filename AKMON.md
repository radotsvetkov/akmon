# Akmon

Akmon is a Rust-native AI coding agent 
built for local-first, trust-first 
developer use.

## Project structure

- crates/akmon-core — security 
  primitives, FSM types, policy engine, 
  sandbox, secret types, audit events
- crates/akmon-models — LlmProvider 
  trait, OllamaBackend, message types, 
  streaming types
- crates/akmon-tools — Tool trait, 
  ReadFileTool, WriteFileTool, 
  ListDirectoryTool
- crates/akmon-query — AgentSession, 
  agent loop, context builder
- crates/akmon-cli — binary entry point, 
  CLI args, event printer

## Conventions

- No unwrap() outside tests
- Every public item has a doc comment
- All file paths go through sandbox 
  validation before use
- Secrets never appear in logs or 
  debug output
