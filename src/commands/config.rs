//! Config subcommand — validate, migrate, import, and export.

use anyhow::{Context, Result};
use kestrel_config::Config;
use std::path::Path;
use tracing::info;

/// Run config validation.
pub fn validate(config: &Config) -> Result<()> {
    info!("Validating configuration...");

    let report = kestrel_config::validate(config);

    if report.is_empty() {
        println!("Configuration is valid. No issues found.");
        return Ok(());
    }

    let num_errors = report.errors().len();
    let num_warnings = report.warnings().len();

    let warnings = report.warnings();
    if !warnings.is_empty() {
        println!("Warnings ({}):", warnings.len());
        for w in &warnings {
            println!("  {}", w);
        }
    }

    let errors = report.errors();
    if !errors.is_empty() {
        println!("Errors ({}):", errors.len());
        for e in &errors {
            println!("  {}", e);
        }
        println!(
            "\nConfiguration has {} error(s). Fix them before running.",
            num_errors
        );
        std::process::exit(1);
    }

    println!("\nConfiguration is valid with {} warning(s).", num_warnings);
    Ok(())
}

/// Run Python kestrel config migration.
pub fn migrate(from: &Path, dry_run: bool) -> Result<()> {
    info!("Migrating Python kestrel config from: {}", from.display());

    let opts = kestrel_config::MigrationOptions {
        dry_run,
        ..Default::default()
    };

    let result = kestrel_config::migrate_from_python(from, &opts)?;

    // Print migration report
    if !result.report.mapped.is_empty() {
        println!("Mapped fields ({}):", result.report.mapped.len());
        for field in &result.report.mapped {
            println!("  [OK] {}", field);
        }
    }

    if !result.report.notes.is_empty() {
        println!("\nNotes ({}):", result.report.notes.len());
        for note in &result.report.notes {
            println!("  [NOTE] {}", note);
        }
    }

    if !result.report.unmapped.is_empty() {
        println!("\nUnmapped fields ({}):", result.report.unmapped.len());
        for field in &result.report.unmapped {
            println!("  [SKIP] {}", field);
        }
    }

    if dry_run {
        println!("\n--- Generated config.toml (dry run) ---\n");
        let toml_str = toml::to_string(&result.config)?;
        println!("{}", toml_str);
    } else {
        let config_path = kestrel_config::paths::get_config_path()?;
        println!("\nWriting config to: {}", config_path.display());
        kestrel_config::loader::save_config(&result.config, &config_path)?;
        println!("Migration complete.");
    }

    Ok(())
}

/// Import encrypted config from a URL, decrypt, validate, and save.
pub async fn import(url: &str, password: &str) -> Result<()> {
    info!("Downloading encrypted config from: {url}");

    let response = reqwest::get(url)
        .await
        .with_context(|| format!("Failed to download from {url}"))?;

    if !response.status().is_success() {
        anyhow::bail!("HTTP {} from {}", response.status(), url);
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read response body")?;

    info!("Downloaded {} bytes, decrypting...", bytes.len());

    let toml_str = kestrel_config::crypto::decrypt(&bytes, password)
        .context("Decryption failed — wrong password or corrupted data")?;

    // Validate TOML by parsing into Config
    let config: Config = toml::from_str(&toml_str).context("Decrypted data is not valid TOML")?;

    // Run schema validation
    let report = kestrel_config::validate(&config);
    if !report.errors().is_empty() {
        println!("Decrypted config has validation errors:");
        for e in report.errors() {
            println!("  {}", e);
        }
        anyhow::bail!("Import aborted — config has validation errors");
    }

    let config_path = kestrel_config::paths::get_config_path()?;
    println!("Saving config to: {}", config_path.display());
    kestrel_config::loader::save_config(&config, &config_path)?;

    println!("Import complete.");
    Ok(())
}

/// Export current config: encrypt with password and write to file.
pub fn export(config: &Config, password: &str, output: &Path) -> Result<()> {
    let toml_str = toml::to_string(config)?;
    let encrypted = kestrel_config::crypto::encrypt(&toml_str, password)?;

    std::fs::write(output, &encrypted)
        .with_context(|| format!("Failed to write to {}", output.display()))?;

    println!("Exported encrypted config to: {}", output.display());
    Ok(())
}
