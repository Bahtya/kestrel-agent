//! # nanobot-heartbeat
//!
//! Heartbeat service — periodic health checks, auto-restart, and state persistence.
//!
//! Components implement the `HealthCheck` trait and register with the
//! `HealthCheckRegistry`. The `HeartbeatService` polls all registered
//! components periodically, tracks consecutive failures, and triggers
//! automatic restarts with exponential backoff via the message bus.

pub mod service;
pub mod types;

pub use service::HeartbeatService;
pub use types::*;
