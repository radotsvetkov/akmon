//! Builds rows for the `/model` picker overlay (one section per configured provider).

use akmon_models::BEDROCK_DISPLAY_MODEL_IDS;

use crate::app::ModelPickerRow;
use crate::config::TuiLaunchConfig;

fn nonempty(s: &Option<String>) -> bool {
    s.as_ref().is_some_and(|x| !x.trim().is_empty())
}

/// When AWS access key is present, Bedrock resolution matches the CLI.
fn aws_env_suggests_bedrock() -> bool {
    std::env::var("AWS_ACCESS_KEY_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .is_some()
}

/// Constructs sectioned model suggestions; each provider block is omitted when not configured.
pub fn build_model_picker_rows(cfg: &TuiLaunchConfig) -> Vec<ModelPickerRow> {
    let mut out: Vec<ModelPickerRow> = Vec::new();

    out.push(ModelPickerRow {
        section_header: true,
        label: "Ollama (local)".to_string(),
    });
    for id in ["llama3.2", "qwen2.5-coder:7b", "codellama"] {
        out.push(ModelPickerRow {
            section_header: false,
            label: id.to_string(),
        });
    }

    if nonempty(&cfg.anthropic_key) {
        out.push(ModelPickerRow {
            section_header: true,
            label: "Anthropic (API)".to_string(),
        });
        for id in [
            "claude-haiku-4-5-20251001",
            "claude-sonnet-4-20250514",
            "claude-opus-4-1-20250805",
        ] {
            out.push(ModelPickerRow {
                section_header: false,
                label: id.to_string(),
            });
        }
    }

    if nonempty(&cfg.openrouter_key) {
        out.push(ModelPickerRow {
            section_header: true,
            label: "OpenRouter".to_string(),
        });
        for id in [
            "anthropic/claude-3.5-haiku",
            "anthropic/claude-3.5-sonnet",
            "meta-llama/llama-3.3-70b-instruct",
            "deepseek/deepseek-chat",
        ] {
            out.push(ModelPickerRow {
                section_header: false,
                label: id.to_string(),
            });
        }
    }

    if nonempty(&cfg.openai_key) {
        out.push(ModelPickerRow {
            section_header: true,
            label: "OpenAI".to_string(),
        });
        for id in ["gpt-4o", "gpt-4o-mini", "gpt-4-turbo"] {
            out.push(ModelPickerRow {
                section_header: false,
                label: id.to_string(),
            });
        }
    }

    if nonempty(&cfg.groq_key) {
        out.push(ModelPickerRow {
            section_header: true,
            label: "Groq".to_string(),
        });
        for id in ["llama-3.3-70b-versatile", "mixtral-8x7b-32768"] {
            out.push(ModelPickerRow {
                section_header: false,
                label: id.to_string(),
            });
        }
    }

    if nonempty(&cfg.azure_endpoint) && nonempty(&cfg.azure_key) {
        out.push(ModelPickerRow {
            section_header: true,
            label: "Azure OpenAI (deployment in URL)".to_string(),
        });
        out.push(ModelPickerRow {
            section_header: false,
            label: "gpt-4o".to_string(),
        });
    }

    if cfg.bedrock || aws_env_suggests_bedrock() {
        out.push(ModelPickerRow {
            section_header: true,
            label: "Amazon Bedrock".to_string(),
        });
        for id in BEDROCK_DISPLAY_MODEL_IDS {
            out.push(ModelPickerRow {
                section_header: false,
                label: (*id).to_string(),
            });
        }
    }

    if nonempty(&cfg.openai_compatible_url) && nonempty(&cfg.openai_compatible_key) {
        out.push(ModelPickerRow {
            section_header: true,
            label: "OpenAI-compatible (custom URL)".to_string(),
        });
        out.push(ModelPickerRow {
            section_header: false,
            label: "llama3.2".to_string(),
        });
    }

    out
}
