#![forbid(unsafe_code)]

//! Rust-only guarded adapter for the `dpm` CLI.
//!
//! The protocol lifecycle is represented as a typestate transition. An
//! uninitialized session has no tool methods; initialization consumes it and
//! returns the initialized capability.
//!
//! ```compile_fail
//! use cli_mcp_contract::{Session, Uninitialized};
//! use serde_json::json;
//!
//! let session = Session::<Uninitialized>::new();
//! let _ = session.list_tools(json!(1));
//! ```

use anyhow::{Context, anyhow};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Read;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const PROTOCOL_VERSION: &str = "2025-03-26";
pub const SERVER_NAME: &str = "dpm-mcp";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolName {
    Diff,
    Verify,
    Apply,
}

impl ToolName {
    pub const ALL: [Self; 3] = [Self::Diff, Self::Verify, Self::Apply];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Diff => "dpm_diff",
            Self::Verify => "dpm_verify",
            Self::Apply => "dpm_apply",
        }
    }

    const fn command(self) -> &'static str {
        match self {
            Self::Diff => "diff",
            Self::Verify => "verify",
            Self::Apply => "apply",
        }
    }

    pub fn parse(value: &str) -> Result<Self, InputError> {
        match value {
            "dpm_diff" => Ok(Self::Diff),
            "dpm_verify" => Ok(Self::Verify),
            "dpm_apply" => Ok(Self::Apply),
            _ => Err(InputError::new("unknown tool")),
        }
    }
}

pub fn tool_descriptors() -> Vec<Value> {
    vec![
        json!({
            "name": ToolName::Diff.as_str(),
            "description": "Generate a read-only declarative migration plan.",
            "inputSchema": {
                "type": "object",
                "required": ["source", "target"],
                "properties": {
                    "source": {"type": "string"},
                    "target": {"type": "string"},
                    "shadow": {"type": "string"},
                    "format": {"enum": ["sql", "json"]}
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": ToolName::Verify.as_str(),
            "description": "Replay a plan on a shadow target and prove convergence.",
            "inputSchema": {
                "type": "object",
                "required": ["source", "target", "shadow"],
                "properties": {
                    "source": {"type": "string"},
                    "target": {"type": "string"},
                    "shadow": {"type": "string"}
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": ToolName::Apply.as_str(),
            "description": "Apply a declarative plan only with exact target confirmation.",
            "inputSchema": {
                "type": "object",
                "required": ["source", "target", "shadow", "confirm_target"],
                "properties": {
                    "source": {"type": "string"},
                    "target": {"type": "string"},
                    "shadow": {"type": "string"},
                    "confirm_target": {"type": "string"},
                    "allow_destructive": {"type": "boolean"}
                },
                "additionalProperties": false
            }
        }),
    ]
}

#[derive(Debug, Eq, PartialEq)]
pub struct CommandSpec {
    program: PathBuf,
    args: Vec<OsString>,
}

impl CommandSpec {
    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn args(&self) -> &[OsString] {
        &self.args
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputError {
    message: String,
}

impl InputError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for InputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for InputError {}

#[derive(Debug)]
pub enum CallError {
    Invalid(InputError),
    Timeout(Duration),
    Runtime(anyhow::Error),
}

impl fmt::Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(formatter),
            Self::Timeout(timeout) => write!(
                formatter,
                "dpm command timed out after {} seconds",
                timeout.as_secs()
            ),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl Error for CallError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Invalid(error) => Some(error),
            Self::Runtime(error) => Some(error.as_ref()),
            Self::Timeout(_) => None,
        }
    }
}

impl From<InputError> for CallError {
    fn from(value: InputError) -> Self {
        Self::Invalid(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolResult {
    pub content: Vec<TextContent>,
    #[serde(rename = "isError")]
    pub is_error: bool,
    pub exit_code: i32,
    pub stderr: String,
}

impl ToolResult {
    pub fn text(&self) -> &str {
        self.content
            .first()
            .map(|content| content.text.as_str())
            .unwrap_or("")
    }
}

#[derive(Clone, Debug)]
pub struct DpmAdapter {
    binary: PathBuf,
    timeout: Duration,
}

impl DpmAdapter {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn from_env() -> Self {
        let binary = std::env::var_os("DPM_BIN").unwrap_or_else(|| OsString::from("dpm"));
        Self::new(PathBuf::from(binary))
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn command_for(
        &self,
        tool: ToolName,
        arguments: &Map<String, Value>,
    ) -> Result<CommandSpec, InputError> {
        let source = required_string(arguments, "source")?;
        let target = required_string(arguments, "target")?;
        let mut args = vec![
            OsString::from(tool.command()),
            OsString::from("--source"),
            OsString::from(source),
            OsString::from("--target"),
            OsString::from(target),
        ];

        if let Some(shadow) = optional_string(arguments, "shadow")? {
            args.push(OsString::from("--shadow"));
            args.push(OsString::from(shadow));
        }

        match tool {
            ToolName::Diff => {
                let format = optional_string(arguments, "format")?.unwrap_or("sql");
                if !matches!(format, "sql" | "json") {
                    return Err(InputError::new("format must be sql or json"));
                }
                args.push(OsString::from("--format"));
                args.push(OsString::from(format));
            }
            ToolName::Verify => {
                if optional_string(arguments, "shadow")?.is_none() {
                    return Err(InputError::new("shadow is a required string"));
                }
            }
            ToolName::Apply => {
                if optional_string(arguments, "shadow")?.is_none() {
                    return Err(InputError::new("shadow is a required string"));
                }
                let confirmation = required_string(arguments, "confirm_target")?;
                if confirmation != target {
                    return Err(InputError::new(
                        "confirm_target must exactly equal target",
                    ));
                }
                args.push(OsString::from("--yes"));
                if arguments.get("allow_destructive") == Some(&Value::Bool(true)) {
                    args.push(OsString::from("--allow-destructive"));
                }
            }
        }

        Ok(CommandSpec {
            program: self.binary.clone(),
            args,
        })
    }

    pub fn call_tool(
        &self,
        name: &str,
        arguments: &Map<String, Value>,
    ) -> Result<ToolResult, CallError> {
        let tool = ToolName::parse(name)?;
        let command = self.command_for(tool, arguments)?;
        self.execute(command)
    }

    fn execute(&self, specification: CommandSpec) -> Result<ToolResult, CallError> {
        let mut child = Command::new(&specification.program)
            .args(&specification.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                CallError::Runtime(anyhow!(error).context(format!(
                    "spawning {}",
                    specification.program.display()
                )))
            })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CallError::Runtime(anyhow!("dpm child has no stdout")))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CallError::Runtime(anyhow!("dpm child has no stderr")))?;
        let stdout_reader = read_in_background(stdout);
        let stderr_reader = read_in_background(stderr);
        let deadline = Instant::now() + self.timeout;
        let mut timed_out = false;

        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().map_err(|error| {
                        CallError::Runtime(anyhow!(error).context("waiting after dpm timeout"))
                    })?;
                }
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(error) => {
                    return Err(CallError::Runtime(
                        anyhow!(error).context("polling dpm process"),
                    ));
                }
            }
        };

        let stdout = join_reader(stdout_reader, "stdout").map_err(CallError::Runtime)?;
        let stderr = join_reader(stderr_reader, "stderr").map_err(CallError::Runtime)?;
        if timed_out {
            return Err(CallError::Timeout(self.timeout));
        }

        Ok(ToolResult {
            content: vec![TextContent {
                kind: "text",
                text: String::from_utf8_lossy(&stdout).into_owned(),
            }],
            is_error: !status.success(),
            exit_code: status.code().unwrap_or(-1),
            stderr: tail_text(&String::from_utf8_lossy(&stderr), 8_192),
        })
    }
}

fn required_string<'a>(
    arguments: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str, InputError> {
    optional_string(arguments, name)?.ok_or_else(|| {
        InputError::new(format!("{name} is a required string"))
    })
}

fn optional_string<'a>(
    arguments: &'a Map<String, Value>,
    name: &str,
) -> Result<Option<&'a str>, InputError> {
    match arguments.get(name) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| InputError::new(format!("{name} must be a string"))),
    }
}

fn read_in_background(
    mut reader: impl Read + Send + 'static,
) -> JoinHandle<std::io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

fn join_reader(
    handle: JoinHandle<std::io::Result<Vec<u8>>>,
    stream: &str,
) -> anyhow::Result<Vec<u8>> {
    handle
        .join()
        .map_err(|_| anyhow!("{stream} reader thread panicked"))?
        .with_context(|| format!("reading dpm {stream}"))
}

fn tail_text(text: &str, maximum: usize) -> String {
    let mut tail = text.chars().rev().take(maximum).collect::<Vec<_>>();
    tail.reverse();
    tail.into_iter().collect()
}

#[derive(Debug)]
pub struct Uninitialized;

#[derive(Debug)]
pub struct Initialized;

#[derive(Debug)]
pub struct Session<State> {
    state: PhantomData<State>,
}

impl Session<Uninitialized> {
    pub const fn new() -> Self {
        Self { state: PhantomData }
    }

    pub fn initialize(self, identifier: Value) -> (Session<Initialized>, Value) {
        let response = rpc_result(
            identifier,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": SERVER_NAME,
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        );
        (
            Session {
                state: PhantomData,
            },
            response,
        )
    }
}

impl Default for Session<Uninitialized> {
    fn default() -> Self {
        Self::new()
    }
}

impl Session<Initialized> {
    pub fn list_tools(&self, identifier: Value) -> Value {
        rpc_result(identifier, json!({"tools": tool_descriptors()}))
    }

    fn handle_message(&self, message: &Value, adapter: &DpmAdapter) -> Option<Value> {
        let identifier = message.get("id").cloned().unwrap_or(Value::Null);
        match message.get("method").and_then(Value::as_str) {
            Some("notifications/initialized") => None,
            Some("tools/list") => Some(self.list_tools(identifier)),
            Some("tools/call") => Some(handle_tool_call(identifier, message, adapter)),
            _ => Some(rpc_error(identifier, -32601, "method not found")),
        }
    }
}

#[derive(Debug)]
pub enum AnySession {
    Uninitialized(Session<Uninitialized>),
    Initialized(Session<Initialized>),
}

impl Default for AnySession {
    fn default() -> Self {
        Self::Uninitialized(Session::new())
    }
}

#[derive(Debug)]
pub struct Dispatch {
    pub session: AnySession,
    pub response: Option<Value>,
}

impl AnySession {
    pub fn dispatch(self, message: &Value, adapter: &DpmAdapter) -> Dispatch {
        match self {
            Self::Uninitialized(session) => {
                let identifier = message.get("id").cloned().unwrap_or(Value::Null);
                if message.get("method").and_then(Value::as_str) == Some("initialize") {
                    let (session, response) = session.initialize(identifier);
                    Dispatch {
                        session: Self::Initialized(session),
                        response: Some(response),
                    }
                } else {
                    Dispatch {
                        session: Self::Uninitialized(session),
                        response: Some(rpc_error(
                            identifier,
                            -32002,
                            "server is not initialized",
                        )),
                    }
                }
            }
            Self::Initialized(session) => {
                let response = session.handle_message(message, adapter);
                Dispatch {
                    session: Self::Initialized(session),
                    response,
                }
            }
        }
    }
}

fn handle_tool_call(identifier: Value, message: &Value, adapter: &DpmAdapter) -> Value {
    let Some(parameters) = message.get("params").and_then(Value::as_object) else {
        return rpc_error(identifier, -32602, "tools/call params must be an object");
    };
    let Some(name) = parameters.get("name").and_then(Value::as_str) else {
        return rpc_error(identifier, -32602, "tool name must be a string");
    };
    let arguments = parameters
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let Some(arguments) = arguments.as_object() else {
        return rpc_error(identifier, -32602, "tool arguments must be an object");
    };

    match adapter.call_tool(name, arguments) {
        Ok(result) => rpc_result(identifier, json!(result)),
        Err(CallError::Invalid(error)) => rpc_error(identifier, -32602, error.message()),
        Err(CallError::Timeout(_)) => rpc_error(identifier, -32001, "dpm command timed out"),
        Err(CallError::Runtime(error)) => rpc_error(
            identifier,
            -32000,
            &format!("dpm command failed to start or complete: {error}"),
        ),
    }
}

fn rpc_result(identifier: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": identifier, "result": result})
}

pub fn rpc_error(identifier: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": identifier,
        "error": {"code": code, "message": message}
    })
}

pub fn os_strings(values: &[OsString]) -> Vec<&OsStr> {
    values.iter().map(OsString::as_os_str).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().expect("test JSON object").clone()
    }

    #[test]
    fn tools_are_exact_and_guarded() {
        assert_eq!(
            ToolName::ALL.map(ToolName::as_str),
            ["dpm_diff", "dpm_verify", "dpm_apply"]
        );
        let descriptors = tool_descriptors();
        assert_eq!(descriptors.len(), 3);
        assert!(descriptors.iter().all(|tool| {
            tool.pointer("/inputSchema/additionalProperties") == Some(&Value::Bool(false))
        }));
    }

    #[test]
    fn apply_requires_exact_target_confirmation() {
        let adapter = DpmAdapter::new("dpm");
        let arguments = object(json!({
            "source": "source.sql",
            "target": "postgres://db/target",
            "shadow": "postgres://db/admin",
            "confirm_target": "postgres://db/other"
        }));
        let error = adapter
            .command_for(ToolName::Apply, &arguments)
            .expect_err("mismatched confirmation must fail");
        assert_eq!(error.message(), "confirm_target must exactly equal target");
    }

    #[test]
    fn arguments_are_not_shell_interpolated() {
        let adapter = DpmAdapter::new("/opt/dpm");
        let target = "postgres://db/target;touch /tmp/should-not-exist";
        let arguments = object(json!({
            "source": "source.sql",
            "target": target,
            "shadow": "postgres://db/admin",
            "format": "json"
        }));
        let command = adapter
            .command_for(ToolName::Diff, &arguments)
            .expect("valid diff command");
        assert_eq!(command.program(), Path::new("/opt/dpm"));
        assert!(command.args().iter().any(|argument| argument == target));
        assert_ne!(command.program(), Path::new("sh"));
    }

    #[test]
    fn protocol_rejects_preinitialization_calls() {
        let adapter = DpmAdapter::new("dpm");
        let dispatch = AnySession::default().dispatch(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            &adapter,
        );
        assert_eq!(
            dispatch.response.as_ref().and_then(|value| value.pointer("/error/code")),
            Some(&json!(-32002))
        );
        assert!(matches!(dispatch.session, AnySession::Uninitialized(_)));
    }

    #[test]
    fn initialize_consumes_the_uninitialized_state() {
        let adapter = DpmAdapter::new("dpm");
        let dispatch = AnySession::default().dispatch(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
            &adapter,
        );
        assert_eq!(
            dispatch
                .response
                .as_ref()
                .and_then(|value| value.pointer("/result/protocolVersion")),
            Some(&json!(PROTOCOL_VERSION))
        );
        let dispatch = dispatch.session.dispatch(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            &adapter,
        );
        assert_eq!(
            dispatch
                .response
                .as_ref()
                .and_then(|value| value.pointer("/result/tools"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(3)
        );
    }

    #[test]
    fn diagnostic_tail_is_bounded() {
        assert_eq!(tail_text("abcdef", 3), "def");
        assert_eq!(tail_text("abc", 10), "abc");
    }
}
