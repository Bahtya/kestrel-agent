//! Provider registry — resolves model names to appropriate providers.
//!
//! Matches the Python `providers/registry.py` keyword-based model matching.

use crate::anthropic::{AnthropicConfig, AnthropicProvider};
use crate::base::LlmProvider;
use crate::openai_compat::{OpenAiCompatConfig, OpenAiCompatProvider};
use anyhow::Result;
use kestrel_config::schema::Config;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{debug, info};

/// Model keyword to provider name mapping.
/// This mirrors the Python registry's MODEL_KEYWORDS map.
const MODEL_KEYWORD_MAP: &[(&str, &str)] = &[
    ("claude", "anthropic"),
    ("anthropic", "anthropic"),
    ("gpt", "openai"),
    ("o1", "openai"),
    ("o3", "openai"),
    ("o4", "openai"),
    ("chatgpt", "openai"),
    ("deepseek-v4", "openrouter"),
    ("deepseek", "deepseek"),
    ("gemini", "gemini"),
    ("groq", "groq"),
    ("moonshot", "moonshot"),
    ("kimi", "moonshot"),
    ("minimax", "minimax"),
    ("llama", "ollama"),
    ("mistral", "ollama"),
    ("qwen", "ollama"),
    ("codestral", "ollama"),
    ("opencode-go", "opencode_go"),
    ("opencode_go", "opencode_go"),
];

/// Registry of LLM providers.
#[derive(Clone)]
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    default_provider: Option<String>,
    /// Semaphore limiting concurrent in-flight provider requests.
    concurrency_limit: Arc<Semaphore>,
}

impl ProviderRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            default_provider: None,
            concurrency_limit: Arc::new(Semaphore::new(4)),
        }
    }

    /// Create a registry with a custom concurrency limit for in-flight requests.
    pub fn with_concurrency_limit(limit: usize) -> Self {
        Self {
            providers: HashMap::new(),
            default_provider: None,
            concurrency_limit: Arc::new(Semaphore::new(limit)),
        }
    }

    /// Acquire a permit from the concurrency semaphore.
    /// Returns a permit that releases on drop.
    pub async fn acquire_permit(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.concurrency_limit
            .clone()
            .acquire_owned()
            .await
            .expect("concurrency semaphore should not be closed")
    }

    /// Build the registry from a Config.
    pub fn from_config(config: &Config) -> Result<Self> {
        let mut registry = Self::new();

        // Register Anthropic provider if configured
        if let Some(entry) = &config.providers.anthropic {
            if let Some(api_key) = &entry.api_key {
                let provider = AnthropicProvider::new(AnthropicConfig {
                    api_key: api_key.clone(),
                    model: entry
                        .model
                        .clone()
                        .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string()),
                    api_version: None,
                    base_url: entry.base_url.clone(),
                })?;
                registry.register("anthropic", provider);
                info!("Registered Anthropic provider");
            }
        }

        // Register OpenAI provider if configured
        if let Some(entry) = &config.providers.openai {
            if let Some(api_key) = &entry.api_key {
                let provider = OpenAiCompatProvider::new(OpenAiCompatConfig {
                    api_key: api_key.clone(),
                    base_url: entry
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
                    model: entry.model.clone().unwrap_or_default(),
                    organization: None,
                    no_proxy: entry.no_proxy.unwrap_or(false),
                })?;
                registry.register("openai", provider);
                info!("Registered OpenAI provider");
            }
        }

        // Register DeepSeek provider
        if let Some(entry) = &config.providers.deepseek {
            if let Some(api_key) = &entry.api_key {
                let provider = OpenAiCompatProvider::new(OpenAiCompatConfig {
                    api_key: api_key.clone(),
                    base_url: entry
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://api.deepseek.com/v1".to_string()),
                    model: entry.model.clone().unwrap_or_default(),
                    organization: None,
                    no_proxy: entry.no_proxy.unwrap_or(false),
                })?;
                registry.register("deepseek", provider);
                info!("Registered DeepSeek provider");
            }
        }

        // Register Groq provider
        if let Some(entry) = &config.providers.groq {
            if let Some(api_key) = &entry.api_key {
                let provider = OpenAiCompatProvider::new(OpenAiCompatConfig {
                    api_key: api_key.clone(),
                    base_url: entry
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://api.groq.com/openai/v1".to_string()),
                    model: entry.model.clone().unwrap_or_default(),
                    organization: None,
                    no_proxy: entry.no_proxy.unwrap_or(false),
                })?;
                registry.register("groq", provider);
                info!("Registered Groq provider");
            }
        }

        // Register OpenRouter provider
        if let Some(entry) = &config.providers.openrouter {
            if let Some(api_key) = &entry.api_key {
                let provider = OpenAiCompatProvider::new(OpenAiCompatConfig {
                    api_key: api_key.clone(),
                    base_url: entry
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string()),
                    model: entry.model.clone().unwrap_or_default(),
                    organization: None,
                    no_proxy: entry.no_proxy.unwrap_or(false),
                })?;
                registry.register("openrouter", provider);
                info!("Registered OpenRouter provider");
            }
        }

        // Register Ollama provider (localhost — always skip proxy)
        if let Some(entry) = &config.providers.ollama {
            let provider = OpenAiCompatProvider::new(OpenAiCompatConfig {
                api_key: entry.api_key.clone().unwrap_or_default(),
                base_url: entry
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:11434/v1".to_string()),
                model: entry.model.clone().unwrap_or_default(),
                organization: None,
                no_proxy: true,
            })?;
            registry.register("ollama", provider);
            info!("Registered Ollama provider");
        }

        // Register OpenCode Go provider (OpenAI-compatible endpoint)
        if let Some(entry) = &config.providers.opencode_go {
            if let Some(api_key) = &entry.api_key {
                let provider = OpenAiCompatProvider::new(OpenAiCompatConfig {
                    api_key: api_key.clone(),
                    base_url: entry
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://opencode.ai/zen/go/v1".to_string()),
                    model: entry.model.clone().unwrap_or_default(),
                    organization: None,
                    no_proxy: entry.no_proxy.unwrap_or(false),
                })?;
                registry.register("opencode_go", provider);
                info!("Registered OpenCode Go provider");
            }
        }

        // Register custom providers
        for custom in &config.custom_providers {
            let provider = OpenAiCompatProvider::new(OpenAiCompatConfig {
                api_key: custom.api_key.clone().unwrap_or_default(),
                base_url: custom.base_url.clone(),
                model: custom.model_patterns.first().cloned().unwrap_or_default(),
                organization: None,
                no_proxy: custom.no_proxy.unwrap_or(false),
            })?;
            registry.register(&custom.name, provider);
            info!("Registered custom provider: {}", custom.name);
        }

        // Set default provider based on agent model
        let model = &config.agent.model;
        let default = registry
            .resolve_provider_name(model)
            .map(|s| s.to_string())
            .or_else(|| registry.providers.keys().next().cloned());
        registry.default_provider = default;
        if let Some(ref name) = registry.default_provider {
            debug!("Default provider for model '{}': {}", model, name);
        }

        Ok(registry)
    }

    /// Register a provider.
    pub fn register(&mut self, name: &str, provider: impl LlmProvider + 'static) {
        self.providers.insert(name.to_string(), Arc::new(provider));
    }

    /// Set the default provider by name.
    pub fn set_default(&mut self, name: &str) {
        if self.providers.contains_key(name) {
            self.default_provider = Some(name.to_string());
        }
    }

    /// Resolve a model name to a provider name.
    ///
    /// Resolution order:
    /// 1. Explicit provider prefix (`opencode_go/model-name`) → that provider
    /// 2. Keyword matching against [`MODEL_KEYWORD_MAP`]
    /// 3. Fallback to the default provider
    pub fn resolve_provider_name(&self, model: &str) -> Option<&str> {
        // 1. Check for explicit provider prefix first (e.g. "opencode_go/deepseek-v4-flash").
        //    This takes precedence over keyword matching to avoid mis-routing.
        if let Some((prefix, rest)) = model.split_once('/') {
            if !rest.is_empty() && self.providers.contains_key(prefix) {
                return Some(prefix);
            }
        }

        // 2. Fall back to keyword matching.
        let lower = model.to_lowercase();
        for (keyword, provider_name) in MODEL_KEYWORD_MAP {
            if lower.contains(keyword) && self.providers.contains_key(*provider_name) {
                return Some(provider_name);
            }
        }

        // 3. Default provider.
        self.default_provider.as_deref()
    }

    /// Strip the provider prefix from a qualified model name.
    ///
    /// If `model` contains a `/` and the part before it matches a known provider
    /// (either by keyword or exact name), returns the part after the `/`.
    /// Otherwise returns `model` unchanged.
    ///
    /// Examples:
    /// - `"opencode-go/kimi-k2.6"` → `"kimi-k2.6"` (matches "opencode-go" keyword)
    /// - `"opencode_go/glm-5.1"` → `"glm-5.1"` (matches "opencode_go" keyword)
    /// - `"claude-sonnet-4-6"` → `"claude-sonnet-4-6"` (no slash, unchanged)
    /// - `"deepseek/deepseek-v4-flash"` → `"deepseek-v4-flash"` (matches "deepseek" keyword)
    pub fn strip_provider_prefix(&self, model: &str) -> String {
        let Some((prefix, rest)) = model.split_once('/') else {
            return model.to_string();
        };
        if rest.is_empty() {
            return model.to_string();
        }
        let lower_prefix = prefix.to_lowercase();
        for (keyword, provider_name) in MODEL_KEYWORD_MAP {
            if (lower_prefix == *keyword || lower_prefix == *provider_name)
                && self.providers.contains_key(*provider_name)
            {
                return rest.to_string();
            }
        }
        // Also check if prefix is an exact provider name (handles custom providers).
        if self.providers.contains_key(prefix) {
            return rest.to_string();
        }
        model.to_string()
    }

    /// Resolve a model name to a provider name, with an explicit provider override.
    ///
    /// Resolution order:
    /// 1. Explicit `provider_override` parameter (from `agent.provider` config)
    /// 2. Explicit provider prefix in model string
    /// 3. Keyword matching against [`MODEL_KEYWORD_MAP`]
    /// 4. Fallback to the default provider
    pub fn resolve_provider_name_with_override(
        &self,
        model: &str,
        provider_override: Option<&str>,
    ) -> Option<&str> {
        // 0. Explicit provider override takes absolute precedence.
        if let Some(name) = provider_override {
            if self.providers.contains_key(name) {
                return Some(name);
            }
        }
        self.resolve_provider_name(model)
    }

    /// Get a provider for a given model, with optional explicit provider override.
    pub fn get_provider_with_override(
        &self,
        model: &str,
        provider_override: Option<&str>,
    ) -> Option<Arc<dyn LlmProvider>> {
        if let Some(name) = self.resolve_provider_name_with_override(model, provider_override) {
            self.providers.get(name).cloned()
        } else {
            self.default_provider
                .as_ref()
                .and_then(|name| self.providers.get(name).cloned())
        }
    }

    /// Get a provider for a given model.
    pub fn get_provider(&self, model: &str) -> Option<Arc<dyn LlmProvider>> {
        if let Some(name) = self.resolve_provider_name(model) {
            self.providers.get(name).cloned()
        } else {
            self.default_provider
                .as_ref()
                .and_then(|name| self.providers.get(name).cloned())
        }
    }

    /// Get a provider by name.
    pub fn get_provider_by_name(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.providers.get(name).cloned()
    }

    /// List all registered provider names.
    pub fn provider_names(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::{
        BoxStream, CompletionChunk, CompletionRequest, CompletionResponse, LlmProvider,
    };
    use async_trait::async_trait;

    struct MockProvider {
        provider_name: String,
        supported_model: String,
    }

    impl MockProvider {
        fn new(name: &str, supported: &str) -> Self {
            Self {
                provider_name: name.to_string(),
                supported_model: supported.to_string(),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &str {
            &self.provider_name
        }
        fn default_model(&self) -> &str {
            &self.supported_model
        }
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> anyhow::Result<CompletionResponse> {
            Ok(CompletionResponse {
                content: Some("mock".to_string()),
                tool_calls: None,
                usage: None,
                finish_reason: None,
            })
        }
        async fn complete_stream(&self, request: CompletionRequest) -> anyhow::Result<BoxStream> {
            let response = self.complete(request).await?;
            let chunk = CompletionChunk {
                delta: response.content,
                tool_call_deltas: None,
                usage: None,
                done: true,
            };
            Ok(Box::pin(futures::stream::once(async move { Ok(chunk) })))
        }
        fn supports_model(&self, model: &str) -> bool {
            model.contains(&self.supported_model)
        }
    }

    #[test]
    fn test_registry_new() {
        let reg = ProviderRegistry::new();
        assert!(reg.provider_names().is_empty());
    }

    #[test]
    fn test_registry_register() {
        let mut reg = ProviderRegistry::new();
        reg.register("mock", MockProvider::new("mock", "test"));
        let names = reg.provider_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"mock".to_string()));
    }

    #[test]
    fn test_registry_resolve_provider_name() {
        let mut reg = ProviderRegistry::new();
        reg.register("anthropic", MockProvider::new("anthropic", "claude"));
        reg.register("openai", MockProvider::new("openai", "gpt"));

        // "claude-3.5-sonnet" should resolve to "anthropic"
        let resolved = reg.resolve_provider_name("claude-3.5-sonnet");
        assert_eq!(resolved, Some("anthropic"));

        // "gpt-4o" should resolve to "openai"
        let resolved = reg.resolve_provider_name("gpt-4o");
        assert_eq!(resolved, Some("openai"));
    }

    #[test]
    fn test_resolve_provider_prefix_takes_precedence_over_keywords() {
        let mut reg = ProviderRegistry::new();
        reg.register("opencode_go", MockProvider::new("opencode_go", "glm"));
        reg.register("openrouter", MockProvider::new("openrouter", "deepseek"));

        // "opencode_go/deepseek-v4-flash" should resolve to opencode_go,
        // NOT openrouter (even though "deepseek-v4" keyword matches openrouter).
        let resolved = reg.resolve_provider_name("opencode_go/deepseek-v4-flash");
        assert_eq!(resolved, Some("opencode_go"));
    }

    #[test]
    fn test_resolve_provider_override() {
        let mut reg = ProviderRegistry::new();
        reg.register("anthropic", MockProvider::new("anthropic", "claude"));
        reg.register("openai", MockProvider::new("openai", "gpt"));

        // Override takes absolute precedence over keyword matching.
        let resolved = reg.resolve_provider_name_with_override("claude-sonnet-4-6", Some("openai"));
        assert_eq!(resolved, Some("openai"));
    }

    #[test]
    fn test_resolve_provider_override_ignored_when_not_registered() {
        let mut reg = ProviderRegistry::new();
        reg.register("anthropic", MockProvider::new("anthropic", "claude"));

        // Unknown override falls back to keyword matching.
        let resolved =
            reg.resolve_provider_name_with_override("claude-sonnet-4-6", Some("nonexistent"));
        assert_eq!(resolved, Some("anthropic"));
    }

    #[test]
    fn test_registry_default_provider() {
        let reg = ProviderRegistry::default();
        assert!(reg.provider_names().is_empty());
        assert!(reg.default_provider.is_none());
    }

    #[test]
    fn test_strip_provider_prefix_with_keyword() {
        let mut reg = ProviderRegistry::new();
        reg.register("opencode_go", MockProvider::new("opencode_go", "glm"));

        // "opencode-go/kimi-k2.6" → "kimi-k2.6"
        assert_eq!(
            reg.strip_provider_prefix("opencode-go/kimi-k2.6"),
            "kimi-k2.6"
        );
    }

    #[test]
    fn test_strip_provider_prefix_with_provider_name() {
        let mut reg = ProviderRegistry::new();
        reg.register("opencode_go", MockProvider::new("opencode_go", "glm"));

        // "opencode_go/glm-5.1" → "glm-5.1"
        assert_eq!(reg.strip_provider_prefix("opencode_go/glm-5.1"), "glm-5.1");
    }

    #[test]
    fn test_strip_provider_prefix_no_slash() {
        let mut reg = ProviderRegistry::new();
        reg.register("openai", MockProvider::new("openai", "gpt"));

        // No slash → unchanged
        assert_eq!(reg.strip_provider_prefix("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn test_strip_provider_prefix_unknown_prefix() {
        let mut reg = ProviderRegistry::new();
        reg.register("openai", MockProvider::new("openai", "gpt"));

        // Unknown prefix with slash → unchanged
        assert_eq!(reg.strip_provider_prefix("unknown/model"), "unknown/model");
    }

    #[test]
    fn test_strip_provider_prefix_deepseek() {
        let mut reg = ProviderRegistry::new();
        reg.register("deepseek", MockProvider::new("deepseek", "deepseek"));
        reg.register("openrouter", MockProvider::new("openrouter", "deepseek"));

        // "deepseek/deepseek-v4-flash" → "deepseek-v4-flash"
        assert_eq!(
            reg.strip_provider_prefix("deepseek/deepseek-v4-flash"),
            "deepseek-v4-flash"
        );
    }

    #[test]
    fn test_strip_provider_prefix_empty_after_slash() {
        let mut reg = ProviderRegistry::new();
        reg.register("opencode_go", MockProvider::new("opencode_go", "glm"));

        // Empty after slash → unchanged
        assert_eq!(reg.strip_provider_prefix("opencode-go/"), "opencode-go/");
    }

    #[test]
    fn test_strip_provider_prefix_custom_provider() {
        let mut reg = ProviderRegistry::new();
        reg.register("my_custom", MockProvider::new("my_custom", "test"));

        // Custom provider by exact name match
        assert_eq!(
            reg.strip_provider_prefix("my_custom/mymodel-v1"),
            "mymodel-v1"
        );
    }
}
