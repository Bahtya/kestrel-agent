//! Path resolution for nanobot data directories.
//!
//! Mirrors the Python config/paths.py module for data, media, cron, and workspace paths.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Get the nanobot home directory.
///
/// Priority: `NANOBOT_RUST_HOME` env var > `~/.nanobot-rust` default.
pub fn get_nanobot_home() -> Result<PathBuf> {
    if let Ok(home) = std::env::var("NANOBOT_RUST_HOME") {
        Ok(PathBuf::from(home))
    } else {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".nanobot-rust"))
    }
}

/// Get the data directory for the current instance.
pub fn get_data_dir() -> Result<PathBuf> {
    let home = get_nanobot_home()?;
    let data_dir = home.join("data");
    ensure_dir(&data_dir)?;
    Ok(data_dir)
}

/// Get the media storage directory.
pub fn get_media_dir(channel: Option<&str>) -> Result<PathBuf> {
    let home = get_nanobot_home()?;
    let media_dir = match channel {
        Some(ch) => home.join("media").join(ch),
        None => home.join("media"),
    };
    ensure_dir(&media_dir)?;
    Ok(media_dir)
}

/// Get the cron jobs storage directory.
pub fn get_cron_dir() -> Result<PathBuf> {
    let home = get_nanobot_home()?;
    let cron_dir = home.join("cron");
    ensure_dir(&cron_dir)?;
    Ok(cron_dir)
}

/// Get the sessions storage directory.
pub fn get_sessions_dir() -> Result<PathBuf> {
    let home = get_nanobot_home()?;
    let sessions_dir = home.join("sessions");
    ensure_dir(&sessions_dir)?;
    Ok(sessions_dir)
}

/// Get the config file path.
///
/// Checks in order:
/// 1. `~/.nanobot-rust/config.yaml` (Rust default)
/// 2. `~/.nanobot/config.yaml` (Python legacy fallback)
pub fn get_config_path() -> Result<PathBuf> {
    let home = get_nanobot_home()?;
    let rust_path = home.join("config.yaml");
    if rust_path.exists() {
        return Ok(rust_path);
    }

    // Fall back to Python nanobot config location
    if let Some(home) = dirs::home_dir() {
        let python_path = home.join(".nanobot").join("config.yaml");
        if python_path.exists() {
            return Ok(python_path);
        }
    }

    // Return the Rust default (may not exist yet, will be created)
    Ok(rust_path)
}

/// Get the memory storage directory.
pub fn get_memory_dir() -> Result<PathBuf> {
    let home = get_nanobot_home()?;
    let memory_dir = home.join("memory");
    ensure_dir(&memory_dir)?;
    Ok(memory_dir)
}

/// Get the skills directory.
pub fn get_skills_dir() -> Result<PathBuf> {
    let home = get_nanobot_home()?;
    let skills_dir = home.join("skills");
    ensure_dir(&skills_dir)?;
    Ok(skills_dir)
}

/// Resolve the workspace path from config or default.
pub fn get_workspace_path(config_workspace: Option<&str>) -> Result<PathBuf> {
    match config_workspace {
        Some(ws) if !ws.is_empty() => Ok(PathBuf::from(ws)),
        _ => {
            let home = get_nanobot_home()?;
            Ok(home.join("workspace"))
        }
    }
}

/// Get the templates directory.
pub fn get_templates_dir() -> Result<PathBuf> {
    let home = get_nanobot_home()?;
    Ok(home.join("templates"))
}

/// Ensure a directory exists, creating it if necessary.
fn ensure_dir(path: &PathBuf) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)
            .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nanobot_home_default() {
        std::env::remove_var("NANOBOT_RUST_HOME");
        let home = get_nanobot_home().unwrap();
        assert!(home.to_string_lossy().ends_with(".nanobot-rust"));
    }

    #[test]
    fn test_nanobot_home_env() {
        std::env::set_var("NANOBOT_RUST_HOME", "/tmp/test-nanobot");
        let home = get_nanobot_home().unwrap();
        assert_eq!(home, PathBuf::from("/tmp/test-nanobot"));
        std::env::remove_var("NANOBOT_RUST_HOME");
    }

    #[test]
    fn test_config_path_fallback() {
        // With a custom home that doesn't exist, it falls back to ~/.nanobot/config.yaml
        // if that exists, otherwise returns the Rust default path.
        std::env::set_var("NANOBOT_RUST_HOME", "/tmp/test-nanobot-config-fallback");
        let path = get_config_path().unwrap();
        // Either the Python fallback or the Rust default is acceptable
        let is_python_fallback = path == dirs::home_dir().unwrap().join(".nanobot").join("config.yaml");
        let is_rust_default = path == PathBuf::from("/tmp/test-nanobot-config-fallback/config.yaml");
        assert!(is_python_fallback || is_rust_default, "unexpected config path: {path:?}");
        std::env::remove_var("NANOBOT_RUST_HOME");
    }
}
