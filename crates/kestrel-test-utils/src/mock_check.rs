//! Mock health check implementations for testing.

use async_trait::async_trait;
use chrono::Local;
use kestrel_heartbeat::types::{CheckStatus, HealthCheck, HealthCheckResult};

/// A health check that returns a fixed healthy/unhealthy status.
pub struct MockCheck {
    name: String,
    healthy: bool,
}

impl MockCheck {
    pub fn new(name: &str, healthy: bool) -> Self {
        Self {
            name: name.to_string(),
            healthy,
        }
    }

    pub fn healthy(name: &str) -> Self {
        Self::new(name, true)
    }

    pub fn unhealthy(name: &str) -> Self {
        Self::new(name, false)
    }
}

#[async_trait]
impl HealthCheck for MockCheck {
    fn component_name(&self) -> &str {
        &self.name
    }

    async fn report_health(&self) -> HealthCheckResult {
        HealthCheckResult {
            component: self.name.clone(),
            status: if self.healthy {
                CheckStatus::Healthy
            } else {
                CheckStatus::Unhealthy
            },
            message: if self.healthy {
                "ok".to_string()
            } else {
                "failing".to_string()
            },
            timestamp: Local::now(),
        }
    }
}

/// A health check that wraps the check with context (healthy/unhealthy string pairs).
pub struct MockHealthCheck {
    component: String,
    status: CheckStatus,
    message: String,
}

impl MockHealthCheck {
    pub fn healthy(component: &str) -> Self {
        Self {
            component: component.to_string(),
            status: CheckStatus::Healthy,
            message: "ok".to_string(),
        }
    }

    pub fn unhealthy(component: &str, message: &str) -> Self {
        Self {
            component: component.to_string(),
            status: CheckStatus::Unhealthy,
            message: message.to_string(),
        }
    }
}

#[async_trait]
impl HealthCheck for MockHealthCheck {
    fn component_name(&self) -> &str {
        &self.component
    }

    async fn report_health(&self) -> HealthCheckResult {
        HealthCheckResult {
            component: self.component.clone(),
            status: self.status.clone(),
            message: self.message.clone(),
            timestamp: Local::now(),
        }
    }
}
