# cli-mcp-contract

Rust certification for CLI exit codes, flags-to-environment behavior, JSON plans, and a guarded JSON-RPC/MCP adapter.

This repository is part of the isolated `declarative-migrations-test` fleet. It pins the production implementation as a Git submodule at `declarative-migrations/declarative-postgres-migrate.rs@a5e868acc0206fa9c3e91b5e36e0b1b111805885` and exercises a disposable PostgreSQL instance in GitHub Actions.

The pinned product head stacks the linear migration typestate/advisory-lease layer from product PR #30 with the typed plan-resource borrow checker and exact certificates from product PR #31.

## Rust assurance layers

- `cli_mcp_contract::Session<Uninitialized>` must be consumed into `Session<Initialized>` before tools are available; a compile-fail doctest keeps that negative contract executable.
- `DpmAdapter` constructs `std::process::Command` arguments directly and never delegates untrusted values to a shell.
- `dpm_apply` requires `confirm_target` to exactly match `target` before `--yes` can be emitted.
- child stdout and stderr are drained concurrently while a bounded parent lifecycle owns and terminates the process.
- `repository-check` validates the immutable gitlink/manifest pin, required files, conflict markers, credential-shaped content, and the absence of tracked Python.
- `contract-certify` creates and cleans a real PostgreSQL database, checks CLI exit/environment contracts, then exercises the compiled adapter over stdio with a real `dpm_diff` and a rejected mismatched apply confirmation.

## Local contract

```bash
git submodule update --init --recursive
cargo test --locked --all-targets
cargo test --locked --doc
cargo run --locked --bin repository-check
scripts/build-dpm.sh
```

With PostgreSQL and `psql` available:

```bash
cargo build --locked --bins
DPM_BIN="$(scripts/build-dpm.sh)" \
DPM_ADAPTER_BIN="$PWD/target/debug/dpm-mcp-adapter" \
PRODUCT_SHA=a5e868acc0206fa9c3e91b5e36e0b1b111805885 \
POSTGRES_ADMIN_URL=postgres://postgres@localhost:5432/postgres \
cargo run --locked --bin contract-certify
```

Every behavior change must add a regression, preserve exact dependency pinning, avoid credentials in source or logs, and land through a pull request.

## Test-org harness metadata

Recorded by the `zed-pkg-test/test-org-fleet` bootstrapper (the generated harness under `scripts/`, `tests/` and `pyproject.toml`); the certification lane above remains the source of truth.

- **Readiness:** `ready`
- **Primary dependency strategy:** `matrix`
- **Scheduled cadence:** `23 4 * * 2,5` UTC
- **Live infrastructure:** None for deterministic pull-request checks.

Acceptance objectives:

1. Verify CLI/MCP plan parity, dry-run, approval, apply, rollback, and destructive-operation safeguards across the supported happy-path states and canonical fixtures.
2. Verify CLI/MCP plan parity, dry-run, approval, apply, rollback, and destructive-operation safeguards under retries, interruption, concurrency, offline operation, or partial failure.
3. Verify CLI/MCP plan parity, dry-run, approval, apply, rollback, and destructive-operation safeguards preserves authorization, idempotency, integrity, observability, and actionable failure classification.
