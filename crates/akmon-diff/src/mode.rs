use serde::{Deserialize, Serialize};

/// Diff execution mode.
///
/// v2.0.0 defines one mode. The enum shape is stable so future modes can be
/// added in a controlled, versioned way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffMode {
    /// Default lockstep diff behavior.
    Default,
}

impl std::fmt::Display for DiffMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DiffMode;

    #[test]
    fn t_diff_mode_json_round_trip() {
        let encoded = serde_json::to_string(&DiffMode::Default).expect("serialize");
        assert_eq!(encoded, "\"default\"");
        let decoded: DiffMode = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, DiffMode::Default);
    }
}
