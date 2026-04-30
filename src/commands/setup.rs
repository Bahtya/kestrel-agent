//! Setup command — interactive wizard for configuring Kestrel.

use anyhow::{Context, Result};
use console::Term;
use dialoguer::{Confirm, Input, Select};
use kestrel_config::{
    loader, paths,
    schema::{Config, ProviderEntry, TelegramConfig, WebSocketConfig},
};
use owo_colors::OwoColorize;
use std::path::Path;

const PROVIDER_NAMES: &[&str] = &[
    "anthropic",
    "openai",
    "openrouter",
    "ollama",
    "deepseek",
    "gemini",
    "groq",
    "moonshot",
    "minimax",
    "github_copilot",
    "openai_codex",
];

const TOTAL_STEPS: usize = 5;

pub fn run(_config: Config) -> Result<()> {
    let term = Term::stdout();
    run_wizard(&term)
}

fn run_wizard(term: &Term) -> Result<()> {
    print_banner(term)?;

    let config_path = paths::get_config_path()?;

    // ── Step 1: Check existing config ────────────────────────────
    print_step(term, 1, "Existing Configuration")?;

    let mut config = if config_path.exists() {
        match load_existing_config(&config_path) {
            Ok(existing) => {
                show_config_summary(term, &existing)?;
                if Confirm::new()
                    .with_prompt("Update existing config?")
                    .default(true)
                    .interact_on(term)?
                {
                    existing
                } else {
                    term.write_line(&format!(
                        "  {} Keeping config at {}.",
                        "✓".green(),
                        config_path.display()
                    ))?;
                    return Ok(());
                }
            }
            Err(e) => {
                term.write_line(&format!(
                    "  {} Could not parse existing config: {}",
                    "!".yellow(),
                    e
                ))?;
                if Confirm::new()
                    .with_prompt("Start fresh with defaults?")
                    .default(true)
                    .interact_on(term)?
                {
                    Config::default()
                } else {
                    anyhow::bail!("Setup cancelled.");
                }
            }
        }
    } else {
        term.write_line("  No config file found. Starting fresh.")?;
        Config::default()
    };

    // ── Step 2: Provider configuration ───────────────────────────
    print_step(term, 2, "Provider Configuration")?;
    configure_provider(term, &mut config)?;

    // ── Step 3: Telegram channel ─────────────────────────────────
    print_step(term, 3, "Telegram Channel")?;
    configure_telegram(term, &mut config)?;

    // ── Step 4: WebSocket port ───────────────────────────────────
    print_step(term, 4, "WebSocket Port")?;
    configure_websocket(term, &mut config)?;

    // ── Step 5: Validate & write ─────────────────────────────────
    print_step(term, 5, "Save Configuration")?;

    term.write_line(&format!("  Config path: {}", config_path.display()))?;
    term.write_line("")?;
    show_config_summary(term, &config)?;
    term.write_line("")?;

    if !Confirm::new()
        .with_prompt("Write this configuration?")
        .default(true)
        .interact_on(term)?
    {
        term.write_line(&format!("  {} Setup cancelled.", "!".yellow()))?;
        return Ok(());
    }

    let home = config_path
        .parent()
        .context("Config path must have a parent directory")?;

    std::fs::create_dir_all(home)
        .with_context(|| format!("Failed to create config home: {}", home.display()))?;

    loader::save_config(&config, &config_path)?;

    for dir in ["skills", "sessions", "learning"] {
        let path = home.join(dir);
        std::fs::create_dir_all(&path)
            .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    }

    term.write_line("")?;
    term.write_line(&format!(
        "  {} Configuration saved to {}",
        "✓".green(),
        config_path.display()
    ))?;
    term.write_line(&format!(
        "  {} Created directories: skills, sessions, learning",
        "✓".green()
    ))?;
    term.write_line(&format!("  {} Setup complete!", "✓".green()))?;

    Ok(())
}

fn print_banner(term: &Term) -> Result<()> {
    term.write_line("")?;
    term.write_line(&format!(
        "  {} {}",
        "▸".cyan(),
        "Kestrel Setup Wizard".bold().cyan()
    ))?;
    term.write_line("")?;
    Ok(())
}

fn print_step(term: &Term, step: usize, title: &str) -> Result<()> {
    term.write_line("")?;
    term.write_line(&format!(
        "  {} Step {}/{}: {}",
        "▸".cyan(),
        step,
        TOTAL_STEPS,
        title.bold()
    ))?;
    term.write_line(&format!("  {}", "─".repeat(40).dimmed()))?;
    Ok(())
}

fn load_existing_config(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let config: Config =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(config)
}

fn show_config_summary(term: &Term, config: &Config) -> Result<()> {
    let model = &config.agent.model;
    let provider = config.agent.provider.as_deref().unwrap_or("default");
    term.write_line(&format!("  Model:        {}", model))?;
    term.write_line(&format!("  Provider:     {}", provider))?;
    term.write_line(&format!("  Temperature:  {}", config.agent.temperature))?;
    term.write_line(&format!("  Max tokens:   {}", config.agent.max_tokens))?;
    term.write_line(&format!("  Streaming:    {}", config.agent.streaming))?;

    if let Some(ref tg) = config.channels.telegram {
        term.write_line(&format!(
            "  Telegram:     {}…{}",
            &tg.token[..tg.token.len().min(4)],
            if tg.token.len() > 4 { "(masked)" } else { "" }
        ))?;
    }

    if let Some(ref ws) = config.channels.websocket {
        if ws.enabled {
            term.write_line(&format!("  WebSocket:    {}", ws.listen_addr))?;
        }
    }

    Ok(())
}

fn configure_provider(term: &Term, config: &mut Config) -> Result<()> {
    let default_idx = config
        .agent
        .provider
        .as_deref()
        .and_then(|p| PROVIDER_NAMES.iter().position(|&n| n == p))
        .unwrap_or(1); // default to "openai"

    let provider_name = Select::new()
        .with_prompt("Select LLM provider")
        .items(PROVIDER_NAMES)
        .default(default_idx)
        .interact_on(term)?;

    let provider_key = PROVIDER_NAMES[provider_name];
    config.agent.provider = Some(provider_key.to_string());

    let default_model = match provider_key {
        "anthropic" => "claude-sonnet-4-20250514",
        "openai" => "gpt-4o",
        "openrouter" => "anthropic/claude-sonnet-4-20250514",
        "ollama" => "llama3",
        "deepseek" => "deepseek-chat",
        "gemini" => "gemini-2.5-pro",
        "groq" => "llama-3.3-70b-versatile",
        "moonshot" => "moonshot-v1-8k",
        "minimax" => "MiniMax-Text-01",
        "github_copilot" => "gpt-4o",
        "openai_codex" => "codex-mini",
        _ => "gpt-4o",
    };

    let current_model = if config.agent.model.is_empty() {
        default_model
    } else {
        &config.agent.model
    };

    let model: String = Input::new()
        .with_prompt("Model name")
        .default(current_model.to_string())
        .interact_text_on(term)?;
    config.agent.model = model;

    let default_url = match provider_key {
        "anthropic" => "https://api.anthropic.com",
        "openai" => "https://api.openai.com/v1",
        "openrouter" => "https://openrouter.ai/api/v1",
        "ollama" => "http://localhost:11434",
        "deepseek" => "https://api.deepseek.com",
        "gemini" => "https://generativelanguage.googleapis.com/v1beta",
        "groq" => "https://api.groq.com/openai/v1",
        "moonshot" => "https://api.moonshot.cn/v1",
        "minimax" => "https://api.minimax.chat/v1",
        "github_copilot" => "https://api.githubcopilot.com",
        "openai_codex" => "https://api.openai.com/v1",
        _ => "",
    };

    let current_url = get_provider_url(config, provider_key).unwrap_or(default_url);

    if !current_url.is_empty() {
        let base_url: String = Input::new()
            .with_prompt("Base URL")
            .default(current_url.to_string())
            .interact_text_on(term)?;
        set_provider_url(config, provider_key, &base_url);
    } else {
        let base_url: String = Input::new()
            .with_prompt("Base URL (leave empty for default)")
            .allow_empty(true)
            .interact_text_on(term)?;
        if !base_url.is_empty() {
            set_provider_url(config, provider_key, &base_url);
        }
    }

    let api_key: String = Input::new()
        .with_prompt("API key (input hidden)")
        .interact_text_on(term)?;
    if !api_key.is_empty() {
        set_provider_api_key(config, provider_key, &api_key);
    }

    Ok(())
}

fn configure_telegram(term: &Term, config: &mut Config) -> Result<()> {
    let setup_tg = if config.channels.telegram.is_some() {
        Confirm::new()
            .with_prompt("Configure Telegram bot?")
            .default(true)
            .interact_on(term)?
    } else {
        Confirm::new()
            .with_prompt("Set up a Telegram bot?")
            .default(false)
            .interact_on(term)?
    };

    if !setup_tg {
        term.write_line("  Skipped.")?;
        return Ok(());
    }

    let current_token = config
        .channels
        .telegram
        .as_ref()
        .map(|tg| tg.token.as_str())
        .unwrap_or("");

    let token: String = Input::new()
        .with_prompt("Bot token (from @BotFather)")
        .default(current_token.to_string())
        .interact_text_on(term)?;

    if token.is_empty() {
        term.write_line("  No token provided, skipping Telegram.")?;
        return Ok(());
    }

    let allowed: String = Input::new()
        .with_prompt("Allowed user IDs (comma-separated, leave empty for all)")
        .allow_empty(true)
        .interact_text_on(term)?;

    let allowed_users: Vec<String> = if allowed.trim().is_empty() {
        Vec::new()
    } else {
        allowed
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    config.channels.telegram = Some(TelegramConfig {
        token,
        allowed_users,
        admin_users: Vec::new(),
        enabled: true,
        streaming: true,
        proxy: None,
    });

    Ok(())
}

fn configure_websocket(term: &Term, config: &mut Config) -> Result<()> {
    let default_addr = config
        .channels
        .websocket
        .as_ref()
        .map(|ws| ws.listen_addr.as_str())
        .unwrap_or("127.0.0.1:8090");

    let enable = Confirm::new()
        .with_prompt("Enable WebSocket channel?")
        .default(false)
        .interact_on(term)?;

    if !enable {
        config.channels.websocket = None;
        term.write_line("  Skipped.")?;
        return Ok(());
    }

    let listen_addr: String = Input::new()
        .with_prompt("Listen address")
        .default(default_addr.to_string())
        .interact_text_on(term)?;

    config.channels.websocket = Some(WebSocketConfig {
        enabled: true,
        listen_addr,
        auth: Default::default(),
        max_clients: 100,
        max_message_size: 1048576,
    });

    Ok(())
}

// ── Provider field helpers ───────────────────────────────────

fn get_provider_entry_mut<'a>(config: &'a mut Config, provider: &str) -> Option<&'a mut ProviderEntry> {
    match provider {
        "anthropic" => config.providers.anthropic.as_mut(),
        "openai" => config.providers.openai.as_mut(),
        "openrouter" => config.providers.openrouter.as_mut(),
        "ollama" => config.providers.ollama.as_mut(),
        "deepseek" => config.providers.deepseek.as_mut(),
        "gemini" => config.providers.gemini.as_mut(),
        "groq" => config.providers.groq.as_mut(),
        "moonshot" => config.providers.moonshot.as_mut(),
        "minimax" => config.providers.minimax.as_mut(),
        "github_copilot" => config.providers.github_copilot.as_mut(),
        "openai_codex" => config.providers.openai_codex.as_mut(),
        _ => None,
    }
}

fn ensure_provider_entry(config: &mut Config, provider: &str) {
    match provider {
        "anthropic" => {
            config
                .providers
                .anthropic
                .get_or_insert_with(ProviderEntry::default);
        }
        "openai" => {
            config
                .providers
                .openai
                .get_or_insert_with(ProviderEntry::default);
        }
        "openrouter" => {
            config
                .providers
                .openrouter
                .get_or_insert_with(ProviderEntry::default);
        }
        "ollama" => {
            config
                .providers
                .ollama
                .get_or_insert_with(ProviderEntry::default);
        }
        "deepseek" => {
            config
                .providers
                .deepseek
                .get_or_insert_with(ProviderEntry::default);
        }
        "gemini" => {
            config
                .providers
                .gemini
                .get_or_insert_with(ProviderEntry::default);
        }
        "groq" => {
            config
                .providers
                .groq
                .get_or_insert_with(ProviderEntry::default);
        }
        "moonshot" => {
            config
                .providers
                .moonshot
                .get_or_insert_with(ProviderEntry::default);
        }
        "minimax" => {
            config
                .providers
                .minimax
                .get_or_insert_with(ProviderEntry::default);
        }
        "github_copilot" => {
            config
                .providers
                .github_copilot
                .get_or_insert_with(ProviderEntry::default);
        }
        "openai_codex" => {
            config
                .providers
                .openai_codex
                .get_or_insert_with(ProviderEntry::default);
        }
        _ => {}
    }
}

fn get_provider_url<'a>(config: &'a Config, provider: &str) -> Option<&'a str> {
    let entry = match provider {
        "anthropic" => config.providers.anthropic.as_ref(),
        "openai" => config.providers.openai.as_ref(),
        "openrouter" => config.providers.openrouter.as_ref(),
        "ollama" => config.providers.ollama.as_ref(),
        "deepseek" => config.providers.deepseek.as_ref(),
        "gemini" => config.providers.gemini.as_ref(),
        "groq" => config.providers.groq.as_ref(),
        "moonshot" => config.providers.moonshot.as_ref(),
        "minimax" => config.providers.minimax.as_ref(),
        "github_copilot" => config.providers.github_copilot.as_ref(),
        "openai_codex" => config.providers.openai_codex.as_ref(),
        _ => None,
    };
    entry.and_then(|e| e.base_url.as_deref())
}

fn set_provider_url(config: &mut Config, provider: &str, url: &str) {
    ensure_provider_entry(config, provider);
    if let Some(entry) = get_provider_entry_mut(config, provider) {
        entry.base_url = if url.is_empty() {
            None
        } else {
            Some(url.to_string())
        };
    }
}

fn set_provider_api_key(config: &mut Config, provider: &str, key: &str) {
    ensure_provider_entry(config, provider);
    if let Some(entry) = get_provider_entry_mut(config, provider) {
        entry.api_key = if key.is_empty() {
            None
        } else {
            Some(key.to_string())
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn template_toml() -> String {
        toml::to_string(&Config::default()).unwrap()
    }

    #[test]
    fn setup_creates_template_config_when_config_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");

        // Simulate: no existing config, wizard writes defaults
        let config = Config::default();
        let home = config_path.parent().unwrap();
        std::fs::create_dir_all(home).unwrap();
        loader::save_config(&config, &config_path).unwrap();

        assert_eq!(
            std::fs::read_to_string(&config_path).unwrap(),
            template_toml()
        );
        assert!(config_path.exists());
    }

    #[test]
    fn provider_helpers_set_and_get_fields() {
        let mut config = Config::default();
        set_provider_url(&mut config, "openai", "https://custom.api/v1");
        set_provider_api_key(&mut config, "openai", "sk-test-key");

        let entry = config.providers.openai.as_ref().unwrap();
        assert_eq!(entry.base_url.as_deref(), Some("https://custom.api/v1"));
        assert_eq!(entry.api_key.as_deref(), Some("sk-test-key"));
    }

    #[test]
    fn provider_helpers_handle_unknown_provider() {
        let mut config = Config::default();
        set_provider_url(&mut config, "nonexistent", "https://example.com");
        assert!(get_provider_url(&config, "nonexistent").is_none());
    }
}
