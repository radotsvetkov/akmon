//! `akmon config` subcommands for `~/.akmon/config.toml`.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use akmon_config::{
    AkmonGlobalConfig, McpScope, McpServerEntry, append_akmon_gitignore_line, load_user_config,
    save_config_to, save_user_config,
};
use akmon_core::McpServerConfig;
use akmon_tools::discover_mcp_tools;
use clap::Subcommand;
use serde_json::json;

/// Top-level `akmon config …` options.
#[derive(Debug, Clone, clap::Args)]
pub struct ConfigArgs {
    /// Machine-readable JSON on stdout.
    #[arg(long, global = true)]
    pub json: bool,
    /// When omitted (and not `--json`), runs an interactive setup wizard.
    #[command(subcommand)]
    pub cmd: Option<ConfigSubcommand>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigSubcommand {
    /// Print the config file path.
    Path,
    /// Print full config (API keys masked).
    Show,
    /// Open the config in `$EDITOR` and validate TOML after exit.
    Edit,
    /// Reset config to empty defaults.
    Reset {
        #[arg(long)]
        yes: bool,
    },
    /// Default model helpers.
    #[command(subcommand)]
    Model(ModelCmd),
    /// Ollama base URL.
    #[command(subcommand)]
    OllamaUrl(OllamaUrlCmd),
    /// API key storage.
    #[command(subcommand)]
    Key(KeyCmd),
    /// MCP server registry.
    #[command(subcommand)]
    Mcp(McpCmd),
    /// Show how Akmon would route the current model to a provider (explainability only; does not change routing).
    ExplainProvider,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ModelCmd {
    /// Print current default model.
    Get,
    /// Set default model.
    Set { model: String },
    /// List models from Ollama and common Anthropic ids when a key is set.
    List,
    /// Send a minimal prompt to verify the model responds.
    Test { model: Option<String> },
}

#[derive(Debug, Clone, Subcommand)]
pub enum OllamaUrlCmd {
    Set { url: String },
}

#[derive(Debug, Clone, Subcommand)]
pub enum KeyCmd {
    Set {
        provider: KeyProvider,
        key: String,
    },
    Unset {
        provider: KeyProvider,
    },
    /// Report whether configured credentials look present.
    Test,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum KeyProvider {
    Anthropic,
}

#[derive(Debug, Clone, Subcommand)]
pub enum McpCmd {
    List,
    Add {
        name: String,
        url: String,
        #[arg(long, value_enum, default_value_t = McpScopeArg::User)]
        scope: McpScopeArg,
        #[arg(long)]
        test: bool,
        #[arg(long = "env", value_name = "KEY=VAL")]
        env: Vec<String>,
    },
    /// Remove a server by name.
    #[command(alias = "delete", alias = "rm")]
    Remove {
        name: String,
    },
    Enable {
        name: String,
    },
    Disable {
        name: String,
    },
    Test {
        name: Option<String>,
    },
    Show {
        name: String,
    },
}

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum McpScopeArg {
    #[default]
    User,
    Project,
}

impl From<McpScopeArg> for McpScope {
    fn from(a: McpScopeArg) -> Self {
        match a {
            McpScopeArg::User => McpScope::User,
            McpScopeArg::Project => McpScope::Project,
        }
    }
}

/// Runs one `akmon config` invocation.
pub async fn run_config(args: ConfigArgs, cli: &crate::Cli) -> ExitCode {
    match run_config_inner(args, cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("✗ {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run_config_inner(args: ConfigArgs, cli: &crate::Cli) -> Result<(), String> {
    let Some(cmd) = args.cmd.as_ref() else {
        if args.json {
            return Err(
                "`akmon config --json` requires a subcommand (e.g. `akmon config show --json`)."
                    .into(),
            );
        }
        return run_config_wizard().await;
    };
    match cmd {
        ConfigSubcommand::Path => {
            let Some(p) = akmon_config::akmon_config_path() else {
                return Err("cannot resolve home directory".into());
            };
            if args.json {
                let j = json!({ "path": p });
                println!("{j}");
            } else {
                let pd = p.display();
                println!("{pd}");
            }
            Ok(())
        }
        ConfigSubcommand::Show => {
            let (_, cfg) = load_user_config().map_err(|e| e.to_string())?;
            if args.json {
                let v = json!({
                    "default_model": cfg.default_model,
                    "ollama_url": cfg.ollama_url,
                    "anthropic_api_key": cfg.anthropic_api_key.as_ref().map(|k| mask_key(k)),
                    "openrouter_api_key": cfg.openrouter_api_key.as_ref().map(|k| mask_key(k)),
                    "openai_api_key": cfg.openai_api_key.as_ref().map(|k| mask_key(k)),
                    "groq_api_key": cfg.groq_api_key.as_ref().map(|k| mask_key(k)),
                    "azure_openai_endpoint": cfg.azure_openai_endpoint,
                    "azure_openai_api_key": cfg.azure_openai_api_key.as_ref().map(|k| mask_key(k)),
                    "azure_api_version": cfg.azure_api_version,
                    "openai_compatible_url": cfg.openai_compatible_url,
                    "openai_compatible_api_key": cfg.openai_compatible_api_key.as_ref().map(|k| mask_key(k)),
                    "architect": cfg.architect,
                    "policy": cfg.policy,
                    "slo": cfg.slo,
                    "mcp": cfg.mcp,
                });
                let pretty = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
                println!("{pretty}");
            } else {
                let toml = cfg.display_masked_toml();
                print!("{toml}");
            }
            Ok(())
        }
        ConfigSubcommand::Edit => {
            let (path, _) = load_user_config().map_err(|e| e.to_string())?;
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".into());
            let st = Command::new(&editor)
                .arg(&path)
                .status()
                .map_err(|e| format!("failed to run {editor}: {e}"))?;
            if !st.success() {
                return Err(format!("editor exited with {st}"));
            }
            let _ = akmon_config::load_config_from(&path).map_err(|e| e.to_string())?;
            if args.json {
                let j = json!({ "ok": true, "path": path });
                println!("{j}");
            } else {
                let pd = path.display();
                println!("✓ config valid: {pd}");
            }
            Ok(())
        }
        ConfigSubcommand::Reset { yes } => {
            if !yes {
                print!("Reset ~/.akmon/config.toml to defaults? [y/N]: ");
                let _ = io::stdout().flush();
                let mut line = String::new();
                io::stdin()
                    .read_line(&mut line)
                    .map_err(|e| e.to_string())?;
                if !line.trim().eq_ignore_ascii_case("y") {
                    return Err("aborted".into());
                }
            }
            let (path, _) = load_user_config().map_err(|e| e.to_string())?;
            save_config_to(&path, &AkmonGlobalConfig::default()).map_err(|e| e.to_string())?;
            if args.json {
                let j = json!({ "ok": true });
                println!("{j}");
            } else {
                println!("✓ config reset");
            }
            Ok(())
        }
        ConfigSubcommand::Model(m) => match m {
            ModelCmd::Get => {
                let (_, cfg) = load_user_config().map_err(|e| e.to_string())?;
                let d = cfg.default_model.unwrap_or_else(|| "llama3.2".into());
                if args.json {
                    let j = json!({ "default": d });
                    println!("{j}");
                } else {
                    println!("default: {d}");
                }
                Ok(())
            }
            ModelCmd::Set { model } => {
                let (_, mut cfg) = load_user_config().map_err(|e| e.to_string())?;
                cfg.default_model = Some(model.clone());
                save_user_config(&cfg).map_err(|e| e.to_string())?;
                if args.json {
                    let j = json!({ "ok": true, "default": model });
                    println!("{j}");
                } else {
                    println!("✓ default model → {model}");
                }
                Ok(())
            }
            ModelCmd::List => model_list(&args).await,
            ModelCmd::Test { model } => model_test(&args, model.as_deref()).await,
        },
        ConfigSubcommand::OllamaUrl(OllamaUrlCmd::Set { url }) => {
            let (_, mut cfg) = load_user_config().map_err(|e| e.to_string())?;
            cfg.ollama_url = Some(url.clone());
            save_user_config(&cfg).map_err(|e| e.to_string())?;
            if args.json {
                let j = json!({ "ok": true, "ollama_url": url });
                println!("{j}");
            } else {
                println!("✓ ollama_url → {url}");
            }
            Ok(())
        }
        ConfigSubcommand::Key(k) => match k {
            KeyCmd::Set {
                provider: KeyProvider::Anthropic,
                key,
            } => {
                let (_, mut cfg) = load_user_config().map_err(|e| e.to_string())?;
                cfg.anthropic_api_key = Some(key.clone());
                save_user_config(&cfg).map_err(|e| e.to_string())?;
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                if append_akmon_gitignore_line(&cwd).unwrap_or(false) && !args.json {
                    let cwd_disp = cwd.display();
                    eprintln!("✓ appended .akmon/ to .gitignore in {cwd_disp}");
                }
                if !args.json {
                    eprintln!(
                        "Warning: ~/.akmon/config.toml may contain secrets — do not commit it."
                    );
                }
                if args.json {
                    let j = json!({ "ok": true });
                    println!("{j}");
                } else {
                    println!("✓ anthropic key stored");
                }
                Ok(())
            }
            KeyCmd::Unset {
                provider: KeyProvider::Anthropic,
            } => {
                let (_, mut cfg) = load_user_config().map_err(|e| e.to_string())?;
                cfg.anthropic_api_key = None;
                save_user_config(&cfg).map_err(|e| e.to_string())?;
                if args.json {
                    let j = json!({ "ok": true });
                    println!("{j}");
                } else {
                    println!("✓ anthropic key removed");
                }
                Ok(())
            }
            KeyCmd::Test => {
                let (_, cfg) = load_user_config().map_err(|e| e.to_string())?;
                let url = cfg
                    .ollama_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:11434".into());
                let (ollama_ok, ollama_n) = match ollama_tags(&url).await {
                    Ok(n) => (true, n),
                    Err(_) => (false, 0),
                };
                if args.json {
                    let j = json!({
                        "anthropic_configured": cfg.anthropic_api_key.as_ref().is_some_and(|s| !s.is_empty()),
                        "ollama_running": ollama_ok,
                        "ollama_model_count": ollama_n,
                    });
                    println!("{j}");
                } else {
                    if cfg
                        .anthropic_api_key
                        .as_ref()
                        .is_some_and(|s| !s.is_empty())
                    {
                        println!("anthropic  ✓ key set");
                    } else {
                        println!("anthropic  ✗ no key");
                    }
                    if ollama_ok {
                        println!("ollama     ✓ running ({ollama_n} models)");
                    } else {
                        println!("ollama     ✗ (ollama not running — start with: ollama serve)");
                    }
                }
                Ok(())
            }
        },
        ConfigSubcommand::Mcp(m) => run_mcp(&args, m).await,
        ConfigSubcommand::ExplainProvider => {
            let (_, global) = load_user_config().map_err(|e| e.to_string())?;
            let connect = crate::llm_connect_from_cli(cli, &global, cli.model.clone());
            let trace = connect.explain_provider_resolution();
            let use_json = args.json || matches!(cli.output, crate::OutputFormat::Json);
            if use_json {
                let pretty = serde_json::to_string_pretty(&trace).map_err(|e| e.to_string())?;
                println!("{pretty}");
            } else {
                print_explain_provider_text(&trace);
            }
            Ok(())
        }
    }
}

fn print_explain_provider_text(trace: &akmon_models::ProviderResolutionTrace) {
    println!(
        "Provider resolution (explainability only; same routing as `LlmConnectConfig::resolve`)"
    );
    println!("model_id: {}", trace.model_id);
    match (&trace.selected_provider, &trace.resolution_error) {
        (Some(p), None) => println!("selected_provider: {p}"),
        (None, Some(e)) => println!("resolution_error: {e}"),
        _ => {}
    }
    println!("{}", trace.selected_reason);
    println!();
    println!("candidates (priority_order ascending):");
    for c in &trace.candidates {
        let elig = if c.eligible {
            "eligible"
        } else {
            "not_eligible"
        };
        println!(
            "  {:2} {:18} {elig}: {}",
            c.priority_order, c.provider, c.reason
        );
        if !c.missing_prerequisites.is_empty() {
            let joined = c.missing_prerequisites.join(", ");
            println!("      missing_prerequisites: {joined}");
        }
    }
}

fn read_wizard_line(prompt: &str) -> Result<String, String> {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(line.trim().to_string())
}

/// Interactive stdin wizard when `akmon config` is run without a subcommand.
async fn run_config_wizard() -> Result<(), String> {
    let Some(path) = akmon_config::akmon_config_path() else {
        return Err("cannot resolve home directory".into());
    };
    let (_, mut cfg) = load_user_config().map_err(|e| e.to_string())?;
    println!("Akmon configuration wizard");
    println!("─────────────────────────");
    println!(
        "This writes `{}` (masked values: `akmon config show`).",
        path.display()
    );
    println!();
    println!("Tip: keys can also live in env vars (see docs: Environment variables).");
    println!("     `akmon config key set anthropic …` avoids pasting into this prompt.");
    println!();

    let cur = cfg
        .default_model
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("(not set)");
    let model = read_wizard_line(&format!("Default model [{cur}]: "))?;
    if !model.is_empty() {
        cfg.default_model = Some(model);
    }

    let anthropic = read_wizard_line("Anthropic API key (Enter to skip; paste is visible): ")?;
    if !anthropic.is_empty() {
        cfg.anthropic_api_key = Some(anthropic);
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if append_akmon_gitignore_line(&cwd).unwrap_or(false) {
            eprintln!("✓ appended .akmon/ to .gitignore in {}", cwd.display());
        }
        eprintln!("Warning: ~/.akmon/config.toml may contain secrets — do not commit it.");
    }

    let or_key = read_wizard_line("OpenRouter API key (Enter to skip): ")?;
    if !or_key.is_empty() {
        cfg.openrouter_api_key = Some(or_key);
    }

    let ou = cfg
        .ollama_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:11434".into());
    let url = read_wizard_line(&format!("Ollama URL [{ou}]: "))?;
    if !url.is_empty() {
        cfg.ollama_url = Some(url);
    }

    save_user_config(&cfg).map_err(|e| e.to_string())?;
    println!();
    println!("✓ Saved. Next:  akmon config show");
    println!("CLI reference:  akmon config --help");
    Ok(())
}

fn mask_key(k: &str) -> String {
    if k.len() <= 8 {
        "****".into()
    } else {
        let prefix: String = k.chars().take(8).collect();
        format!("{prefix}****")
    }
}

async fn openrouter_models_top_display(api_key: &str) -> Result<String, String> {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|e| e.to_string())?;
    let r = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", {
            let key = api_key.trim();
            format!("Bearer {key}")
        })
        .header("HTTP-Referer", "https://akmon.dev")
        .header("X-Title", "Akmon")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        let st = r.status();
        return Err(format!("HTTP {st}"));
    }
    let v: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
    let Some(arr) = v.get("data").and_then(|x| x.as_array()) else {
        return Ok("  (no data)\n".into());
    };
    #[derive(Clone)]
    struct Row {
        id: String,
        ctx: u64,
        price: String,
    }
    let mut rows: Vec<Row> = Vec::new();
    for m in arr {
        let id = m
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let ctx = m
            .get("context_length")
            .and_then(|x| x.as_u64())
            .or_else(|| {
                m.get("top_provider")
                    .and_then(|t| t.get("context_length"))
                    .and_then(|x| x.as_u64())
            })
            .unwrap_or(0);
        let price = m
            .get("pricing")
            .map(|p| {
                let prompt = p.get("prompt").and_then(|x| x.as_str()).unwrap_or("?");
                let comp = p.get("completion").and_then(|x| x.as_str()).unwrap_or("?");
                format!("{prompt}/{comp}")
            })
            .unwrap_or_else(|| "?".into());
        rows.push(Row { id, ctx, price });
    }
    rows.sort_by(|a, b| b.ctx.cmp(&a.ctx));
    rows.truncate(20);
    let mut s = String::new();
    for row in rows {
        let ck = row.ctx / 1000;
        let id = &row.id;
        let price = &row.price;
        s.push_str(&format!("  {id}  {ck}k  {price}\n"));
    }
    Ok(s)
}

async fn model_list(args: &ConfigArgs) -> Result<(), String> {
    let (_, cfg) = load_user_config().map_err(|e| e.to_string())?;
    let url = cfg
        .ollama_url
        .clone()
        .unwrap_or_else(|| "http://localhost:11434".into());
    let mut out = String::new();
    out.push_str("Ollama (local):\n");
    match ollama_models_display(&url).await {
        Ok(s) => out.push_str(&s),
        Err(_) => {
            out.push_str("  (ollama not running — start with: ollama serve)\n");
        }
    }
    if cfg
        .anthropic_api_key
        .as_ref()
        .is_some_and(|k| !k.is_empty())
    {
        out.push_str("Anthropic (API):\n");
        for line in anthropic_model_lines() {
            out.push_str(&line);
        }
    }
    let mut openrouter_text: Option<String> = None;
    if let Some(or_key) = cfg.openrouter_api_key.as_deref() {
        let key = or_key.trim();
        if !key.is_empty() {
            out.push_str("OpenRouter:\n");
            match openrouter_models_top_display(key).await {
                Ok(s) => {
                    out.push_str(&s);
                    openrouter_text = Some(s);
                }
                Err(e) => {
                    let line = format!("  (could not list: {e})\n");
                    out.push_str(&line);
                    openrouter_text = Some(line);
                }
            }
        }
    }
    if args.json {
        let tags = ollama_tags_json(&url).await.unwrap_or_default();
        let j = json!({
            "ollama": tags,
            "anthropic_key_set": cfg.anthropic_api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false),
            "openrouter_key_set": cfg.openrouter_api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false),
            "openrouter_top_display": openrouter_text,
        });
        println!("{j}");
    } else {
        print!("{out}");
    }
    Ok(())
}

fn anthropic_model_lines() -> Vec<String> {
    vec![
        "  claude-haiku-4-5-20251001  fast\n".into(),
        "  claude-sonnet-4-6          balanced ★\n".into(),
    ]
}

async fn ollama_tags(base: &str) -> Result<usize, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/api/tags", base.trim_end_matches('/'));
    let r = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err("bad status".into());
    }
    let v: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
    let n = v
        .get("models")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    Ok(n)
}

async fn ollama_tags_json(base: &str) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/api/tags", base.trim_end_matches('/'));
    let r = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let v: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
    Ok(v)
}

async fn ollama_models_display(base: &str) -> Result<String, String> {
    let v = ollama_tags_json(base).await?;
    let mut s = String::new();
    if let Some(arr) = v.get("models").and_then(|m| m.as_array()) {
        for m in arr {
            let name = m.get("name").and_then(|x| x.as_str()).unwrap_or("?");
            let size = m
                .get("size")
                .and_then(|x| x.as_u64())
                .map(|b| format!("{:.1} GB", b as f64 / 1e9))
                .unwrap_or_else(|| "?".into());
            let star = if name.contains("coder") {
                "  ★ coding"
            } else {
                ""
            };
            s.push_str(&format!("  {name:<22}  {size}{star}\n"));
        }
    }
    Ok(s)
}

async fn model_test(args: &ConfigArgs, model_override: Option<&str>) -> Result<(), String> {
    let (_, cfg) = load_user_config().map_err(|e| e.to_string())?;
    let model = model_override
        .map(|s| s.to_string())
        .or(cfg.default_model.clone())
        .ok_or_else(|| {
            "pass a model name or set default with akmon config model set".to_string()
        })?;
    let url = cfg
        .ollama_url
        .clone()
        .unwrap_or_else(|| "http://localhost:11434".into());
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let start = std::time::Instant::now();
    let body = json!({
        "model": model,
        "messages": [{"role":"user","content":"reply with only the word: ok"}],
        "stream": false
    });
    let r = client
        .post(format!("{}/api/chat", url.trim_end_matches('/')))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !r.status().is_success() {
        let st = r.status();
        return Err(format!("Ollama returned {st}"));
    }
    let elapsed = start.elapsed().as_secs_f32();
    if args.json {
        let j = json!({ "ok": true, "model": model, "seconds": elapsed });
        println!("{j}");
    } else {
        println!("Testing {model}…");
        println!("✓ responded in {elapsed:.1}s");
    }
    Ok(())
}

async fn run_mcp(args: &ConfigArgs, m: &McpCmd) -> Result<(), String> {
    let (_, mut cfg) = load_user_config().map_err(|e| e.to_string())?;
    match m {
        McpCmd::List => {
            if args.json {
                let pretty = serde_json::to_string_pretty(&json!({ "mcp": cfg.mcp }))
                    .map_err(|e| e.to_string())?;
                println!("{pretty}");
            } else {
                println!("user scope:");
                for e in cfg.mcp.iter().filter(|e| e.scope == McpScope::User) {
                    let on = if e.enabled { "[on]" } else { "[off]" };
                    let n = &e.name;
                    let u = &e.url;
                    println!("  {n}  {u}  {on}");
                }
                println!("project scope:");
                for e in cfg.mcp.iter().filter(|e| e.scope == McpScope::Project) {
                    let on = if e.enabled { "[on]" } else { "[off]" };
                    let n = &e.name;
                    let u = &e.url;
                    println!("  {n}  {u}  {on}");
                }
            }
            Ok(())
        }
        McpCmd::Add {
            name,
            url,
            scope,
            test,
            env: _,
        } => {
            if cfg.mcp.iter().any(|e| e.name == *name) {
                return Err(format!("server '{name}' already exists"));
            }
            let server_cfg = McpServerConfig {
                name: name.clone(),
                url: url.clone(),
                description: String::new(),
            };
            let discover = discover_mcp_tools(&server_cfg).await;
            if *test {
                discover
                    .as_ref()
                    .map_err(|e| format!("MCP test failed: {e}"))?;
            }
            let tool_count = discover.map(|t| t.len()).unwrap_or(0);
            let scope_m: McpScope = (*scope).into();
            cfg.mcp.push(McpServerEntry {
                name: name.clone(),
                url: url.clone(),
                enabled: true,
                scope: scope_m,
            });
            save_user_config(&cfg).map_err(|e| e.to_string())?;
            let scope_label = match scope {
                McpScopeArg::User => "user",
                McpScopeArg::Project => "project",
            };
            if args.json {
                let j = json!({ "ok": true, "name": name, "scope": scope_label, "tool_count": tool_count });
                println!("{j}");
            } else {
                println!("✓ {name} added ({scope_label} scope, {tool_count} tools)");
            }
            Ok(())
        }
        McpCmd::Remove { name } => {
            let n0 = cfg.mcp.len();
            cfg.mcp.retain(|e| e.name != *name);
            if cfg.mcp.len() == n0 {
                return Err(format!("unknown server: {name}"));
            }
            save_user_config(&cfg).map_err(|e| e.to_string())?;
            if args.json {
                let j = json!({ "ok": true });
                println!("{j}");
            } else {
                println!("✓ {name} removed");
            }
            Ok(())
        }
        McpCmd::Enable { name } => {
            let Some(e) = cfg.mcp.iter_mut().find(|e| e.name == *name) else {
                return Err(format!("unknown server: {name}"));
            };
            e.enabled = true;
            save_user_config(&cfg).map_err(|e| e.to_string())?;
            if args.json {
                let j = json!({ "ok": true });
                println!("{j}");
            } else {
                println!("✓ {name} enabled");
            }
            Ok(())
        }
        McpCmd::Disable { name } => {
            let Some(e) = cfg.mcp.iter_mut().find(|e| e.name == *name) else {
                return Err(format!("unknown server: {name}"));
            };
            e.enabled = false;
            save_user_config(&cfg).map_err(|e| e.to_string())?;
            if args.json {
                let j = json!({ "ok": true });
                println!("{j}");
            } else {
                println!("✓ {name} disabled");
            }
            Ok(())
        }
        McpCmd::Test { name } => {
            let list: Vec<_> = if let Some(n) = name {
                cfg.mcp.iter().filter(|e| &e.name == n).collect()
            } else {
                cfg.mcp.iter().collect()
            };
            for e in list {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(8))
                    .build()
                    .map_err(|err| err.to_string())?;
                match client.get(&e.url).send().await {
                    Ok(r) if r.status().is_success() => {
                        if args.json {
                            let j = json!({ "name": e.name, "ok": true });
                            println!("{j}");
                        } else {
                            let n = &e.name;
                            let u = &e.url;
                            println!("{n}  {u}  ✓");
                        }
                    }
                    Ok(r) => {
                        if args.json {
                            let j = json!({ "name": e.name, "ok": false, "status": r.status().as_u16() });
                            println!("{j}");
                        } else {
                            let n = &e.name;
                            let u = &e.url;
                            let st = r.status();
                            println!("{n}  {u}  ✗ status {st}");
                        }
                    }
                    Err(_) => {
                        if args.json {
                            let j = json!({ "name": e.name, "ok": false, "error": "timeout" });
                            println!("{j}");
                        } else {
                            let n = &e.name;
                            let u = &e.url;
                            println!("{n}  {u}  ✗ timeout");
                        }
                    }
                }
            }
            Ok(())
        }
        McpCmd::Show { name } => {
            let Some(e) = cfg.mcp.iter().find(|e| e.name == *name) else {
                return Err(format!("unknown server: {name}"));
            };
            if args.json {
                let pretty = serde_json::to_string_pretty(e).map_err(|err| err.to_string())?;
                println!("{pretty}");
            } else {
                let n = &e.name;
                let u = &e.url;
                let en = e.enabled;
                println!("name: {n}");
                println!("url: {u}");
                println!("enabled: {en}");
                println!("scope: {:?}", e.scope);
                println!("(tool list requires MCP session — use `akmon` with --mcp-server)");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use clap::Parser;
    use std::path::Path;
    use std::process::ExitCode;
    use std::sync::Mutex;

    static FAKE_HOME_LOCK: Mutex<()> = Mutex::new(());

    fn sample_cli() -> crate::Cli {
        crate::Cli::parse_from(["akmon", "--model", "llama3.2"])
    }

    struct TempHome {
        old: Option<std::ffi::OsString>,
    }

    impl TempHome {
        fn new(home: &Path) -> Self {
            let old = std::env::var_os("HOME");
            // SAFETY: `FAKE_HOME_LOCK` serializes tests that mutate `HOME`.
            unsafe {
                std::env::set_var("HOME", home);
            }
            Self { old }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            // SAFETY: same as `TempHome::new`.
            unsafe {
                match &self.old {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
    }

    #[tokio::test]
    async fn config_json_without_subcommand_errors() {
        let code = run_config(
            ConfigArgs {
                json: true,
                cmd: None,
            },
            &sample_cli(),
        )
        .await;
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[tokio::test]
    async fn model_set_saves_to_config() {
        let _lock = FAKE_HOME_LOCK.lock().expect("lock");
        let tmp = tempfile::tempdir().expect("tmp");
        let _home = TempHome::new(tmp.path());
        let code = run_config(
            ConfigArgs {
                json: true,
                cmd: Some(ConfigSubcommand::Model(ModelCmd::Set {
                    model: "llama3.2".into(),
                })),
            },
            &sample_cli(),
        )
        .await;
        assert_eq!(code, ExitCode::SUCCESS);
        let path = tmp.path().join(".akmon/config.toml");
        let cfg = akmon_config::load_config_from(&path).expect("load");
        assert_eq!(cfg.default_model.as_deref(), Some("llama3.2"));
    }

    #[tokio::test]
    async fn mcp_add_saves_project_scope() {
        let _lock = FAKE_HOME_LOCK.lock().expect("lock");
        let tmp = tempfile::tempdir().expect("tmp");
        let _home = TempHome::new(tmp.path());
        let code = run_config(
            ConfigArgs {
                json: false,
                cmd: Some(ConfigSubcommand::Mcp(McpCmd::Add {
                    name: "srv".into(),
                    url: "http://127.0.0.1:9/none".into(),
                    scope: McpScopeArg::Project,
                    test: false,
                    env: vec![],
                })),
            },
            &sample_cli(),
        )
        .await;
        assert_eq!(code, ExitCode::SUCCESS);
        let path = tmp.path().join(".akmon/config.toml");
        let cfg = akmon_config::load_config_from(&path).expect("load");
        assert_eq!(cfg.mcp.len(), 1);
        assert_eq!(cfg.mcp[0].scope, McpScope::Project);
    }

    #[tokio::test]
    async fn mcp_remove_deletes_entry() {
        let _lock = FAKE_HOME_LOCK.lock().expect("lock");
        let tmp = tempfile::tempdir().expect("tmp");
        let _home = TempHome::new(tmp.path());
        run_config(
            ConfigArgs {
                json: true,
                cmd: Some(ConfigSubcommand::Mcp(McpCmd::Add {
                    name: "x".into(),
                    url: "http://127.0.0.1:9/none".into(),
                    scope: McpScopeArg::User,
                    test: false,
                    env: vec![],
                })),
            },
            &sample_cli(),
        )
        .await;
        let code = run_config(
            ConfigArgs {
                json: true,
                cmd: Some(ConfigSubcommand::Mcp(McpCmd::Remove { name: "x".into() })),
            },
            &sample_cli(),
        )
        .await;
        assert_eq!(code, ExitCode::SUCCESS);
        let path = tmp.path().join(".akmon/config.toml");
        let cfg = akmon_config::load_config_from(&path).expect("load");
        assert!(cfg.mcp.is_empty());
    }

    #[tokio::test]
    async fn mcp_disable_sets_enabled_false() {
        let _lock = FAKE_HOME_LOCK.lock().expect("lock");
        let tmp = tempfile::tempdir().expect("tmp");
        let _home = TempHome::new(tmp.path());
        run_config(
            ConfigArgs {
                json: true,
                cmd: Some(ConfigSubcommand::Mcp(McpCmd::Add {
                    name: "y".into(),
                    url: "http://127.0.0.1:9/none".into(),
                    scope: McpScopeArg::User,
                    test: false,
                    env: vec![],
                })),
            },
            &sample_cli(),
        )
        .await;
        let code = run_config(
            ConfigArgs {
                json: true,
                cmd: Some(ConfigSubcommand::Mcp(McpCmd::Disable { name: "y".into() })),
            },
            &sample_cli(),
        )
        .await;
        assert_eq!(code, ExitCode::SUCCESS);
        let path = tmp.path().join(".akmon/config.toml");
        let cfg = akmon_config::load_config_from(&path).expect("load");
        assert!(!cfg.mcp[0].enabled);
    }

    #[tokio::test]
    async fn mcp_remove_unknown_exits_failure() {
        let _lock = FAKE_HOME_LOCK.lock().expect("lock");
        let tmp = tempfile::tempdir().expect("tmp");
        let _home = TempHome::new(tmp.path());
        let code = run_config(
            ConfigArgs {
                json: false,
                cmd: Some(ConfigSubcommand::Mcp(McpCmd::Remove {
                    name: "nope".into(),
                })),
            },
            &sample_cli(),
        )
        .await;
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[test]
    fn show_masks_api_key_in_toml() {
        let cfg = AkmonGlobalConfig {
            anthropic_api_key: Some("sk-ant-api03-secretvaluehere".into()),
            ..Default::default()
        };
        let toml = cfg.display_masked_toml();
        assert!(!toml.contains("secretvaluehere"));
        assert!(toml.contains("sk-ant-a"));
    }

    #[tokio::test]
    async fn key_set_appends_akmon_to_gitignore_when_present() {
        let _lock = FAKE_HOME_LOCK.lock().expect("lock");
        let tmp = tempfile::tempdir().expect("tmp");
        let _home = TempHome::new(tmp.path());
        let proj = tempfile::tempdir().expect("proj");
        std::fs::write(proj.path().join(".gitignore"), "node_modules/\n").expect("gi");
        let old_cwd = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(proj.path()).expect("cd");
        let code = run_config(
            ConfigArgs {
                json: true,
                cmd: Some(ConfigSubcommand::Key(KeyCmd::Set {
                    provider: KeyProvider::Anthropic,
                    key: "sk-ant-test".into(),
                })),
            },
            &sample_cli(),
        )
        .await;
        std::env::set_current_dir(&old_cwd).expect("cd back");
        assert_eq!(code, ExitCode::SUCCESS);
        let gi = std::fs::read_to_string(proj.path().join(".gitignore")).expect("read");
        assert!(gi.contains(".akmon/"));
    }
}
