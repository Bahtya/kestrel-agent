//! Standalone Python kestrel config migration tool.
//!
//! Reads a Python kestrel `config.json` (plus per-channel configs) and converts
//! it to kestrel `config.toml`.
//!
//! # Usage
//!
//! ```sh
//! # Dry run — print TOML to stdout
//! cargo run -p kestrel-config --example migrate -- --from ~/.kestrel --dry-run
//!
//! # Write to auto-detected config path (~/.kestrel/config.toml)
//! cargo run -p kestrel-config --example migrate -- --from ~/.kestrel
//!
//! # Write to a specific output file
//! cargo run -p kestrel-config --example migrate -- --from ~/.kestrel --output ./config.toml
//! ```

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

/// Migrate Python kestrel config to kestrel TOML format.
#[derive(Parser)]
#[command(
    name = "migrate",
    about = "Migrate Python kestrel config to kestrel format"
)]
struct Cli {
    /// Path to Python kestrel config directory (e.g. ~/.kestrel).
    #[arg(long)]
    from: PathBuf,

    /// Output path for config.toml.
    /// Defaults to auto-detected path (~/.kestrel/config.toml).
    #[arg(long)]
    output: Option<PathBuf>,

    /// Dry run: print TOML to stdout instead of writing to file.
    #[arg(long)]
    dry_run: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let from = &cli.from;
    anyhow::ensure!(
        from.exists(),
        "Source directory does not exist: {}",
        from.display()
    );

    eprintln!("Migrating Python kestrel config from: {}", from.display());

    let opts = kestrel_config::MigrationOptions {
        dry_run: cli.dry_run,
        output_file: cli.output.clone(),
        ..Default::default()
    };

    let result = kestrel_config::migrate_from_python(from, &opts)
        .context("Failed to migrate Python config")?;

    // Print migration report to stderr so stdout stays clean for --dry-run
    print_report(&result.report);

    let toml_str = toml::to_string(&result.config).context("Failed to serialize config to TOML")?;

    if cli.dry_run {
        println!("{}", toml_str);
    } else {
        let output_path = match cli.output {
            Some(ref p) => p.clone(),
            None => kestrel_config::paths::get_config_path()?,
        };
        // Ensure parent directory exists
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }
        std::fs::write(&output_path, &toml_str)
            .with_context(|| format!("Failed to write config to {}", output_path.display()))?;
        eprintln!("\nConfig written to: {}", output_path.display());
    }

    Ok(())
}

/// Print a human-readable migration report to stderr.
fn print_report(report: &kestrel_config::MigrationReport) {
    if !report.mapped.is_empty() {
        eprintln!("\nMapped fields ({}):", report.mapped.len());
        for field in &report.mapped {
            eprintln!("  [OK] {}", field);
        }
    }

    if !report.notes.is_empty() {
        eprintln!("\nNotes ({}):", report.notes.len());
        for note in &report.notes {
            eprintln!("  [NOTE] {}", note);
        }
    }

    if !report.unmapped.is_empty() {
        eprintln!("\nUnmapped fields ({}):", report.unmapped.len());
        for field in &report.unmapped {
            eprintln!("  [SKIP] {}", field);
        }
    }

    eprintln!(
        "\nSummary: {} mapped, {} unmapped, {} notes",
        report.mapped.len(),
        report.unmapped.len(),
        report.notes.len()
    );
}
