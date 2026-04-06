//! HTTP GET helper with conservative SSRF checks (no DNS resolution).

use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

use akmon_core::Permission;
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use url::{Host, Url};

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};

fn web_fetch_permissions() -> &'static [Permission] {
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| [Permission::NetworkFetch { url: String::new() }])
        .as_slice()
}

/// Validates a URL for safe outbound HTTP(S) fetches (blocks loopback, private, link-local, and cloud metadata targets).
///
/// Parsing uses the `url` crate only; hostnames are **not** resolved to addresses (DNS rebinding is out of scope).
pub fn validate_url(url_str: &str) -> Result<Url, ToolOutput> {
    let u = match Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => {
            return Err(ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: "invalid URL".into(),
            });
        }
    };

    let scheme = u.scheme();
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(ToolOutput::Error {
            code: ToolErrorCode::InvalidArgs,
            message: format!("unsupported scheme: {scheme}"),
        });
    }

    match u.host() {
        None => {
            return Err(ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: "URL has no host".into(),
            });
        }
        Some(Host::Ipv4(addr)) => {
            block_if_forbidden_ip(IpAddr::V4(addr))?;
        }
        Some(Host::Ipv6(addr)) => {
            block_if_forbidden_ip(IpAddr::V6(addr))?;
        }
        Some(Host::Domain(host)) => {
            if host.eq_ignore_ascii_case("metadata.google.internal") {
                return Err(ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "cloud metadata endpoint blocked".into(),
                });
            }
            if host.eq_ignore_ascii_case("localhost") {
                return Err(ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "localhost URLs are not permitted".into(),
                });
            }
            if let Ok(ip) = host.parse::<IpAddr>() {
                block_if_forbidden_ip(ip)?;
            }
        }
    }

    Ok(u)
}

fn block_if_forbidden_ip(ip: IpAddr) -> Result<(), ToolOutput> {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            if o == [169, 254, 169, 254] {
                return Err(ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "cloud metadata endpoint blocked".into(),
                });
            }
            if v4.is_loopback() {
                return Err(ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "localhost URLs are not permitted".into(),
                });
            }
            if v4.is_private() {
                return Err(ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "private network URLs are not permitted".into(),
                });
            }
            if v4.is_link_local() {
                return Err(ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message:
                        "link-local URLs are not permitted (potential cloud metadata endpoint)"
                            .into(),
                });
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return Err(ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "localhost URLs are not permitted".into(),
                });
            }
            if v6.is_unicast_link_local() {
                return Err(ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message:
                        "link-local URLs are not permitted (potential cloud metadata endpoint)"
                            .into(),
                });
            }
            if v6.is_unique_local() {
                return Err(ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "private network URLs are not permitted".into(),
                });
            }
        }
    }
    Ok(())
}

/// Fetches public HTTP(S) documents with size and time limits and SSRF checks.
pub struct WebFetchTool {
    max_response_bytes: usize,
    timeout_secs: u64,
    allowed_schemes: Vec<String>,
}

impl WebFetchTool {
    /// Default limits: 512 KiB body, 30 s timeout, schemes `https` and `http`.
    pub fn new() -> Self {
        Self {
            max_response_bytes: 524_288,
            timeout_secs: 30,
            allowed_schemes: vec!["https".into(), "http".into()],
        }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebFetchTool {
    fn scheme_allowed(&self, scheme: &str) -> bool {
        self.allowed_schemes
            .iter()
            .any(|s| s.eq_ignore_ascii_case(scheme))
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch the content of a URL. Use for reading documentation, API references, GitHub issues, and public web pages. Only https and http URLs are supported. Internal network addresses are blocked."
    }

    fn required_permissions(&self) -> &[Permission] {
        web_fetch_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch. Must be https or http. Internal/private IP addresses are blocked."
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: JsonValue, _ctx: &ToolContext) -> ToolOutput {
        let url_str = match args.get("url").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing or empty \"url\" string".into(),
                };
            }
        };

        let parsed = match validate_url(url_str) {
            Ok(u) => u,
            Err(e) => return e,
        };

        let scheme = parsed.scheme();
        if !self.scheme_allowed(scheme) {
            return ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: format!("unsupported scheme: {scheme}"),
            };
        }

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent("akmon/0.1.0")
            .use_rustls_tls()
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("could not build HTTP client: {e}"),
                };
            }
        };

        let mut response = match client.get(parsed.as_str()).send().await {
            Ok(r) => r,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("request failed: {e}"),
                };
            }
        };

        let status = response.status();
        let url_display = parsed.as_str().to_string();
        if status.is_client_error() || status.is_server_error() {
            return ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: format!("HTTP {status}: {url_display}"),
            };
        }

        let mut body: Vec<u8> = Vec::new();
        let mut truncated = false;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    let cap = self.max_response_bytes;
                    let remaining = cap.saturating_sub(body.len());
                    if remaining == 0 {
                        truncated = true;
                        break;
                    }
                    if chunk.len() <= remaining {
                        body.extend_from_slice(&chunk);
                    } else {
                        body.extend_from_slice(&chunk[..remaining]);
                        truncated = true;
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::PermissionDenied,
                        message: format!("failed to read response body: {e}"),
                    };
                }
            }
        }

        let mut text = match String::from_utf8(body) {
            Ok(s) => s,
            Err(_) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::BinaryContent,
                    message: "response body is not valid UTF-8".into(),
                };
            }
        };

        if truncated {
            let kb = self.max_response_bytes.div_ceil(1024);
            text.push_str(&format!("\n[response truncated at {kb}KB]"));
        }

        let payload = serde_json::json!({
            "url": url_display,
            "status": status.as_u16(),
            "content": text,
            "truncated": truncated,
        });

        let content = match serde_json::to_string(&payload) {
            Ok(s) => s,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: format!("failed to serialize result: {e}"),
                };
            }
        };

        ToolOutput::Success { content }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_url_accepts_public_https() {
        let u = validate_url("https://example.com/path?q=1").expect("ok");
        assert_eq!(u.scheme(), "https");
        assert_eq!(u.host_str(), Some("example.com"));
    }

    #[test]
    fn validate_url_rejects_http_localhost() {
        let e = validate_url("http://localhost/foo").expect_err("blocked");
        match e {
            ToolOutput::Error { message, .. } => {
                assert!(message.contains("localhost"), "{message}");
            }
            ToolOutput::Success { .. } => panic!("expected error"),
        }
    }

    #[test]
    fn validate_url_rejects_127() {
        assert!(validate_url("https://127.0.0.1/").is_err());
    }

    #[test]
    fn validate_url_rejects_10_net() {
        assert!(validate_url("http://10.0.0.1/").is_err());
    }

    #[test]
    fn validate_url_rejects_192_168() {
        assert!(validate_url("http://192.168.1.1/").is_err());
    }

    #[test]
    fn validate_url_rejects_172_16() {
        assert!(validate_url("http://172.16.0.1/").is_err());
    }

    #[test]
    fn validate_url_rejects_metadata_ip() {
        let e = validate_url("http://169.254.169.254/").expect_err("blocked");
        match e {
            ToolOutput::Error { message, .. } => assert!(
                message.contains("metadata") || message.contains("cloud"),
                "{message}"
            ),
            ToolOutput::Success { .. } => panic!("expected error"),
        }
    }

    #[test]
    fn validate_url_rejects_metadata_host() {
        let e = validate_url("http://metadata.google.internal/").expect_err("blocked");
        match e {
            ToolOutput::Error { message, .. } => assert!(message.contains("metadata"), "{message}"),
            ToolOutput::Success { .. } => panic!("expected error"),
        }
    }

    #[test]
    fn validate_url_rejects_ftp() {
        let e = validate_url("ftp://example.com/").expect_err("blocked");
        match e {
            ToolOutput::Error { message, .. } => assert!(message.contains("scheme"), "{message}"),
            ToolOutput::Success { .. } => panic!("expected error"),
        }
    }

    #[test]
    fn validate_url_rejects_no_host() {
        // Forms with an empty authority: `url` may reject at parse time (`invalid URL`) or yield `host: None`.
        let candidates = ["https:///", "http:///"];
        let mut matched = false;
        for s in candidates {
            if let Err(ToolOutput::Error { message, .. }) = validate_url(s) {
                if message.contains("host") || message == "invalid URL" {
                    matched = true;
                    break;
                }
                panic!("unexpected message for {s:?}: {message}");
            }
        }
        assert!(
            matched,
            "expected an empty-authority URL to be rejected (no host or invalid URL)"
        );
    }

    #[test]
    fn validate_url_rejects_colon_colon_one() {
        assert!(validate_url("http://[::1]/").is_err());
    }
}
