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
}

fn parse_tags_json(json: serde_json::Value) -> Vec<OllamaModel> {
    if let Ok(env) = serde_json::from_value::<TagsEnvelope>(json.clone()) {
        return env
            .models
            .into_iter()
            .map(|m| OllamaModel {
                name: m.name,
                size_bytes: m.size,
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
