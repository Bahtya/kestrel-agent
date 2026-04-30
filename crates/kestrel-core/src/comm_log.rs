//! Communication log helpers — thin wrappers around `tracing` macros.
//!
//! All methods emit events with `target: "comm"` so the logging subscriber
//! can route them to a separate file. The `trace_id` field is always present.

use std::collections::HashMap;
use tracing_appender::non_blocking::WorkerGuard;

/// Guard returned by [`setup_comm_log`]. Must be kept alive for the
/// application lifetime — dropping flushes remaining log lines.
pub struct CommLogGuard {
    _guard: WorkerGuard,
}

/// Set up a dedicated non-blocking writer for `comm.log`.
///
/// Returns a guard that must be held for the application's lifetime.
pub fn setup_comm_log(log_dir: &str) -> anyhow::Result<CommLogGuard> {
    let log_path = std::path::Path::new(log_dir);
    std::fs::create_dir_all(log_path)?;

    let file_appender = tracing_appender::rolling::daily(log_path, "comm.log");
    let (_non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    Ok(CommLogGuard { _guard: guard })
}

/// Sanitize sensitive headers before logging.
pub fn sanitize_headers(headers: &HashMap<String, String>) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(k, v)| {
            let lower = k.to_lowercase();
            if lower == "authorization" || lower == "x-api-key" || lower.contains("token") {
                (k.clone(), "***".to_string())
            } else {
                (k.clone(), v.clone())
            }
        })
        .collect()
}

/// Log an HTTP request via tracing.
pub fn log_http_request(
    trace_id: &str,
    method: &str,
    url: &str,
    headers: &HashMap<String, String>,
    body: &serde_json::Value,
    level: &str,
) {
    let sanitized = sanitize_headers(headers);
    match level {
        "trace" => tracing::trace!(
            target: "comm",
            trace_id = %trace_id,
            method = %method,
            url = %url,
            headers = ?sanitized,
            body = %body,
            "HTTP REQ"
        ),
        "debug" => tracing::debug!(
            target: "comm",
            trace_id = %trace_id,
            method = %method,
            url = %url,
            headers = ?sanitized,
            "HTTP REQ"
        ),
        _ => tracing::info!(
            target: "comm",
            trace_id = %trace_id,
            method = %method,
            url = %url,
            "HTTP REQ"
        ),
    }
}

/// Log an HTTP response via tracing.
pub fn log_http_response(
    trace_id: &str,
    status: u16,
    body_summary: &str,
    duration_ms: u64,
    level: &str,
) {
    match level {
        "trace" => tracing::trace!(
            target: "comm",
            trace_id = %trace_id,
            status = status,
            duration_ms = duration_ms,
            body = %body_summary,
            "HTTP RESP"
        ),
        "debug" => tracing::debug!(
            target: "comm",
            trace_id = %trace_id,
            status = status,
            duration_ms = duration_ms,
            "HTTP RESP"
        ),
        _ => tracing::info!(
            target: "comm",
            trace_id = %trace_id,
            status = status,
            duration_ms = duration_ms,
            "HTTP RESP"
        ),
    }
}

/// Log a WebSocket inbound message.
pub fn log_ws_inbound(
    trace_id: &str,
    client_id: &str,
    msg_type: &str,
    payload: &str,
    level: &str,
) {
    match level {
        "trace" => tracing::trace!(
            target: "comm",
            trace_id = %trace_id,
            client_id = %client_id,
            msg_type = %msg_type,
            payload = %payload,
            "WS IN"
        ),
        "debug" => tracing::debug!(
            target: "comm",
            trace_id = %trace_id,
            client_id = %client_id,
            msg_type = %msg_type,
            "WS IN"
        ),
        _ => tracing::info!(
            target: "comm",
            trace_id = %trace_id,
            client_id = %client_id,
            msg_type = %msg_type,
            "WS IN"
        ),
    }
}

/// Log a WebSocket outbound message.
pub fn log_ws_outbound(
    trace_id: &str,
    client_id: &str,
    msg_type: &str,
    payload: &str,
    level: &str,
) {
    match level {
        "trace" => tracing::trace!(
            target: "comm",
            trace_id = %trace_id,
            client_id = %client_id,
            msg_type = %msg_type,
            payload = %payload,
            "WS OUT"
        ),
        "debug" => tracing::debug!(
            target: "comm",
            trace_id = %trace_id,
            client_id = %client_id,
            msg_type = %msg_type,
            "WS OUT"
        ),
        _ => tracing::info!(
            target: "comm",
            trace_id = %trace_id,
            client_id = %client_id,
            msg_type = %msg_type,
            "WS OUT"
        ),
    }
}

/// Log a tool call start.
pub fn log_tool_call(
    trace_id: &str,
    tool_name: &str,
    params: &serde_json::Value,
    level: &str,
) {
    match level {
        "trace" => tracing::trace!(
            target: "comm",
            trace_id = %trace_id,
            tool = %tool_name,
            params = %params,
            "TOOL START"
        ),
        "debug" => tracing::debug!(
            target: "comm",
            trace_id = %trace_id,
            tool = %tool_name,
            params = %params,
            "TOOL START"
        ),
        _ => tracing::info!(
            target: "comm",
            trace_id = %trace_id,
            tool = %tool_name,
            "TOOL START"
        ),
    }
}

/// Log a tool call result.
pub fn log_tool_result(
    trace_id: &str,
    tool_name: &str,
    result_summary: &str,
    duration_ms: u64,
    success: bool,
    level: &str,
) {
    match level {
        "trace" => tracing::trace!(
            target: "comm",
            trace_id = %trace_id,
            tool = %tool_name,
            duration_ms = duration_ms,
            success = success,
            result = %result_summary,
            "TOOL END"
        ),
        "debug" => tracing::debug!(
            target: "comm",
            trace_id = %trace_id,
            tool = %tool_name,
            duration_ms = duration_ms,
            success = success,
            "TOOL END"
        ),
        _ => tracing::info!(
            target: "comm",
            trace_id = %trace_id,
            tool = %tool_name,
            duration_ms = duration_ms,
            success = success,
            "TOOL END"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_headers_masks_authorization() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer secret".to_string());
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        let sanitized = sanitize_headers(&headers);
        assert_eq!(sanitized.get("Authorization").unwrap(), "***");
        assert_eq!(
            sanitized.get("Content-Type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_sanitize_headers_masks_x_api_key() {
        let mut headers = HashMap::new();
        headers.insert("x-api-key".to_string(), "sk-123".to_string());
        let sanitized = sanitize_headers(&headers);
        assert_eq!(sanitized.get("x-api-key").unwrap(), "***");
    }

    #[test]
    fn test_sanitize_headers_masks_token_fields() {
        let mut headers = HashMap::new();
        headers.insert("X-Auth-Token".to_string(), "abc".to_string());
        let sanitized = sanitize_headers(&headers);
        assert_eq!(sanitized.get("X-Auth-Token").unwrap(), "***");
    }

    #[test]
    fn test_sanitize_headers_preserves_safe_headers() {
        let mut headers = HashMap::new();
        headers.insert("Accept".to_string(), "application/json".to_string());
        let sanitized = sanitize_headers(&headers);
        assert_eq!(sanitized.get("Accept").unwrap(), "application/json");
    }
}
