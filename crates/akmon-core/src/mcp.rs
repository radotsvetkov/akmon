//! Model Context Protocol (MCP) configuration types shared across crates.

use serde::{Deserialize, Serialize};

/// Configuration for a single MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    /// Human-readable name for this server shown in logs.
    pub name: String,
    /// Base URL of the MCP server (for example `http://localhost:3000`).
    pub url: String,
    /// Optional description shown in tool registry metadata.
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_server_config_serializes_and_deserializes() {
        let c = McpServerConfig {
            name: "local".into(),
            url: "http://localhost:3000".into(),
            description: "test server".into(),
        };
        let j = match serde_json::to_string(&c) {
            Ok(s) => s,
            Err(e) => panic!("serialize: {e}"),
        };
        let d: McpServerConfig = match serde_json::from_str(&j) {
            Ok(v) => v,
            Err(e) => panic!("deserialize: {e}"),
        };
        assert_eq!(d, c);
    }
}
