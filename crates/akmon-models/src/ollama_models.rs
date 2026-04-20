//! Discover models from a running Ollama server (`GET /api/tags`).

use std::time::Duration;

use serde::Deserialize;

/// One entry returned by Ollama's `/api/tags`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaModel {
    /// Model name with optional tag (e.g. `qwen2.5-coder:7b`).
    pub name: String,
    /// Model weight in bytes when the server reports it.
    pub size_bytes: u64,
    /// Optional context-window hint reported by some Ollama versions.
    pub context_window_tokens: Option<u64>,
}

impl OllamaModel {
    /// Human-readable size for picker / errors.
    pub fn display_size(&self) -> String {
        if self.size_bytes > 1_000_000_000 {
            format!("{:.1}GB", self.size_bytes as f64 / 1e9)
        } else {
            format!("{:.0}MB", self.size_bytes as f64 / 1e6)
        }
    }
}

#[derive(Debug, Deserialize)]
struct TagsEnvelope {
    #[serde(default)]
    models: Vec<TagsModel>,
}

#[derive(Debug, Deserialize)]
struct TagsModel {
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    details: Option<TagsModelDetails>,
}

#[derive(Debug, Deserialize)]
struct TagsModelDetails {
    #[serde(default)]
    context_length: Option<u64>,
}

fn parse_tags_json(json: serde_json::Value) -> Vec<OllamaModel> {
    if let Ok(env) = serde_json::from_value::<TagsEnvelope>(json.clone()) {
        return env
            .models
            .into_iter()
            .map(|m| OllamaModel {
                name: m.name,
                size_bytes: m.size,
                context_window_tokens: m.details.and_then(|d| d.context_length),
            })
            .collect();
    }

    json["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    Some(OllamaModel {
                        name: m["name"].as_str()?.to_string(),
                        size_bytes: m["size"].as_u64().unwrap_or(0),
                        context_window_tokens: m["details"]["context_length"].as_u64(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Result of probing `GET {base}/api/tags` (distinguishes down vs empty catalog).
#[derive(Debug, Clone)]
pub struct OllamaProbe {
    /// `true` when the server responded successfully to `/api/tags`.
    pub reachable: bool,
    /// Installed models when [`Self::reachable`] is `true`.
    pub models: Vec<OllamaModel>,
}

/// Best-effort local capability hints derived from model metadata and optional probe data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaCapabilityHint {
    /// Context window used for local reliability hints when explicit model metadata is available.
    pub context_window_tokens_hint: usize,
    /// Whether the model is expected to handle tool/function calls.
    pub likely_tool_call_support: bool,
    /// Adaptive first-token deadline for this model/profile.
    pub first_token_deadline_ms: u64,
    /// Adaptive idle-stream timeout for this model/profile.
    pub idle_stream_timeout_secs: u64,
}

/// Probes Ollama with a 2-second timeout.
pub async fn probe_ollama(base_url: &str) -> OllamaProbe {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => reqwest::Client::new(),
    };

    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));

    let Ok(resp) = client.get(&url).send().await else {
        return OllamaProbe {
            reachable: false,
            models: vec![],
        };
    };

    if !resp.status().is_success() {
        return OllamaProbe {
            reachable: false,
            models: vec![],
        };
    }

    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return OllamaProbe {
            reachable: true,
            models: vec![],
        };
    };

    let models = parse_tags_json(json);
    OllamaProbe {
        reachable: true,
        models,
    }
}

/// Lists models from `GET {base_url}/api/tags`.
///
/// Uses a 2-second timeout. On any failure, returns an empty list.
pub async fn fetch_ollama_models(base_url: &str) -> Vec<OllamaModel> {
    let p = probe_ollama(base_url).await;
    if p.reachable { p.models } else { vec![] }
}

fn find_probe_model<'a>(model: &str, probe: Option<&'a OllamaProbe>) -> Option<&'a OllamaModel> {
    let probe = probe?;
    let needle = model.to_lowercase();
    probe.models.iter().find(|m| {
        let name = m.name.to_lowercase();
        name == needle || name.starts_with(&needle) || needle.starts_with(&name)
    })
}

fn context_window_hint_for_name(model: &str) -> usize {
    let m = model.to_lowercase();
    if m.contains("70b") || m.contains("72b") || m.contains("120b") || m.contains("gpt-oss") {
        32_768
    } else if m.contains("27b") || m.contains("32b") {
        16_384
    } else if m.contains("13b") || m.contains("14b") {
        12_288
    } else {
        8_192
    }
}

fn likely_tool_support_for_name(model: &str) -> bool {
    let m = model.to_lowercase();
    if m.contains("embed") || m.contains("embedding") {
        return false;
    }
    true
}

/// Derives local reliability/capability hints from a model id and optional probe data.
///
/// Probe failures are safe: callers can pass `None` and still get deterministic defaults.
#[must_use]
pub fn infer_ollama_capability_hint(
    model: &str,
    probe: Option<&OllamaProbe>,
) -> OllamaCapabilityHint {
    let mut first_ms = ollama_first_token_deadline_ms(model);
    let mut idle_s = ollama_stream_idle_timeout_secs(model);
    let mut context_hint = context_window_hint_for_name(model);
    let mut likely_tool = likely_tool_support_for_name(model);

    if let Some(pm) = find_probe_model(model, probe) {
        if let Some(ctx) = pm.context_window_tokens {
            context_hint = usize::try_from(ctx).unwrap_or(usize::MAX).max(1);
        }
        if pm.size_bytes >= 25_000_000_000 {
            first_ms = first_ms.saturating_add(45_000);
            idle_s = idle_s.saturating_add(90);
        } else if pm.size_bytes >= 10_000_000_000 {
            first_ms = first_ms.saturating_add(30_000);
            idle_s = idle_s.saturating_add(60);
        } else if pm.size_bytes >= 5_000_000_000 {
            first_ms = first_ms.saturating_add(15_000);
            idle_s = idle_s.saturating_add(30);
        }
        likely_tool = likely_tool_support_for_name(&pm.name);
    }

    OllamaCapabilityHint {
        context_window_tokens_hint: context_hint,
        likely_tool_call_support: likely_tool,
        first_token_deadline_ms: first_ms,
        idle_stream_timeout_secs: idle_s,
    }
}

/// First-token deadline for Ollama (local load + inference can be tens of seconds on Apple Silicon).
#[must_use]
pub fn ollama_first_token_deadline_ms(model: &str) -> u64 {
    let m = model.to_lowercase();
    let base_secs: u64 =
        if m.contains("120b") || m.contains("70b") || m.contains("72b") || m.contains("gpt-oss") {
            120
        } else if m.contains("27b") || m.contains("32b") {
            90
        } else if m.contains("13b") || m.contains("14b") {
            60
        } else {
            45
        };
    (base_secs.saturating_add(30)).saturating_mul(1000)
}

/// Maximum silence between streamed lines from Ollama before considering the stream stalled.
#[must_use]
pub fn ollama_stream_idle_timeout_secs(model: &str) -> u64 {
    let m = model.to_lowercase();
    if m.contains("27b") || m.contains("32b") {
        180
    } else if m.contains("14b") || m.contains("13b") {
        120
    } else if m.contains("9b") || m.contains("7b") {
        90
    } else {
        60
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_hint_falls_back_without_probe() {
        let hint = infer_ollama_capability_hint("qwen2.5-coder:7b", None);
        assert!(hint.context_window_tokens_hint >= 8_192);
        assert!(hint.first_token_deadline_ms >= 45_000);
        assert!(hint.idle_stream_timeout_secs >= 60);
        assert!(hint.likely_tool_call_support);
    }

    #[test]
    fn capability_hint_uses_probe_size_and_context_length() {
        let probe = OllamaProbe {
            reachable: true,
            models: vec![OllamaModel {
                name: "qwen3.5:32b".into(),
                size_bytes: 30_000_000_000,
                context_window_tokens: Some(65_536),
            }],
        };
        let hint = infer_ollama_capability_hint("qwen3.5:32b", Some(&probe));
        assert_eq!(hint.context_window_tokens_hint, 65_536);
        assert!(hint.first_token_deadline_ms > ollama_first_token_deadline_ms("qwen3.5:32b"));
        assert!(hint.idle_stream_timeout_secs > ollama_stream_idle_timeout_secs("qwen3.5:32b"));
    }
}
