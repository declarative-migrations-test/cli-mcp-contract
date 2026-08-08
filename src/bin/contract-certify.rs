#![forbid(unsafe_code)]

use anyhow::{Context, Result, bail};
use cli_mcp_contract::PROTOCOL_VERSION;
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};

const EXPECTED_PRODUCT_SHA: &str = "a5e868acc0206fa9c3e91b5e36e0b1b111805885";
const DATABASE_NAME: &str = "dm_cli_mcp_contract";

fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let artifacts = root.join("artifacts");
    fs::create_dir_all(&artifacts)
        .with_context(|| format!("creating {}", artifacts.display()))?;

    let dpm = required_path("DPM_BIN")?;
    let adapter = required_path("DPM_ADAPTER_BIN")?;
    let product_sha = env::var("PRODUCT_SHA").context("PRODUCT_SHA is required")?;
    if product_sha != EXPECTED_PRODUCT_SHA {
        bail!(
            "product SHA drifted: expected {EXPECTED_PRODUCT_SHA}, observed {product_sha}"
        );
    }
    let admin = env::var("POSTGRES_ADMIN_URL")
        .unwrap_or_else(|_| "postgres://postgres@localhost:5432/postgres".to_owned());
    let target = database_url(&admin, DATABASE_NAME)?;
    let cleanup = DatabaseCleanup {
        admin: admin.clone(),
        database: DATABASE_NAME.to_owned(),
    };
    cleanup.drop_now();
    psql(&admin, &format!("CREATE DATABASE {DATABASE_NAME}"))?;

    let help = run_checked(
        Command::new(&dpm).arg("help"),
        "reading dpm help",
    )?;
    let help = String::from_utf8_lossy(&help.stdout);
    if !help.contains("--allow-destructive-ops") {
        bail!("dpm help omitted --allow-destructive-ops");
    }

    let fixture_v1 = root.join("fixtures/v1.sql");
    let fixture_v2 = root.join("fixtures/v2.sql");
    run_checked(
        Command::new(&dpm)
            .arg("apply")
            .args(["--source-sql"])
            .arg(&fixture_v1)
            .args(["--target", &target, "--shadow", &admin, "--yes"]),
        "applying the v1 fixture",
    )?;

    let drift = run(
        Command::new(&dpm)
            .arg("diff")
            .args(["--source-sql"])
            .arg(&fixture_v2)
            .args([
                "--target",
                &target,
                "--shadow",
                &admin,
                "--fail-on-diff",
            ]),
        "checking the fail-on-diff exit code",
    )?;
    if drift.status.code() != Some(2) {
        bail!(
            "dpm diff --fail-on-diff returned {:?}, expected 2: {}",
            drift.status.code(),
            String::from_utf8_lossy(&drift.stderr).trim()
        );
    }

    let plan = run_checked(
        Command::new(&dpm)
            .arg("diff")
            .env("SOURCE_SQL_FILE", &fixture_v2)
            .env("TARGET_DATABASE_URL", &target)
            .env("SHADOW_DATABASE_URL", &admin)
            .env("DPM_FORMAT", "json"),
        "generating the flags-to-environment JSON plan",
    )?;
    let plan_json: Value = serde_json::from_slice(&plan.stdout)
        .context("dpm environment-driven diff did not return JSON")?;
    if !plan_json.is_object() {
        bail!("dpm JSON plan is not an object");
    }
    fs::write(
        artifacts.join("plan.json"),
        serde_json::to_vec_pretty(&plan_json).context("encoding plan evidence")?,
    )
    .context("writing plan evidence")?;

    let mut mcp = McpProcess::spawn(&adapter, &dpm)?;
    let initialize = mcp.request(
        1,
        "initialize",
        json!({"protocolVersion": PROTOCOL_VERSION, "capabilities": {}}),
    )?;
    if initialize.pointer("/result/protocolVersion").and_then(Value::as_str)
        != Some(PROTOCOL_VERSION)
    {
        bail!("MCP initialize returned the wrong protocol version: {initialize}");
    }
    mcp.notify("notifications/initialized")?;

    let tools = mcp.request(2, "tools/list", json!({}))?;
    let names = tools
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .context("tools/list did not return an array")?
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    if names != ["dpm_diff", "dpm_verify", "dpm_apply"] {
        bail!("guarded MCP tools drifted: {names:?}");
    }

    let source = fixture_v2.to_string_lossy();
    let diff = mcp.request(
        3,
        "tools/call",
        json!({
            "name": "dpm_diff",
            "arguments": {
                "source": source,
                "target": target,
                "shadow": admin,
                "format": "json"
            }
        }),
    )?;
    if diff.pointer("/result/isError").and_then(Value::as_bool) != Some(false)
        || diff.pointer("/result/exit_code").and_then(Value::as_i64) != Some(0)
    {
        bail!("real MCP diff failed: {diff}");
    }
    let diff_text = diff
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
        .context("real MCP diff omitted text content")?;
    let diff_plan: Value = serde_json::from_str(diff_text)
        .context("real MCP diff content was not a JSON plan")?;
    if !diff_plan.is_object() {
        bail!("real MCP diff plan is not an object");
    }

    let rejected_apply = mcp.request(
        4,
        "tools/call",
        json!({
            "name": "dpm_apply",
            "arguments": {
                "source": source,
                "target": target,
                "shadow": admin,
                "confirm_target": "postgres://postgres@localhost:5432/not-the-target"
            }
        }),
    )?;
    if rejected_apply.pointer("/error/code").and_then(Value::as_i64) != Some(-32602)
        || !rejected_apply
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("exactly equal"))
    {
        bail!("MCP apply confirmation guard did not reject the request: {rejected_apply}");
    }

    let summary = json!({
        "schema_version": 1,
        "product_sha": EXPECTED_PRODUCT_SHA,
        "checks": {
            "help_contract": "passed",
            "fail_on_diff_exit_2": "passed",
            "flags_to_environment_json": "passed",
            "mcp_initialize": "passed",
            "mcp_exact_tool_list": "passed",
            "mcp_real_diff": "passed",
            "mcp_apply_confirmation": "passed"
        }
    });
    fs::write(
        artifacts.join("summary.json"),
        serde_json::to_vec_pretty(&summary).context("encoding summary evidence")?,
    )
    .context("writing summary evidence")?;
    println!("CLI and Rust MCP contract certification passed");
    drop(cleanup);
    Ok(())
}

fn required_path(name: &str) -> Result<PathBuf> {
    let path = PathBuf::from(env::var_os(name).with_context(|| format!("{name} is required"))?);
    if !path.is_file() {
        bail!("{name} does not point to a regular file: {}", path.display());
    }
    Ok(path)
}

fn database_url(admin: &str, database: &str) -> Result<String> {
    let (base, query) = admin
        .split_once('?')
        .map_or((admin, None), |(base, query)| (base, Some(query)));
    let (prefix, _) = base
        .rsplit_once('/')
        .context("POSTGRES_ADMIN_URL must include a database path")?;
    Ok(match query {
        Some(query) => format!("{prefix}/{database}?{query}"),
        None => format!("{prefix}/{database}"),
    })
}

fn psql(admin: &str, sql: &str) -> Result<()> {
    run_checked(
        Command::new("psql")
            .arg(admin)
            .args(["-v", "ON_ERROR_STOP=1", "-c", sql]),
        "executing PostgreSQL administration command",
    )?;
    Ok(())
}

fn run(command: &mut Command, description: &str) -> Result<Output> {
    command.output().with_context(|| description.to_owned())
}

fn run_checked(command: &mut Command, description: &str) -> Result<Output> {
    let output = run(command, description)?;
    if output.status.success() {
        Ok(output)
    } else {
        bail!(
            "{description} failed with {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

struct DatabaseCleanup {
    admin: String,
    database: String,
}

impl DatabaseCleanup {
    fn drop_now(&self) {
        let _ = Command::new("psql")
            .arg(&self.admin)
            .args([
                "-v",
                "ON_ERROR_STOP=1",
                "-c",
                &format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.database),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

impl Drop for DatabaseCleanup {
    fn drop(&mut self) {
        self.drop_now();
    }
}

struct McpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpProcess {
    fn spawn(adapter: &Path, dpm: &Path) -> Result<Self> {
        let mut child = Command::new(adapter)
            .env("DPM_BIN", dpm)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("spawning {}", adapter.display()))?;
        let stdin = child.stdin.take().context("MCP adapter has no stdin")?;
        let stdout = child.stdout.take().context("MCP adapter has no stdout")?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    fn notify(&mut self, method: &str) -> Result<()> {
        self.send(&json!({"jsonrpc": "2.0", "method": method}))
    }

    fn request(&mut self, identifier: u64, method: &str, params: Value) -> Result<Value> {
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": identifier,
            "method": method,
            "params": params
        }))?;
        let mut line = String::new();
        let bytes = self
            .stdout
            .read_line(&mut line)
            .with_context(|| format!("reading response to {method}"))?;
        if bytes == 0 {
            bail!("MCP adapter closed stdout before responding to {method}");
        }
        let response: Value = serde_json::from_str(&line)
            .with_context(|| format!("MCP response was not JSON: {line:?}"))?;
        if response.get("id") != Some(&json!(identifier)) {
            bail!("MCP response id drifted for {method}: {response}");
        }
        Ok(response)
    }

    fn send(&mut self, message: &Value) -> Result<()> {
        serde_json::to_writer(&mut self.stdin, message).context("encoding MCP request")?;
        self.stdin.write_all(b"\n").context("terminating MCP request")?;
        self.stdin.flush().context("flushing MCP request")
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
