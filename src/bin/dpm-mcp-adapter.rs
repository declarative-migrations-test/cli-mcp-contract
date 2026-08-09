#![forbid(unsafe_code)]

use anyhow::{Context, Result};
use cli_mcp_contract::{AnySession, DpmAdapter, rpc_error};
use serde_json::Value;
use std::io::{self, BufRead, Write};

fn main() -> Result<()> {
    let adapter = DpmAdapter::from_env();
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut session = AnySession::default();

    for line in stdin.lock().lines() {
        let line = line.context("reading MCP request")?;
        let message: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(error) => {
                write_response(
                    &mut stdout,
                    &rpc_error(Value::Null, -32700, &format!("invalid JSON: {error}")),
                )?;
                continue;
            }
        };

        let dispatch = session.dispatch(&message, &adapter);
        session = dispatch.session;
        if let Some(response) = dispatch.response {
            write_response(&mut stdout, &response)?;
        }
    }
    Ok(())
}

fn write_response(writer: &mut impl Write, response: &Value) -> Result<()> {
    serde_json::to_writer(&mut *writer, response).context("encoding MCP response")?;
    writer
        .write_all(b"\n")
        .context("terminating MCP response")?;
    writer.flush().context("flushing MCP response")
}
