# AGENTS.md

Repository: `declarative-migrations-test/cli-mcp-contract`  
Production dependency: `declarative-migrations/declarative-postgres-migrate.rs@a5e868acc0206fa9c3e91b5e36e0b1b111805885`

Use focused pull requests. Keep database tests deterministic and self-cleaning. Never weaken a failing convergence, rollback, drift, locking, atomicity, CLI, or MCP assertion merely to make CI green. Never commit credentials or production data. Resolve conflicts semantically with both sides and relevant history.

This repository certifies a Rust product with Rust test drivers. New repository validation, JSON-RPC adapters, subprocess guards, and contract runners must be written in Rust unless a pull request documents a concrete interoperability requirement. Preserve the immutable product gitlink and update every recorded SHA together.
