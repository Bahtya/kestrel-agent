//! Provider registry — direct provider lookup by name.
//!
//! Users explicitly select both provider and model. No keyword guessing
//! or prefix-based routing.

use crate::anthropic::{AnthropicConfig, AnthropicProvider};
use crate::base::LlmProvider;
use crate::openai_compat::{OpenAiCompatConfig, OpenAiCompatProvider};
use anyhow::Result;
use kestrel_config::schema::Config;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{debug, info};

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

        // Set default provider from explicit config, otherwise pick first registered.
        let default = if let Some(ref name) = config.agent.provider {
            if registry.providers.contains_key(name) {
                Some(name.clone())
            } else {
                registry.providers.keys().next().cloned()
            }
        } else {
            registry.providers.keys().next().cloned()
        };
        registry.default_provider = default;
        if let Some(ref name) = registry.default_provider {
            debug!("Default provider: {}", name);
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

    /// Get a provider by name, falling back to the default provider.
    pub fn get_provider_by_name(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.providers.get(name).cloned().or_else(|| {
            self.default_provider
                .as_ref()
                .and_then(|n| self.providers.get(n).cloned())
        })
    }

    /// Get the provider for the given provider name string.
    ///
    /// `provider_name` must be an exact provider name (e.g. "opencode_go", "anthropic").
    /// Returns the provider, or the default provider as fallback.
    pub fn get_provider(&self, provider_name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.get_provider_by_name(provider_name)
    }

    /// List all registered provider names.
    pub fn provider_names(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    /// Return the name of the default provider, if any.
    pub fn default_provider_name(&self) -> Option<&str> {
        self.default_provider.as_deref()
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
    fn test_get_provider_by_name() {
        let mut reg = ProviderRegistry::new();
        reg.register("anthropic", MockProvider::new("anthropic", "claude"));
        reg.register("openai", MockProvider::new("openai", "gpt"));

        let p = reg.get_provider_by_name("anthropic");
        assert!(p.is_some());
        assert_eq!(p.unwrap().name(), "anthropic");

        // Unknown name returns None (no default set).
        assert!(reg.get_provider_by_name("unknown").is_none());
    }

    #[test]
    fn test_get_provider_by_name_fallback_to_default() {
        let mut reg = ProviderRegistry::new();
        reg.register("anthropic", MockProvider::new("anthropic", "claude"));
        reg.register("openai", MockProvider::new("openai", "gpt"));
        reg.set_default("openai");

        // Unknown name falls back to default.
        let p = reg.get_provider_by_name("nonexistent");
        assert!(p.is_some());
        assert_eq!(p.unwrap().name(), "openai");
    }

    #[test]
    fn test_get_provider() {
        let mut reg = ProviderRegistry::new();
        reg.register("opencode_go", MockProvider::new("opencode_go", "glm"));

        let p = reg.get_provider("opencode_go");
        assert!(p.is_some());
        assert_eq!(p.unwrap().name(), "opencode_go");
    }

    #[test]
    fn test_registry_default_provider() {
        let reg = ProviderRegistry::default();
        assert!(reg.provider_names().is_empty());
        assert!(reg.default_provider.is_none());
    }

    #[test]
    fn test_set_default_validates() {
        let mut reg = ProviderRegistry::new();
        reg.register("anthropic", MockProvider::new("anthropic", "claude"));
        reg.set_default("nonexistent");
        assert!(reg.default_provider.is_none());
        reg.set_default("anthropic");
        assert_eq!(reg.default_provider.as_deref(), Some("anthropic"));
    }

    #[test]
    fn test_default_provider_name() {
        let mut reg = ProviderRegistry::new();
        assert!(reg.default_provider_name().is_none());
        reg.register("openai", MockProvider::new("openai", "gpt"));
        reg.set_default("openai");
        assert_eq!(reg.default_provider_name(), Some("openai"));
    }
}
