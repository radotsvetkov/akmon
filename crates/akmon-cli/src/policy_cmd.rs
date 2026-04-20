//! `akmon policy` subcommands and effective policy resolution.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use akmon_config::AkmonGlobalConfig;
use akmon_core::{
    PolicyConfig, PolicyProfileName, built_in_policy_profile, merge_policy_config,
    parse_policy_config_file,
};
use clap::{Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::json;

/// CLI policy profile selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PolicyProfileArg {
    /// Developer profile.
    Dev,
    /// Staging profile.
    Staging,
    /// Production profile.
    Prod,
}

impl From<PolicyProfileArg> for PolicyProfileName {
    fn from(value: PolicyProfileArg) -> Self {
        match value {
            PolicyProfileArg::Dev => PolicyProfileName::Dev,
            PolicyProfileArg::Staging => PolicyProfileName::Staging,
            PolicyProfileArg::Prod => PolicyProfileName::Prod,
        }
    }
}

/// Top-level `akmon policy …` options.
#[derive(Debug, Clone, clap::Args)]
pub struct PolicyArgs {
    /// Policy subcommand.
    #[command(subcommand)]
    pub cmd: PolicySubcommand,
}

/// Supported `akmon policy` subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum PolicySubcommand {
    /// Print effective merged policy for operators.
    ShowEffective {
        /// Optional built-in profile.
        #[arg(long = "profile", value_enum)]
        profile: Option<PolicyProfileArg>,
        /// Additional policy pack path(s), repeatable.
        #[arg(long = "policy-pack", value_name = "PATH", action = clap::ArgAction::Append)]
        policy_pack: Vec<PathBuf>,
        /// Highest-precedence override policy file.
        #[arg(long = "policy-override", value_name = "PATH")]
        policy_override: Option<PathBuf>,
    },
}

/// Effective policy resolution options.
#[derive(Debug, Clone, Default)]
pub struct PolicyResolutionOptions {
    /// Optional profile selection.
    pub profile: Option<PolicyProfileName>,
    /// Additional pack paths (layer 2).
    pub pack_paths: Vec<PathBuf>,
    /// Highest-precedence override file.
    pub override_path: Option<PathBuf>,
}

/// Provenance + merged policy result.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedPolicy {
    /// Final merged effective policy.
    pub effective: PolicyConfig,
    /// Ordered source labels applied during merge.
    pub sources: Vec<String>,
}

/// Runs one `akmon policy` invocation.
pub fn run_policy(
    args: PolicyArgs,
    json_output: bool,
    project_root: &Path,
    global: &AkmonGlobalConfig,
) -> ExitCode {
    match args.cmd {
        PolicySubcommand::ShowEffective {
            profile,
            policy_pack,
            policy_override,
        } => {
            let opts = PolicyResolutionOptions {
                profile: profile.map(Into::into),
                pack_paths: policy_pack,
                override_path: policy_override,
            };
            match resolve_effective_policy(project_root, global, &opts) {
                Ok(Some(resolved)) => {
                    if json_output {
                        let payload = show_effective_json_payload(Some(&resolved), None);
                        println!("{payload}");
                    } else {
                        println!("policy show-effective: configured policy active");
                        println!("sources:");
                        for src in &resolved.sources {
                            println!(" - {src}");
                        }
                        match serde_json::to_string_pretty(&resolved.effective) {
                            Ok(pretty) => println!("{pretty}"),
                            Err(e) => {
                                eprintln!("policy show-effective: failed to serialize policy: {e}");
                                return ExitCode::from(1);
                            }
                        }
                    }
                    ExitCode::SUCCESS
                }
                Ok(None) => {
                    if json_output {
                        let payload = show_effective_json_payload(
                            None,
                            Some("no profile/packs/local/override policy sources selected"),
                        );
                        println!("{payload}");
                    } else {
                        println!(
                            "policy show-effective: no configured policy sources selected (runtime keeps interactive/--yes behavior)"
                        );
                    }
                    ExitCode::SUCCESS
                }
                Err(message) => {
                    if json_output {
                        let payload = json!({
                            "ok": false,
                            "error": message,
                        });
                        println!("{payload}");
                    } else {
                        eprintln!("policy show-effective: {message}");
                    }
                    ExitCode::from(1)
                }
            }
        }
    }
}

fn show_effective_json_payload(
    resolved: Option<&ResolvedPolicy>,
    message: Option<&str>,
) -> serde_json::Value {
    if let Some(r) = resolved {
        return json!({
            "ok": true,
            "configured_policy_active": true,
            "sources": r.sources,
            "effective_policy": r.effective,
        });
    }
    if let Some(msg) = message {
        return json!({
            "ok": true,
            "configured_policy_active": false,
            "message": msg
        });
    }
    json!({
        "ok": true,
        "configured_policy_active": false
    })
}

/// Resolves effective merged policy with precedence:
/// profile < packs < project-local policy < CLI override.
pub fn resolve_effective_policy(
    project_root: &Path,
    global: &AkmonGlobalConfig,
    options: &PolicyResolutionOptions,
) -> Result<Option<ResolvedPolicy>, String> {
    let selected_profile = options.profile.or(global.policy.profile);
    let discovered_pack_paths = discover_project_policy_packs(project_root)?;
    let global_pack_paths = global
        .policy
        .packs
        .iter()
        .map(|p| to_abs(project_root, Path::new(p)))
        .collect::<Vec<_>>();
    let explicit_pack_paths = options
        .pack_paths
        .iter()
        .map(|p| to_abs(project_root, p))
        .collect::<Vec<_>>();

    let local_policy_path = resolve_project_local_policy_file(project_root)?;
    let override_path = options
        .override_path
        .as_ref()
        .map(|p| to_abs(project_root, p));

    let has_any = selected_profile.is_some()
        || !discovered_pack_paths.is_empty()
        || !global_pack_paths.is_empty()
        || !explicit_pack_paths.is_empty()
        || local_policy_path.is_some()
        || override_path.is_some();
    if !has_any {
        return Ok(None);
    }

    let mut merged = PolicyConfig::default();
    let mut sources: Vec<String> = Vec::new();
    if let Some(profile) = selected_profile {
        merged = merge_policy_config(&merged, &built_in_policy_profile(profile));
        sources.push(format!("profile:{profile:?}"));
    }

    for pack in discovered_pack_paths
        .into_iter()
        .chain(global_pack_paths.into_iter())
        .chain(explicit_pack_paths.into_iter())
    {
        let parsed = parse_policy_config_file(&pack)
            .map_err(|e| format!("failed to load policy pack {}: {e}", pack.display()))?;
        merged = merge_policy_config(&merged, &parsed);
        sources.push(format!("pack:{}", pack.display()));
    }

    if let Some(local) = local_policy_path {
        let parsed = parse_policy_config_file(&local)
            .map_err(|e| format!("failed to load project policy {}: {e}", local.display()))?;
        merged = merge_policy_config(&merged, &parsed);
        sources.push(format!("local:{}", local.display()));
    }
    if let Some(override_file) = override_path {
        let parsed = parse_policy_config_file(&override_file).map_err(|e| {
            format!(
                "failed to load policy override {}: {e}",
                override_file.display()
            )
        })?;
        merged = merge_policy_config(&merged, &parsed);
        sources.push(format!("override:{}", override_file.display()));
    }

    Ok(Some(ResolvedPolicy {
        effective: merged,
        sources,
    }))
}

fn resolve_project_local_policy_file(project_root: &Path) -> Result<Option<PathBuf>, String> {
    let toml = project_root.join(".akmon").join("policy.toml");
    let json = project_root.join(".akmon").join("policy.json");
    let has_toml = toml.is_file();
    let has_json = json.is_file();
    match (has_toml, has_json) {
        (true, true) => Err(format!(
            "ambiguous local policy: both {} and {} exist",
            toml.display(),
            json.display()
        )),
        (true, false) => Ok(Some(toml)),
        (false, true) => Ok(Some(json)),
        (false, false) => Ok(None),
    }
}

fn discover_project_policy_packs(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    let dir = project_root.join(".akmon").join("policy-packs");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("failed to read {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| matches!(ext, "toml" | "json"))
        })
        .collect();
    out.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    Ok(out)
}

fn to_abs(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    project_root.join(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_config::{AkmonGlobalConfig, PolicyGovernanceConfig};
    use akmon_core::{PolicyEngineMode, ReplayHashInputs, build_replay_metadata};

    #[test]
    fn resolve_none_when_no_sources() {
        let dir = tempfile::tempdir().expect("tmp");
        let cfg = AkmonGlobalConfig::default();
        let resolved =
            resolve_effective_policy(dir.path(), &cfg, &PolicyResolutionOptions::default())
                .expect("ok");
        assert!(resolved.is_none());
    }

    #[test]
    fn precedence_profile_then_pack_then_local_then_override() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::create_dir_all(dir.path().join(".akmon")).expect("mkdir");
        let pack = dir.path().join(".akmon/policy-packs/10-team.toml");
        std::fs::create_dir_all(pack.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &pack,
            r#"[tools]
allow = ["read_*"]
deny = ["shell"]
"#,
        )
        .expect("write");
        let local = dir.path().join(".akmon/policy.toml");
        std::fs::write(&local, "[tools]\nallow=[\"shell\"]\n").expect("write");
        let override_file = dir.path().join("override.toml");
        std::fs::write(&override_file, "[tools]\ndeny=[\"shell\"]\n").expect("write");

        let cfg = AkmonGlobalConfig::default();
        let resolved = resolve_effective_policy(
            dir.path(),
            &cfg,
            &PolicyResolutionOptions {
                profile: Some(PolicyProfileName::Dev),
                pack_paths: Vec::new(),
                override_path: Some(override_file),
            },
        )
        .expect("ok")
        .expect("some");
        assert!(resolved.sources[0].starts_with("profile:"));
        assert!(resolved.sources.iter().any(|s| s.starts_with("pack:")));
        assert!(resolved.sources.iter().any(|s| s.starts_with("local:")));
        assert!(resolved.sources.iter().any(|s| s.starts_with("override:")));
        assert_eq!(
            resolved.effective.tools.deny.last().map(String::as_str),
            Some("shell")
        );
    }

    #[test]
    fn invalid_pack_fails_closed() {
        let dir = tempfile::tempdir().expect("tmp");
        let pack = dir.path().join(".akmon/policy-packs/10-bad.toml");
        std::fs::create_dir_all(pack.parent().expect("parent")).expect("mkdir");
        std::fs::write(&pack, "[tools\nbad").expect("write");
        let cfg = AkmonGlobalConfig::default();
        let err = resolve_effective_policy(
            dir.path(),
            &cfg,
            &PolicyResolutionOptions {
                profile: None,
                pack_paths: Vec::new(),
                override_path: None,
            },
        )
        .expect_err("should fail");
        assert!(err.contains("failed to load policy pack"));
    }

    #[test]
    fn project_pack_loading_order_is_deterministic() {
        let dir = tempfile::tempdir().expect("tmp");
        let pack_dir = dir.path().join(".akmon/policy-packs");
        std::fs::create_dir_all(&pack_dir).expect("mkdir");
        std::fs::write(pack_dir.join("20-b.toml"), "[tools]\nallow=[\"tool_b\"]\n").expect("write");
        std::fs::write(pack_dir.join("10-a.toml"), "[tools]\nallow=[\"tool_a\"]\n").expect("write");
        let cfg = AkmonGlobalConfig::default();
        let resolved = resolve_effective_policy(
            dir.path(),
            &cfg,
            &PolicyResolutionOptions {
                profile: None,
                pack_paths: Vec::new(),
                override_path: None,
            },
        )
        .expect("ok")
        .expect("some");
        assert!(resolved.sources[0].contains("10-a.toml"));
        assert!(resolved.sources[1].contains("20-b.toml"));
    }

    #[test]
    fn global_policy_defaults_are_applied() {
        let dir = tempfile::tempdir().expect("tmp");
        let cfg = AkmonGlobalConfig {
            policy: PolicyGovernanceConfig {
                profile: Some(PolicyProfileName::Prod),
                packs: Vec::new(),
            },
            ..Default::default()
        };
        let resolved =
            resolve_effective_policy(dir.path(), &cfg, &PolicyResolutionOptions::default())
                .expect("ok")
                .expect("some");
        assert!(resolved.sources.iter().any(|s| s.starts_with("profile:")));
    }

    #[test]
    fn show_effective_json_payload_shape_is_stable() {
        let payload = show_effective_json_payload(
            Some(&ResolvedPolicy {
                effective: PolicyConfig::default(),
                sources: vec!["profile:Dev".into()],
            }),
            None,
        );
        assert!(
            payload["configured_policy_active"]
                .as_bool()
                .unwrap_or(false)
        );
        assert!(payload["sources"].is_array());
        assert!(payload["effective_policy"].is_object());
    }

    #[test]
    fn policy_hash_changes_when_effective_policy_changes() {
        let dir = tempfile::tempdir().expect("tmp");
        let cfg = AkmonGlobalConfig::default();
        let dev = resolve_effective_policy(
            dir.path(),
            &cfg,
            &PolicyResolutionOptions {
                profile: Some(PolicyProfileName::Dev),
                pack_paths: Vec::new(),
                override_path: None,
            },
        )
        .expect("ok")
        .expect("some");
        let prod = resolve_effective_policy(
            dir.path(),
            &cfg,
            &PolicyResolutionOptions {
                profile: Some(PolicyProfileName::Prod),
                pack_paths: Vec::new(),
                override_path: None,
            },
        )
        .expect("ok")
        .expect("some");

        let dev_inputs = ReplayHashInputs {
            policy: serde_json::to_value(PolicyEngineMode::Configured(dev.effective))
                .expect("policy json"),
            config: serde_json::json!({}),
            tool_registry: serde_json::json!([]),
            prompt_assembly: None,
        };
        let prod_inputs = ReplayHashInputs {
            policy: serde_json::to_value(PolicyEngineMode::Configured(prod.effective))
                .expect("policy json"),
            config: serde_json::json!({}),
            tool_registry: serde_json::json!([]),
            prompt_assembly: None,
        };
        let dev_meta =
            build_replay_metadata("ollama", "llama3.2", "sess", &dev_inputs).expect("dev metadata");
        let prod_meta = build_replay_metadata("ollama", "llama3.2", "sess", &prod_inputs)
            .expect("prod metadata");
        assert_ne!(dev_meta.policy_hash, prod_meta.policy_hash);
    }
}
