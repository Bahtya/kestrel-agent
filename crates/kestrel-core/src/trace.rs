//! Trace ID generation for request correlation.

/// Generate a short trace ID for request correlation.
///
/// Format: `tr-{uuid_first_12_hex}` — compact yet globally unique.
pub fn generate_trace_id() -> String {
    let uuid = uuid::Uuid::new_v4();
    format!("tr-{}", &uuid.to_string().replace('-', "")[..12])
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
        let ids: std::collections::HashSet<String> =
            (0..1000).map(|_| generate_trace_id()).collect();
        assert_eq!(ids.len(), 1000, "all trace IDs should be unique");
    }
}
