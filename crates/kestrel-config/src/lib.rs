//! # kestrel-config
//!
//! Configuration loading and schema for kestrel.
//! Uses TOML format for the config file (`config.toml`).

pub mod loader;
pub mod migration;
pub mod paths;
pub mod platform;
pub mod python_migrate;
pub mod python_schema;
pub mod schema;
pub mod validate;

pub use loader::load_config;
pub use python_migrate::{
    migrate_from_python, migrate_from_str, MigrationOptions, MigrationReport, MigrationResult,
};
pub use schema::Config;
pub use validate::{fill_defaults, validate, validate_and_fill, ValidationReport};
