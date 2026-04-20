//! Provider metadata and runtime activity state.

use akmon_models::OllamaProbe;

/// Runtime/provider status used by headers, hints, and event handlers.
#[derive(Debug, Clone)]
pub struct ProviderRuntimeState {
    /// Provider label for UI.
    pub provider_display_name: String,
    /// OpenRouter routing flag.
    pub uses_openrouter: bool,
    /// Free local inference flag.
    pub free_local_inference: bool,
    /// Light-theme body rendering flag.
    pub light_body_text: bool,
    /// Agent run active flag.
    pub agent_running: bool,
    /// Current status line.
    pub agent_activity_line: String,
    /// Current iteration index.
    pub current_iteration: u32,
    /// Maximum iterations.
    pub max_iterations: u32,
    /// Streaming cursor visibility bit.
    pub stream_cursor_visible: bool,
    /// Index mode flag.
    pub index_enabled: bool,
    /// Latest Ollama probe.
    pub ollama_probe: OllamaProbe,
}

impl ProviderRuntimeState {
    /// Creates runtime state from launch metadata.
    #[must_use]
    pub fn new(
        provider_display_name: String,
        uses_openrouter: bool,
        free_local_inference: bool,
        light_body_text: bool,
        max_iterations: u32,
        index_enabled: bool,
    ) -> Self {
        Self {
            provider_display_name,
            uses_openrouter,
            free_local_inference,
            light_body_text,
            agent_running: false,
            agent_activity_line: String::new(),
            current_iteration: 0,
            max_iterations,
            stream_cursor_visible: true,
            index_enabled,
            ollama_probe: OllamaProbe {
                reachable: false,
                models: vec![],
            },
        }
    }

    /// Applies provider confirmation updates.
    pub fn apply_provider_confirmed(&mut self, provider: &str) {
        self.provider_display_name = provider.to_string();
        self.uses_openrouter = provider == "OpenRouter";
        self.free_local_inference = provider == "Ollama";
    }

    /// Applies iteration progress.
    pub fn apply_iteration_started(&mut self, n: u32, max: u32) {
        self.current_iteration = n;
        self.max_iterations = max;
        self.agent_activity_line = format!("Step {n}/{max} · contacting model…");
    }

    /// Toggles stream cursor blink bit.
    pub fn tick_stream_cursor(&mut self) {
        self.stream_cursor_visible = !self.stream_cursor_visible;
    }
}
