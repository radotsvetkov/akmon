//! Builds rows for the `/model` picker overlay (one section per configured provider).

use akmon_models::{BEDROCK_DISPLAY_MODEL_IDS, OllamaProbe};

use crate::app::ModelPickerRow;
use crate::config::TuiLaunchConfig;

fn nonempty(s: &Option<String>) -> bool {
    s.as_ref().is_some_and(|x| !x.trim().is_empty())
}

fn section(title: &str) -> ModelPickerRow {
    ModelPickerRow {
        section_header: true,
        selectable: false,
        label: title.to_string(),
        display: None,
    }
}

fn note(line: &str) -> ModelPickerRow {
    ModelPickerRow {
        section_header: false,
        selectable: false,
        label: line.to_string(),
        display: None,
    }
}

fn model_pick(id: &str, display: String) -> ModelPickerRow {
    ModelPickerRow {
        section_header: false,
        selectable: true,
        label: id.to_string(),
        display: Some(display),
    }
}

/// When AWS access key is present, Bedrock resolution matches the CLI.
fn aws_env_suggests_bedrock() -> bool {
    std::env::var("AWS_ACCESS_KEY_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .is_some()
}

/// Constructs sectioned model suggestions; cloud blocks reflect configured keys.
pub fn build_model_picker_rows(
    cfg: &TuiLaunchConfig,
    probe: &OllamaProbe,
    current_model: &str,
) -> Vec<ModelPickerRow> {
    let mut out: Vec<ModelPickerRow> = Vec::new();

    out.push(section("Local (Ollama — free, offline)"));
    if !probe.reachable {
        out.push(note("Ollama: not running (install from ollama.com)"));
    } else if probe.models.is_empty() {
        out.push(note(
            "No models installed — run: ollama pull qwen2.5-coder:7b",
        ));
    } else {
        let mut sorted = probe.models.clone();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        for m in sorted {
            let mark = if m.name == current_model { '●' } else { ' ' };
            let sz = m.display_size();
            let disp = format!("{mark} {:<30} {sz:>8}", m.name);
            out.push(model_pick(&m.name, disp));
        }
    }

    out.push(section("Anthropic"));
    if nonempty(&cfg.anthropic_key) {
        for id in [
            "claude-haiku-4-5-20251001",
            "claude-sonnet-4-6",
            "claude-opus-4-6",
        ] {
            let mark = if id == current_model { '●' } else { ' ' };
            let disp = format!("{mark} {id}");
            out.push(model_pick(id, disp));
        }
    } else {
        out.push(note("Anthropic: not configured (/config to set up)"));
    }

    out.push(section("OpenRouter"));
    if nonempty(&cfg.openrouter_key) {
        for id in [
            "anthropic/claude-haiku-4-5",
            "meta-llama/llama-3.3-70b-instruct",
        ] {
            let mark = if id == current_model { '●' } else { ' ' };
            let disp = format!("{mark} {id}");
            out.push(model_pick(id, disp));
        }
    } else {
        out.push(note("OpenRouter: not configured (/config to set up)"));
    }

    if nonempty(&cfg.openai_key) {
        out.push(section("OpenAI"));
        for id in ["gpt-4o", "gpt-4o-mini", "gpt-4-turbo"] {
            let mark = if id == current_model { '●' } else { ' ' };
            let disp = format!("{mark} {id}");
            out.push(model_pick(id, disp));
        }
    }

    if nonempty(&cfg.groq_key) {
        out.push(section("Groq"));
        for id in ["llama-3.3-70b-versatile", "mixtral-8x7b-32768"] {
            let mark = if id == current_model { '●' } else { ' ' };
            let disp = format!("{mark} {id}");
            out.push(model_pick(id, disp));
        }
    }

    if nonempty(&cfg.azure_endpoint) && nonempty(&cfg.azure_key) {
        out.push(section("Azure OpenAI (deployment in URL)"));
        let id = "gpt-4o";
        let mark = if id == current_model { '●' } else { ' ' };
        out.push(model_pick(id, format!("{mark} {id}")));
    }

    if cfg.bedrock || aws_env_suggests_bedrock() {
        out.push(section("Amazon Bedrock"));
        for id in BEDROCK_DISPLAY_MODEL_IDS {
            let id = *id;
            let mark = if id == current_model { '●' } else { ' ' };
            let disp = format!("{mark} {id}");
            out.push(model_pick(id, disp));
        }
    }

    if nonempty(&cfg.openai_compatible_url) && nonempty(&cfg.openai_compatible_key) {
        out.push(section("OpenAI-compatible (custom URL)"));
        let id = "llama3.2";
        let mark = if id == current_model { '●' } else { ' ' };
        out.push(model_pick(id, format!("{mark} {id}")));
    }

    out
}
