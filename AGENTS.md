# AGENTS.md

Repository: `declarative-migrations-test/cli-mcp-contract`  
Production dependency: `declarative-migrations/declarative-postgres-migrate.rs@a5e868acc0206fa9c3e91b5e36e0b1b111805885`

Use focused pull requests. Keep database tests deterministic and self-cleaning. Never weaken a failing convergence, rollback, drift, locking, atomicity, CLI, or MCP assertion merely to make CI green. Never commit credentials or production data. Resolve conflicts semantically with both sides and relevant history.

This repository certifies a Rust product with Rust test drivers. New repository validation, JSON-RPC adapters, subprocess guards, and contract runners must be written in Rust unless a pull request documents a concrete interoperability requirement. Preserve the immutable product gitlink and update every recorded SHA together.

## Repository-local Git worktrees

- Create or use a Git worktree only when the human operator explicitly authorizes it for the current task. Concurrency or a dirty checkout is not permission by itself.
- Put every authorized worktree at `<repository-root>/tmp/worktrees/<name>`; from the repository root, use `./tmp/worktrees/<name>`. Never place worktrees beside repositories or organization directories.
- Keep `tmp`, `temp`, `tmp/worktrees`, and `temp/worktrees` ignored in the repository-root `.gitignore`. Do not commit files from those directories.
- Relocate or remove a worktree only when the operator explicitly requests it. Before removal, preserve and publish intended changes, verify its commit is represented on the target branch, and confirm there are no tracked, untracked, ignored-sensitive, or in-use files that must survive. Remove it with `git worktree remove <path>` without `--force`; never delete a worktree directory with `rm`.
