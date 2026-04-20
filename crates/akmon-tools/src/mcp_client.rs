//! MCP (Model Context Protocol) HTTP client: `tools/list` discovery and `tools/call` execution.

use std::time::Duration;

use akmon_core::{McpServerConfig, Permission};
use async_trait::async_trait;
use serde_json::{Value as JsonValue, json};

use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};
use crate::{McpPolicyContext, Tool};

/// One tool entry decoded from a `tools/list` JSON-RPC result (before building an [`McpTool`]).
#[derive(Debug, Clone)]
pub(crate) struct McpToolSpec {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: JsonValue,
}

/// Parses a JSON-RPC 2.0 `tools/list` response body into tool specs.
///
/// `value` is the full envelope (`jsonrpc`, `id`, `result` or `error`).
pub fn parse_tools_list_envelope(value: &JsonValue) -> Result<Vec<McpToolSpec>, String> {
    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("MCP tools/list error");
        return Err(msg.to_string());
    }
    let result = value
        .get("result")
        .ok_or_else(|| "missing result in tools/list response".to_string())?;
    let tools = result
        .get("tools")
        .and_then(|t| t.as_array())
        .ok_or_else(|| "missing or invalid tools array".to_string())?;
    let mut out = Vec::new();
    for t in tools {
        let name = t
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or_else(|| "tool entry missing name".to_string())?
            .to_string();
        let description = t
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string();
        let input_schema = t.get("inputSchema").cloned().unwrap_or_else(|| json!({}));
        out.push(McpToolSpec {
            name,
            description,
            input_schema,
        });
    }
    Ok(out)
}

/// Parses a JSON-RPC 2.0 `tools/call` response into joined `text` content lines.
///
/// `value` is the full envelope. On RPC `error`, returns `Err` with `error.message`.
pub fn parse_tool_call_envelope(value: &JsonValue) -> Result<String, String> {
    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("MCP error");
        return Err(msg.to_string());
    }
    let result = value
        .get("result")
        .ok_or_else(|| "missing result in tools/call response".to_string())?;
    let Some(content) = result.get("content") else {
        return Err("unexpected MCP response format".to_string());
    };
    let Some(arr) = content.as_array() else {
        return Err("unexpected MCP response format".to_string());
    };
    let mut parts = Vec::new();
    for item in arr {
        if item.get("type").and_then(|t| t.as_str()) != Some("text") {
            continue;
        }
        if let Some(text) = item.get("text").and_then(|x| x.as_str()) {
            parts.push(text);
        }
    }
    if parts.is_empty() && !arr.is_empty() {
        return Err("unexpected MCP response format".to_string());
    }
    Ok(parts.join("\n"))
}

/// POSTs `tools/list` to `server.url` and returns one [`McpTool`] per advertised tool.
pub async fn discover_mcp_tools(server: &McpServerConfig) -> Result<Vec<McpTool>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("MCP client build failed: {e}"))?;
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });
    let resp = client
        .post(&server.url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("MCP tools/list request failed: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("MCP tools/list body read failed: {e}"))?;
    if !status.is_success() {
        return Err(format!("MCP tools/list HTTP {status}: {text}"));
    }
    let value: JsonValue =
        serde_json::from_str(&text).map_err(|e| format!("MCP tools/list invalid JSON: {e}"))?;
    let specs = parse_tools_list_envelope(&value)?;
    Ok(specs
        .into_iter()
        .map(|s| {
            McpTool::new(
                server.name.clone(),
                s.name,
                s.description,
                s.input_schema,
                server.url.clone(),
            )
        })
        .collect())
}

/// Proxy for one remote MCP tool (invokes `tools/call` over JSON-RPC HTTP).
pub struct McpTool {
    server_name: String,
    remote_tool_name: String,
    tool_name: String,
    tool_description: String,
    input_schema: JsonValue,
    server_url: String,
    http_client: reqwest::Client,
    network_permissions: Vec<Permission>,
}

impl McpTool {
    /// Builds a proxy; uses a default [`reqwest::Client`] (per-request timeout is set in [`Tool::execute`]).
    pub fn new(
        server_name: String,
        tool_name: String,
        tool_description: String,
        input_schema: JsonValue,
        server_url: String,
    ) -> Self {
        let network_permissions = vec![Permission::NetworkFetch {
            url: server_url.clone(),
        }];
        Self {
            server_name,
            remote_tool_name: tool_name.clone(),
            tool_name,
            tool_description,
            input_schema,
            server_url,
            http_client: reqwest::Client::new(),
            network_permissions,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn required_permissions(&self) -> &[Permission] {
        self.network_permissions.as_slice()
    }

    fn parameters_schema(&self) -> JsonValue {
        self.input_schema.clone()
    }

    async fn execute(&self, args: JsonValue, _ctx: &ToolContext) -> ToolOutput {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": &self.remote_tool_name,
                "arguments": args,
            }
        });
        let resp = match self
            .http_client
            .post(&self.server_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(30))
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: format!("MCP tools/call request failed: {e}"),
                };
            }
        };
        let status = resp.status();
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: format!("MCP tools/call read body failed: {e}"),
                };
            }
        };
        if !status.is_success() {
            return ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: format!("MCP tools/call HTTP {status}: {text}"),
            };
        }
        let value: JsonValue = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: format!("MCP tools/call invalid JSON: {e}"),
                };
            }
        };
        match parse_tool_call_envelope(&value) {
            Ok(s) => ToolOutput::Success { content: s },
            Err(msg) => ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: msg,
            },
        }
    }

    fn mcp_policy_context(&self) -> Option<McpPolicyContext> {
        Some(McpPolicyContext {
            server: self.server_name.clone(),
            tool: self.remote_tool_name.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_tool_new_sets_name_and_description() {
        let t = McpTool::new(
            "srv".into(),
            "db_query".into(),
            "Run a query".into(),
            json!({"type": "object"}),
            "http://127.0.0.1:9".into(),
        );
        assert_eq!(t.name(), "db_query");
        assert_eq!(t.description(), "Run a query");
    }

    #[test]
    fn mcp_tool_parameters_schema_matches_input() {
        let schema = json!({"type": "object", "properties": {"q": {}}});
        let t = McpTool::new(
            "srv".into(),
            "x".into(),
            "d".into(),
            schema.clone(),
            "http://a".into(),
        );
        assert_eq!(t.parameters_schema(), schema);
    }

    #[test]
    fn mcp_tool_exposes_policy_context() {
        let t = McpTool::new(
            "github".into(),
            "search_issues".into(),
            "d".into(),
            json!({}),
            "http://a".into(),
        );
        let ctx = t.mcp_policy_context().expect("mcp context");
        assert_eq!(ctx.server, "github");
        assert_eq!(ctx.tool, "search_issues");
    }

    #[test]
    fn parse_tools_list_valid_response() {
        let v = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "tools": [
                    {
                        "name": "alpha",
                        "description": "first tool",
                        "inputSchema": {"type": "object", "properties": {"x": {}}}
                    },
                    {
                        "name": "beta"
                    }
                ]
            }
        });
        let specs = match parse_tools_list_envelope(&v) {
            Ok(s) => s,
            Err(e) => panic!("parse: {e}"),
        };
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "alpha");
        assert_eq!(specs[0].description, "first tool");
        assert!(specs[0].input_schema.get("properties").is_some());
        assert_eq!(specs[1].name, "beta");
        assert_eq!(specs[1].description, "");
        assert_eq!(specs[1].input_schema, json!({}));
    }

    #[test]
    fn parse_tools_list_malformed_missing_tools() {
        let v = json!({"jsonrpc": "2.0", "result": {}});
        let r = parse_tools_list_envelope(&v);
        assert!(r.is_err(), "expected error");
        let msg = match r {
            Err(m) => m,
            Ok(_) => panic!("expected Err"),
        };
        assert!(msg.contains("tools"), "{msg}");
    }

    #[test]
    fn parse_tool_call_joins_text_blocks() {
        let v = json!({
            "result": {
                "content": [
                    {"type": "text", "text": "a"},
                    {"type": "text", "text": "b"}
                ]
            }
        });
        let s = match parse_tool_call_envelope(&v) {
            Ok(x) => x,
            Err(e) => panic!("{e}"),
        };
        assert_eq!(s, "a\nb");
    }

    #[test]
    fn parse_tool_call_rpc_error() {
        let v = json!({"error": {"message": "no such tool"}});
        let r = parse_tool_call_envelope(&v);
        assert!(r.is_err());
        assert_eq!(r, Err("no such tool".into()));
    }

    #[test]
    fn parse_tools_list_rpc_error_envelope() {
        let v = json!({"error": {"message": "unauthorized"}});
        match parse_tools_list_envelope(&v) {
            Err(m) => assert_eq!(m, "unauthorized"),
            Ok(_) => panic!("expected Err"),
        }
    }
}
