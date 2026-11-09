//! JSON-RPC 2.0 envelope, error kinds and the line codec (one message per `\n`-terminated line).

use std::io::BufRead;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
/// Application-level refusal; `data.kind` says which.
pub const REFUSED: i64 = -32000;

/// Longest accepted message in bytes, excluding the newline.
pub const MAX_LINE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Request {
    pub jsonrpc: String,
    /// Absent for notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Exactly one of `result` and `error` is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Response {
        Response {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, error: RpcError) -> Response {
        Response {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

impl RpcError {
    /// An application refusal (code `-32000`) with a stable `kind`.
    pub fn refused(kind: ErrorKind, message: impl Into<String>) -> RpcError {
        RpcError {
            code: REFUSED,
            message: message.into(),
            data: Some(ErrorData::new(kind)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorData {
    pub kind: ErrorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_rev: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<u32>,
}

impl ErrorData {
    pub fn new(kind: ErrorKind) -> ErrorData {
        ErrorData {
            kind,
            uid: None,
            current_rev: None,
            daemon: None,
            protocol: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Blocked,
    Stale,
    Stuck,
    NotFound,
    Invalid,
    Unavailable,
    Protocol,
    WrongInstance,
    Internal,
}

#[derive(Debug, thiserror::Error)]
pub enum LineError {
    #[error("message longer than {MAX_LINE} bytes")]
    TooLong,
    /// The line was complete but is not UTF-8.
    #[error("message is not valid UTF-8")]
    InvalidUtf8,
    #[error("read failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Reads one line, without its `\n` (and a `\r` before it). `None` at end of input; a final
/// line without a newline is returned as is. Never buffers more than [`MAX_LINE`] bytes: on
/// overflow it returns `TooLong` and the caller should close the connection.
pub fn read_line(r: &mut impl BufRead) -> Result<Option<String>, LineError> {
    let mut line: Vec<u8> = Vec::new();
    let mut saw_any = false;
    loop {
        let buf = match r.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        if buf.is_empty() {
            if !saw_any {
                return Ok(None);
            }
            break;
        }
        saw_any = true;
        match buf.iter().position(|&b| b == b'\n') {
            Some(i) => {
                if line.len() + i > MAX_LINE {
                    return Err(LineError::TooLong);
                }
                line.extend_from_slice(&buf[..i]);
                r.consume(i + 1);
                break;
            }
            None => {
                let n = buf.len();
                if line.len() + n > MAX_LINE {
                    return Err(LineError::TooLong);
                }
                line.extend_from_slice(buf);
                r.consume(n);
            }
        }
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|_| LineError::InvalidUtf8)
}
