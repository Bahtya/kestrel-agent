//! Doctor command — comprehensive system diagnostics.

use anyhow::Result;
use kestrel_config::paths::get_config_path;
use kestrel_config::schema::Config;
use kestrel_config::validate;
use kestrel_providers::{CompletionRequest, ProviderRegistry};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// Run all diagnostic checks.
pub async fn run(config: &Config) -> Result<()> {
    println!("=== Kestrel Doctor ===\n");

    let mut errors = 0usize;
    let mut warnings = 0usize;

    check_config_file(config, &mut errors, &mut warnings);
    check_websocket(config, &mut errors, &mut warnings);
    check_providers(config, &mut errors, &mut warnings).await;
    check_telegram(config, &mut errors, &mut warnings).await;

    println!("\n--- Summary ---");
    if errors == 0 && warnings == 0 {
        println!("All checks passed.");
    } else {
        if errors > 0 {
            println!("{} error(s) found.", errors);
        }
        if warnings > 0 {
            println!("{} warning(s) found.", warnings);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 1. Config file validation
// ---------------------------------------------------------------------------

fn check_config_file(config: &Config, errors: &mut usize, warnings: &mut usize) {
    println!("[1/4] Config file validation");

    // Check that config file exists
    match get_config_path() {
        Ok(path) => {
            if path.exists() {
                println!("  Config path:    {}", path.display());
                // Try to re-parse the raw file
                match std::fs::read_to_string(&path) {
                    Ok(raw) => match toml::from_str::<Config>(&raw) {
                        Ok(_) => println!("  Parse:          pass"),
                        Err(e) => {
                            *errors += 1;
                            println!("  Parse:          FAIL — {}", e);
                        }
                    },
                    Err(e) => {
                        *errors += 1;
                        println!("  Read:           FAIL — {}", e);
                    }
                }
            } else {
                *warnings += 1;
                println!(
                    "  Config file:    not found at {} (using defaults)",
                    path.display()
                );
            }
        }
        Err(e) => {
            *errors += 1;
            println!("  Config path:    FAIL — {}", e);
        }
    }

    // Schema validation
    let report = validate::validate(config);
    let err_count = report.errors().len();
    let warn_count = report.warnings().len();
    *errors += err_count;
    *warnings += warn_count;

    if report.is_empty() {
        println!("  Schema:         pass");
    } else {
        for f in report.findings() {
            let label = if f.severity == validate::Severity::Error {
                "FAIL"
            } else {
                "WARN"
            };
            println!(
                "  Schema {:?}: [{}] {} — {}",
                f.severity, f.path, f.message, label
            );
        }
    }

    println!();
}

// ---------------------------------------------------------------------------
// 2. WebSocket port health
// ---------------------------------------------------------------------------

fn check_websocket(config: &Config, errors: &mut usize, warnings: &mut usize) {
    println!("[2/4] WebSocket port health");

    match &config.channels.websocket {
        Some(ws) if ws.enabled => {
            let addr = &ws.listen_addr;
            match addr.parse::<std::net::SocketAddr>() {
                Ok(sock_addr) => {
                    print!("  Port {}: ", addr);
                    match TcpStream::connect_timeout(&sock_addr, Duration::from_secs(5)) {
                        Ok(_) => println!("listening"),
                        Err(e) => {
                            *errors += 1;
                            println!("FAIL — {}", e);
                        }
                    }
                }
                Err(e) => {
                    *warnings += 1;
                    println!("  Port {}: FAIL — invalid address: {}", addr, e);
                }
            }
        }
        _ => {
            // WebSocket not configured or disabled — check if anything is listening anyway
            let default_addr = config
                .channels
                .websocket
                .as_ref()
                .map(|ws| ws.listen_addr.as_str())
                .unwrap_or("127.0.0.1:8090");
            print!("  Port {} (disabled): ", default_addr);
            match default_addr.parse::<std::net::SocketAddr>() {
                Ok(sock_addr) => {
                    match TcpStream::connect_timeout(
                        &sock_addr,
                        Duration::from_secs(3),
                    ) {
                        Ok(_) => println!("listening (but disabled in config)"),
                        Err(_) => println!("nothing listening"),
                    }
                }
                Err(_) => println!("invalid address"),
            }
        }
    }

    println!();
}

// ---------------------------------------------------------------------------
// 3. Provider model availability
// ---------------------------------------------------------------------------

async fn check_providers(config: &Config, errors: &mut usize, warnings: &mut usize) {
    println!("[3/4] Provider model availability");

    let registry = match ProviderRegistry::from_config(config) {
        Ok(r) => r,
        Err(e) => {
            *errors += 1;
            println!("  Registry:       FAIL — {}", e);
            println!();
            return;
        }
    };

    let names = registry.provider_names();
    if names.is_empty() {
        *warnings += 1;
        println!("  No providers configured.");
        println!();
        return;
    }

    for name in &names {
        let provider = match registry.get_provider(name) {
            Some(p) => p,
            None => continue,
        };

        let model = provider.default_model();
        let model_display = if model.is_empty() {
            config.agent.model.as_str()
        } else {
            model
        };

        print!("  {} ({}): ", name, model_display);

        let req = CompletionRequest {
            model: model_display.to_string(),
            messages: vec![kestrel_core::Message {
                role: kestrel_core::MessageRole::User,
                content: "hi".to_string(),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                reasoning_content: None,
            }],
            tools: None,
            max_tokens: Some(5),
            temperature: Some(0.0),
            stream: false,
            reasoning_effort: None,
        };

        let start = Instant::now();
        match tokio::time::timeout(Duration::from_secs(30), provider.complete(req)).await {
            Ok(Ok(resp)) => {
                let elapsed = start.elapsed();
                let content_preview = resp.content.as_deref().unwrap_or("(empty)");
                let content_preview: String =
                    content_preview.chars().take(30).collect();
                println!(
                    "pass ({:.1}s) — {}",
                    elapsed.as_secs_f64(),
                    content_preview
                );
            }
            Ok(Err(e)) => {
                *errors += 1;
                // Extract a concise error message
                let msg = format!("{}", e);
                let short = msg.lines().next().unwrap_or(&msg);
                println!("FAIL — {}", short);
            }
            Err(_) => {
                *errors += 1;
                println!("FAIL — timeout (30s)");
            }
        }
    }

    println!();
}

// ---------------------------------------------------------------------------
// 4. Telegram API health
// ---------------------------------------------------------------------------

async fn check_telegram(config: &Config, errors: &mut usize, _warnings: &mut usize) {
    println!("[4/4] Telegram API health");

    let tg = match &config.channels.telegram {
        Some(tg) if tg.enabled && !tg.token.is_empty() => tg,
        Some(tg) if !tg.token.is_empty() => {
            println!("  Telegram disabled in config — skipping");
            println!();
            return;
        }
        _ => {
            println!("  Not configured — skipping");
            println!();
            return;
        }
    };

    let url = format!("https://api.telegram.org/bot{}/getMe", tg.token);

    let client = build_telegram_http_client(tg.proxy.as_deref());
    let client = match client {
        Ok(c) => c,
        Err(e) => {
            *errors += 1;
            println!("  HTTP client:    FAIL — {}", e);
            println!();
            return;
        }
    };

    print!("  getMe:          ");
    let start = Instant::now();
    match tokio::time::timeout(Duration::from_secs(15), client.get(&url).send()).await {
        Ok(Ok(resp)) => {
            let elapsed = start.elapsed();
            let status = resp.status();
            if status.is_success() {
                match resp.text().await {
                    Ok(body) => {
                        // Try to extract bot username from response
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
                            let username = val.pointer("/result/username");
                            let username = username
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            println!(
                                "pass ({:.1}s) — bot @{}",
                                elapsed.as_secs_f64(),
                                username
                            );
                        } else {
                            println!("pass ({:.1}s)", elapsed.as_secs_f64());
                        }
                    }
                    Err(_) => println!(
                        "pass ({:.1}s) — could not read body",
                        elapsed.as_secs_f64()
                    ),
                }
            } else {
                *errors += 1;
                if status.as_u16() == 401 {
                    println!("FAIL — unauthorized (invalid bot token)");
                } else {
                    println!("FAIL — HTTP {}", status);
                }
            }
        }
        Ok(Err(e)) => {
            *errors += 1;
            let msg = format!("{}", e);
            let short = msg.lines().next().unwrap_or(&msg);
            println!("FAIL — {}", short);
        }
        Err(_) => {
            *errors += 1;
            println!("FAIL — timeout (15s)");
        }
    }

    println!();
}

fn build_telegram_http_client(
    proxy: Option<&str>,
) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .dns_resolver(kestrel_core::dns::build_dns_resolver());

    if let Some(proxy_url) = proxy {
        if !proxy_url.is_empty() {
            let proxy = reqwest::Proxy::all(proxy_url)?;
            builder = builder.proxy(proxy);
        }
    }

    Ok(builder.build()?)
}
