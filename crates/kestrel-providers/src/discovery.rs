//! Model discovery — fetch available models from provider APIs dynamically.
//!
//! Each provider can implement [`ModelDiscovery`] to list its models.
//! [`ModelCatalog`] aggregates models from all configured providers with
//! time-based caching so repeated `/settings models` calls don't hammer APIs.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Metadata for a single model offered by a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier used in API calls (e.g. "glm-5.1").
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Provider that serves this model (e.g. "opencode_go").
    pub provider: String,
    /// Maximum context window in tokens, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u64>,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Qualified model ID: `provider/model_id` (e.g. "opencode_go/glm-5.1").
impl ModelInfo {
    pub fn qualified_id(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }
}

/// Trait for providers that can enumerate their available models.
#[async_trait::async_trait]
pub trait ModelDiscovery: Send + Sync {
    /// Fetch the list of models available from this provider.
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
}

// ---------------------------------------------------------------------------
// OpenAI-compatible /v1/models discovery
// ---------------------------------------------------------------------------

/// Fetch models from an OpenAI-compatible `/v1/models` endpoint.
pub struct OpenAiCompatDiscovery {
    base_url: String,
    api_key: String,
    provider_name: String,
    no_proxy: bool,
}

impl OpenAiCompatDiscovery {
    pub fn new(base_url: String, api_key: String, provider_name: String, no_proxy: bool) -> Self {
        Self {
            base_url,
            api_key,
            provider_name,
            no_proxy,
        }
    }
}

/// Response shape from OpenAI-compatible `/v1/models`.
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelObject>,
}

#[derive(Debug, Deserialize)]
struct ModelObject {
    id: String,
    #[serde(default)]
    owned_by: Option<String>,
}

#[async_trait::async_trait]
impl ModelDiscovery for OpenAiCompatDiscovery {
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let client = crate::build_client(self.no_proxy)?;
        let url = format!("{}/models", self.base_url);

        debug!(
            "Fetching models from {} for provider {}",
            url, self.provider_name
        );

        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Models API error ({}): {}", status, text);
        }

        let body: ModelsResponse = resp.json().await?;
        let models: Vec<_> = body
            .data
            .into_iter()
            .map(|m| ModelInfo {
                id: m.id.clone(),
                name: m
                    .owned_by
                    .as_ref()
                    .map(|o| format!("{} ({})", m.id, o))
                    .unwrap_or_else(|| m.id.clone()),
                provider: self.provider_name.clone(),
                context_length: None,
                description: None,
            })
            .collect();

        debug!(
            "Discovered {} models from provider {}",
            models.len(),
            self.provider_name
        );

        Ok(models)
    }
}

// ---------------------------------------------------------------------------
// Model catalog with caching
// ---------------------------------------------------------------------------

/// Cache TTL for model lists (5 minutes).
const CACHE_TTL: Duration = Duration::from_secs(300);

/// A cached model list with its fetch time.
struct CachedModels {
    models: Vec<ModelInfo>,
    fetched_at: Instant,
}

/// Aggregates models from multiple discovery sources with caching.
pub struct ModelCatalog {
    discoverers: HashMap<String, Arc<dyn ModelDiscovery>>,
    cache: RwLock<HashMap<String, CachedModels>>,
}

impl ModelCatalog {
    pub fn new() -> Self {
        Self {
            discoverers: HashMap::new(),
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Register a model discovery source for a provider.
    pub fn register(&mut self, provider_name: &str, discoverer: Arc<dyn ModelDiscovery>) {
        self.discoverers
            .insert(provider_name.to_string(), discoverer);
    }

    /// List all available models across all registered providers.
    ///
    /// Uses cached results when available (TTL: 5 minutes). Fetches from
    /// each provider concurrently. Failures are logged but don't block
    /// results from other providers.
    pub async fn list_all_models(&self) -> Vec<ModelInfo> {
        let cache = self.cache.read().await;
        let now = Instant::now();

        // Collect providers that need fresh data.
        let mut results = Vec::new();
        let mut to_fetch = Vec::new();

        for (name, discoverer) in &self.discoverers {
            if let Some(cached) = cache.get(name) {
                if now.duration_since(cached.fetched_at) < CACHE_TTL {
                    results.extend(cached.models.clone());
                    continue;
                }
            }
            to_fetch.push((name.clone(), Arc::clone(discoverer)));
        }

        drop(cache); // Release read lock before fetching.

        // Fetch missing providers concurrently.
        if !to_fetch.is_empty() {
            let fetches: Vec<_> = to_fetch
                .iter()
                .map(|(name, disc)| {
                    let name = name.clone();
                    let disc = Arc::clone(disc);
                    tokio::spawn(async move {
                        let result = disc.list_models().await;
                        (name, result)
                    })
                })
                .collect();

            let mut cache = self.cache.write().await;
            for handle in fetches {
                match handle.await {
                    Ok((name, Ok(models))) => {
                        results.extend(models.clone());
                        cache.insert(
                            name,
                            CachedModels {
                                models,
                                fetched_at: Instant::now(),
                            },
                        );
                    }
                    Ok((name, Err(e))) => {
                        warn!("Failed to fetch models from {}: {}", name, e);
                        // Serve stale cache if available.
                        if let Some(cached) = cache.get(&name) {
                            results.extend(cached.models.clone());
                        }
                    }
                    Err(e) => {
                        warn!("Model discovery task panicked: {}", e);
                    }
                }
            }
        }

        results.sort_by(|a, b| a.provider.cmp(&b.provider).then_with(|| a.id.cmp(&b.id)));

        results
    }

    /// Invalidate cached models for all providers (force refresh on next list).
    pub async fn invalidate_cache(&self) {
        self.cache.write().await.clear();
    }

    /// List models for a single provider.
    pub async fn list_provider_models(&self, provider: &str) -> Vec<ModelInfo> {
        let all = self.list_all_models().await;
        all.into_iter().filter(|m| m.provider == provider).collect()
    }
}

impl Default for ModelCatalog {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a [`ModelCatalog`] from config, registering discovery for providers
/// that support it (currently OpenCode Go and any custom provider with a
/// reachable `/v1/models` endpoint).
pub fn build_catalog(config: &kestrel_config::schema::Config) -> ModelCatalog {
    let mut catalog = ModelCatalog::new();

    // OpenCode Go — primary target with dynamic model fetching.
    if let Some(entry) = &config.providers.opencode_go {
        if let Some(api_key) = &entry.api_key {
            let base_url = entry
                .base_url
                .clone()
                .unwrap_or_else(|| "https://opencode.ai/zen/go/v1".to_string());
            catalog.register(
                "opencode_go",
                Arc::new(OpenAiCompatDiscovery::new(
                    base_url,
                    api_key.clone(),
                    "opencode_go".to_string(),
                    entry.no_proxy.unwrap_or(false),
                )),
            );
        }
    }

    // GLM Coding Plan (智谱) — OpenAI-compatible dynamic model fetching.
    if let Some(entry) = &config.providers.glm_coding_plan {
        if let Some(api_key) = &entry.api_key {
            let base_url = entry
                .base_url
                .clone()
                .unwrap_or_else(|| "https://open.bigmodel.cn/api/coding/paas/v4".to_string());
            catalog.register(
                "glm_coding_plan",
                Arc::new(OpenAiCompatDiscovery::new(
                    base_url,
                    api_key.clone(),
                    "glm_coding_plan".to_string(),
                    entry.no_proxy.unwrap_or(false),
                )),
            );
        }
    }

    // Any OpenAI-compatible provider with an API key can potentially list models.
    // Register discovery for the built-in providers that expose /v1/models.
    let builtins: Vec<(&str, Option<&kestrel_config::schema::ProviderEntry>, &str)> = vec![
        (
            "openai",
            config.providers.openai.as_ref(),
            "https://api.openai.com/v1",
        ),
        (
            "deepseek",
            config.providers.deepseek.as_ref(),
            "https://api.deepseek.com/v1",
        ),
        (
            "groq",
            config.providers.groq.as_ref(),
            "https://api.groq.com/openai/v1",
        ),
        (
            "openrouter",
            config.providers.openrouter.as_ref(),
            "https://openrouter.ai/api/v1",
        ),
    ];

    for (name, entry_opt, default_url) in builtins {
        if let Some(entry) = entry_opt {
            if let Some(api_key) = &entry.api_key {
                let base_url = entry
                    .base_url
                    .clone()
                    .unwrap_or_else(|| default_url.to_string());
                catalog.register(
                    name,
                    Arc::new(OpenAiCompatDiscovery::new(
                        base_url,
                        api_key.clone(),
                        name.to_string(),
                        entry.no_proxy.unwrap_or(false),
                    )),
                );
            }
        }
    }

    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_info_qualified_id() {
        let info = ModelInfo {
            id: "glm-5.1".to_string(),
            name: "GLM-5.1".to_string(),
            provider: "opencode_go".to_string(),
            context_length: Some(200_000),
            description: None,
        };
        assert_eq!(info.qualified_id(), "opencode_go/glm-5.1");
    }

    #[tokio::test]
    async fn test_catalog_empty() {
        let catalog = ModelCatalog::new();
        let models = catalog.list_all_models().await;
        assert!(models.is_empty());
    }

    #[tokio::test]
    async fn test_catalog_default() {
        let catalog = ModelCatalog::default();
        let models = catalog.list_all_models().await;
        assert!(models.is_empty());
    }

    #[tokio::test]
    async fn test_catalog_invalidate() {
        let catalog = ModelCatalog::new();
        catalog.invalidate_cache().await;
        let models = catalog.list_all_models().await;
        assert!(models.is_empty());
    }
}
