//! Communication log helpers.
//!
//! Provides header sanitization for comm-log events. Actual logging is done
//! via `tracing::info!(target: "comm", ...)` in provider/ws/tool code.

use std::collections::HashMap;

/// Sanitize sensitive headers before logging.
///
/// Masks values for `Authorization`, `x-api-key`, and any header whose
/// lowercase name contains "token".
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
        assert_eq!(sanitized.get("Content-Type").unwrap(), "application/json");
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
