//! Trace ID generation and context propagation.

use serde::{Deserialize, Serialize};

/// Generate a short trace ID for request correlation.
///
/// Format: `tr-{uuid_first_12_hex}` — compact yet globally unique.
pub fn generate_trace_id() -> String {
    let uuid = uuid::Uuid::new_v4();
    format!("tr-{}", &uuid.to_string().replace('-', "")[..12])
}

/// Trace context carried through the request lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceContext {
    pub trace_id: String,
    pub parent_span_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_trace_id_format() {
        let id = generate_trace_id();
        assert!(id.starts_with("tr-"));
        assert_eq!(id.len(), 15); // "tr-" + 12 hex chars
    }

    #[test]
    fn test_generate_trace_id_uniqueness() {
        let ids: std::collections::HashSet<String> = (0..1000)
            .map(|_| generate_trace_id())
            .collect();
        assert_eq!(ids.len(), 1000, "all trace IDs should be unique");
    }

    #[test]
    fn test_trace_context_serialization() {
        let ctx = TraceContext {
            trace_id: "tr-abc123def456".to_string(),
            parent_span_id: Some("span-1".to_string()),
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let parsed: TraceContext = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.trace_id, "tr-abc123def456");
        assert_eq!(parsed.parent_span_id, Some("span-1".to_string()));
    }
}
