#![forbid(unsafe_code)]

use anyhow::{Context, Result, bail};
use cli_mcp_contract::PROTOCOL_VERSION;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[test]
fn stdio_requires_initialization_then_lists_exact_tools() -> Result<()> {
    let mut process = AdapterProcess::spawn()?;

    let rejected = process.request(1, "tools/list", json!({}))?;
    assert_eq!(
        rejected.pointer("/error/code").and_then(Value::as_i64),
        Some(-32002)
    );

    let initialize = process.request(2, "initialize", json!({}))?;
    assert_eq!(
        initialize
            .pointer("/result/protocolVersion")
            .and_then(Value::as_str),
        Some(PROTOCOL_VERSION)
    );
    process.notify("notifications/initialized")?;

    let tools = process.request(3, "tools/list", json!({}))?;
    let names = tools
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .context("tools/list result is not an array")?
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(names, ["dpm_diff", "dpm_verify", "dpm_apply"]);
    Ok(())
}

struct AdapterProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl AdapterProcess {
    fn spawn() -> Result<Self> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dpm-mcp-adapter"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("spawning the Rust MCP adapter")?;
        let stdin = child.stdin.take().context("adapter has no stdin")?;
        let stdout = child.stdout.take().context("adapter has no stdout")?;
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
        if self.stdout.read_line(&mut line)? == 0 {
            bail!("adapter closed stdout before responding to {method}");
        }
        serde_json::from_str(&line).with_context(|| format!("invalid adapter JSON: {line:?}"))
    }

    fn send(&mut self, message: &Value) -> Result<()> {
        serde_json::to_writer(&mut self.stdin, message)?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        Ok(())
    }
}

impl Drop for AdapterProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
