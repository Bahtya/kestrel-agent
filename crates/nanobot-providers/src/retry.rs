//! Retry logic with exponential backoff for provider API calls.
//!
//! Handles 429 (rate-limited) and transient server errors (5xx) by retrying
//! requests with increasing delays and optional `Retry-After` header respect.

use std::time::Duration;
use tracing::warn;
use tokio::time::sleep;

/// Default maximum number of retry attempts.
pub const DEFAULT_MAX_RETRIES: u32 = 3;

/// Default initial backoff duration.
const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Maximum backoff cap to prevent unreasonably long waits.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Configuration for retry behaviour.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 = no retries).
    pub max_retries: u32,
    /// Initial backoff duration.
    pub initial_backoff: Duration,
    /// Whether to retry on server errors (5xx).
    pub retry_on_server_error: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_backoff: DEFAULT_INITIAL_BACKOFF,
            retry_on_server_error: true,
        }
    }
}

impl RetryConfig {
    /// Create a config with no retries.
    pub fn no_retries() -> Self {
        Self {
            max_retries: 0,
            initial_backoff: DEFAULT_INITIAL_BACKOFF,
            retry_on_server_error: false,
        }
    }

    /// Create a config with a custom max retry count.
    pub fn with_max_retries(mut self, max: u32) -> Self {
        self.max_retries = max;
        self
    }
}

/// Decide whether a status code is retryable.
pub fn is_retryable_status(status: u16) -> bool {
    status == 429 || (status >= 500 && status < 600)
}

/// Extract the `Retry-After` header value in seconds.
/// Returns `None` if the header is absent or unparseable.
pub fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let val = headers.get("retry-after")?.to_str().ok()?;
    // Try parsing as seconds first.
    if let Ok(secs) = val.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    // Could parse as HTTP-date but we'll skip that complexity.
    None
}

/// Compute the backoff duration for a given attempt (0-indexed).
pub fn backoff_duration(initial: Duration, attempt: u32) -> Duration {
    // Exponential: initial * 2^attempt, capped at MAX_BACKOFF.
    let millis = initial.as_millis() as u64;
    let factor = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    let delay_millis = (millis * factor).min(MAX_BACKOFF.as_millis() as u64);
    Duration::from_millis(delay_millis)
}

/// Execute an async operation with retry and exponential backoff.
///
/// The closure receives the current attempt number (0-indexed).
/// On retryable failures (determined by the caller), retries up to `config.max_retries`.
pub async fn retry_with_backoff<F, Fut, T>(
    config: &RetryConfig,
    op: F,
) -> anyhow::Result<T>
where
    F: Fn(u32) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut attempt = 0u32;
    loop {
        match op(attempt).await {
            Ok(val) => return Ok(val),
            Err(err) => {
                let retries_left = config.max_retries.saturating_sub(attempt);
                if retries_left == 0 || !is_retryable_err(&err) {
                    return Err(err);
                }
                let delay = backoff_duration(config.initial_backoff, attempt);
                warn!(
                    attempt = attempt + 1,
                    max_retries = config.max_retries,
                    ?delay,
                    "Retrying after error: {}",
                    err
                );
                sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

/// Check if an error is retryable (rate limit or server error).
/// The error message is expected to contain the status code from our provider layer.
fn is_retryable_err(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    // Match the error format: "API error (429): ..." or "Anthropic API error (429): ..."
    if let Some(code) = extract_status_code(&msg) {
        return is_retryable_status(code);
    }
    // Also retry on connection errors (network transient).
    msg.contains("connection") || msg.contains("timeout") || msg.contains("refused")
}

/// Extract HTTP status code from an error message like "API error (429): ..."
fn extract_status_code(msg: &str) -> Option<u16> {
    let start = msg.find('(')?;
    let end = msg.find(')').filter(|e| *e > start)?;
    let code_str = &msg[start + 1..end];
    code_str.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_retryable_status() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(599));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(403));
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(200));
    }

    #[test]
    fn test_backoff_duration() {
        let initial = Duration::from_secs(1);
        assert_eq!(backoff_duration(initial, 0), Duration::from_secs(1));
        assert_eq!(backoff_duration(initial, 1), Duration::from_secs(2));
        assert_eq!(backoff_duration(initial, 2), Duration::from_secs(4));
        assert_eq!(backoff_duration(initial, 3), Duration::from_secs(8));
        // Capped at MAX_BACKOFF
        assert_eq!(backoff_duration(initial, 20), MAX_BACKOFF);
    }

    #[test]
    fn test_parse_retry_after_seconds() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "30".parse().unwrap());
        assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(30)));
    }

    #[test]
    fn test_parse_retry_after_missing() {
        let headers = reqwest::header::HeaderMap::new();
        assert_eq!(parse_retry_after(&headers), None);
    }

    #[test]
    fn test_parse_retry_after_invalid() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "not-a-number".parse().unwrap());
        assert_eq!(parse_retry_after(&headers), None);
    }

    #[test]
    fn test_extract_status_code() {
        assert_eq!(extract_status_code("API error (429): rate limited"), Some(429));
        assert_eq!(extract_status_code("Anthropic API error (503): unavailable"), Some(503));
        assert_eq!(extract_status_code("API error (401): unauthorized"), Some(401));
        assert_eq!(extract_status_code("Some other error"), None);
    }

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_backoff, Duration::from_secs(1));
        assert!(config.retry_on_server_error);
    }

    #[test]
    fn test_retry_config_no_retries() {
        let config = RetryConfig::no_retries();
        assert_eq!(config.max_retries, 0);
        assert!(!config.retry_on_server_error);
    }

    #[test]
    fn test_retry_config_with_max_retries() {
        let config = RetryConfig::default().with_max_retries(5);
        assert_eq!(config.max_retries, 5);
    }

    #[tokio::test]
    async fn test_retry_succeeds_on_first_try() {
        let config = RetryConfig::default();
        let result = retry_with_backoff(&config, |_attempt| async {
            Ok::<_, anyhow::Error>(42)
        })
        .await
        .unwrap();
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_retries() {
        let config = RetryConfig::default().with_max_retries(3);
        let mut calls = 0u32;
        let result = retry_with_backoff(&config, move |_attempt| {
            let mut calls_inner = calls;
            async move {
                calls_inner += 1;
                calls = calls_inner;
                if calls_inner < 3 {
                    Err(anyhow::anyhow!("API error (429): rate limited"))
                } else {
                    Ok::<_, anyhow::Error>("success")
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(result, "success");
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let config = RetryConfig::default().with_max_retries(2);
        let result = retry_with_backoff(&config, |_attempt| async {
            Err::<(), _>(anyhow::anyhow!("API error (429): rate limited"))
        })
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("429"));
    }

    #[tokio::test]
    async fn test_retry_non_retryable_error() {
        let config = RetryConfig::default();
        let mut calls = 0u32;
        let result = retry_with_backoff(&config, move |_attempt| {
            async move {
                calls += 1;
                Err::<(), _>(anyhow::anyhow!("API error (401): unauthorized"))
            }
        })
        .await;
        assert!(result.is_err());
        // Should not retry on 401
        assert_eq!(calls, 1);
    }
}
