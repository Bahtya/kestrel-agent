//! OpenAI-compatible HTTP API server using Axum.
//!
//! Provides `/v1/chat/completions` (with SSE streaming), `/v1/models`, and `/health`.
//! The completions endpoint runs the agent directly to produce responses.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures::stream::Stream;
use nanobot_agent::AgentRunner;
use nanobot_bus::MessageBus;
use nanobot_config::Config;
use nanobot_core::{Message, MessageRole};
use nanobot_providers::ProviderRegistry;
use nanobot_session::SessionManager;
use nanobot_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{debug, info, warn};

/// Shared state for the API server.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub bus: Arc<MessageBus>,
    pub session_manager: Arc<SessionManager>,
    pub provider_registry: Arc<ProviderRegistry>,
    pub tool_registry: Arc<ToolRegistry>,
    pub api_key: Option<String>,
}

/// The API server.
pub struct ApiServer {
    state: AppState,
    port: u16,
}

impl ApiServer {
    pub fn new(
        config: Config,
        bus: MessageBus,
        session_manager: SessionManager,
        port: u16,
    ) -> Self {
        let state = AppState {
            config: Arc::new(config),
            bus: Arc::new(bus),
            session_manager: Arc::new(session_manager),
            provider_registry: Arc::new(ProviderRegistry::new()),
            tool_registry: Arc::new(ToolRegistry::new()),
            api_key: None,
        };
        Self { state, port }
    }

    /// Create with pre-built provider and tool registries.
    pub fn with_registries(
        config: Config,
        bus: MessageBus,
        session_manager: SessionManager,
        provider_registry: ProviderRegistry,
        tool_registry: ToolRegistry,
        port: u16,
    ) -> Self {
        let state = AppState {
            config: Arc::new(config),
            bus: Arc::new(bus),
            session_manager: Arc::new(session_manager),
            provider_registry: Arc::new(provider_registry),
            tool_registry: Arc::new(tool_registry),
            api_key: None,
        };
        Self { state, port }
    }

    /// Set an API key for bearer-token authentication.
    pub fn with_api_key(mut self, key: String) -> Self {
        self.state.api_key = Some(key);
        self
    }

    /// Build the Axum router.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/v1/chat/completions", post(chat_completions))
            .route("/v1/models", get(list_models))
            .route("/health", get(health))
            .layer(CorsLayer::permissive())
            .layer(TraceLayer::new_for_http())
            .with_state(self.state.clone())
    }

    /// Start the API server.
    pub async fn run(&self) -> anyhow::Result<()> {
        let app = self.router();
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], self.port));
        info!("API server listening on {}", addr);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }
}

// ─── Request/Response Types ──────────────────────────────────

/// OpenAI-compatible chat completion request.
#[derive(Debug, Deserialize)]
pub struct ChatCompletionRequest {
    /// Model name to use for completion.
    pub model: String,
    /// Conversation messages.
    pub messages: Vec<ApiMessage>,
    /// Sampling temperature (0-2).
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Maximum tokens to generate.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Whether to stream the response.
    #[serde(default)]
    pub stream: bool,
}

/// A message in the chat completion API.
#[derive(Debug, Deserialize, Serialize)]
pub struct ApiMessage {
    /// Role of the message author (system, user, assistant).
    pub role: String,
    /// Text content of the message.
    pub content: String,
}

/// OpenAI-compatible chat completion response.
#[derive(Debug, Serialize)]
pub struct ChatCompletionResponse {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<Choice>,
    usage: UsageInfo,
}

#[derive(Debug, Serialize)]
struct Choice {
    index: u32,
    message: ResponseMessage,
    finish_reason: String,
}

#[derive(Debug, Serialize)]
struct ResponseMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct UsageInfo {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Debug, Serialize)]
struct ModelsResponse {
    object: String,
    data: Vec<ModelInfo>,
}

#[derive(Debug, Serialize)]
struct ModelInfo {
    id: String,
    object: String,
    created: u64,
    owned_by: String,
}

/// Error response matching OpenAI format.
#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    message: String,
    r#type: String,
    code: Option<String>,
}

// ─── Auth helper ────────────────────────────────────────────

/// Validate bearer token if an API key is configured.
/// Returns Ok(()) if auth passes (or no key configured), Err(response) otherwise.
#[allow(clippy::result_large_err)]
fn check_auth(headers: &HeaderMap, expected_key: &Option<String>) -> Result<(), axum::response::Response> {
    let key = match expected_key {
        Some(k) if !k.is_empty() => k,
        _ => return Ok(()), // No auth configured
    };

    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if auth_header == format!("Bearer {}", key) {
        Ok(())
    } else {
        let error = ErrorResponse {
            error: ErrorDetail {
                message: "Invalid or missing API key".to_string(),
                r#type: "authentication_error".to_string(),
                code: Some("invalid_api_key".to_string()),
            },
        };
        Err((StatusCode::UNAUTHORIZED, Json(error)).into_response())
    }
}

// ─── Handlers ──────────────────────────────────────────────

async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChatCompletionRequest>,
) -> impl IntoResponse {
    // Auth check
    if let Err(resp) = check_auth(&headers, &state.api_key) {
        return resp;
    }

    debug!("Chat completion request for model: {} (stream: {})", req.model, req.stream);

    // Validate model is non-empty
    if req.model.is_empty() {
        let error = ErrorResponse {
            error: ErrorDetail {
                message: "Model must be a non-empty string".to_string(),
                r#type: "invalid_request_error".to_string(),
                code: None,
            },
        };
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Extract the last user message
    let user_content = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    if user_content.is_empty() {
        let error = ErrorResponse {
            error: ErrorDetail {
                message: "No user message found in request".to_string(),
                r#type: "invalid_request_error".to_string(),
                code: None,
            },
        };
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Check provider availability early
    if state.provider_registry.get_provider(&req.model).is_none() {
        let error = ErrorResponse {
            error: ErrorDetail {
                message: format!("Model '{}' not found. Check available models at GET /v1/models.", req.model),
                r#type: "invalid_request_error".to_string(),
                code: Some("model_not_found".to_string()),
            },
        };
        return (StatusCode::NOT_FOUND, Json(error)).into_response();
    }

    // Build the system prompt from config
    let system_prompt = state
        .config
        .agent
        .system_prompt
        .clone()
        .unwrap_or_else(|| "You are a helpful AI assistant.".to_string());

    // Convert API messages to nanobot Messages
    let messages: Vec<Message> = req
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| Message {
            role: match m.role.as_str() {
                "assistant" => MessageRole::Assistant,
                _ => MessageRole::User,
            },
            content: m.content.clone(),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        })
        .collect();

    if req.stream {
        return stream_completion(state, req, system_prompt, messages).await;
    }

    // Non-streaming path
    non_stream_completion(state, req, system_prompt, messages).await
}

/// Handle non-streaming completion.
async fn non_stream_completion(
    state: AppState,
    req: ChatCompletionRequest,
    system_prompt: String,
    messages: Vec<Message>,
) -> axum::response::Response {
    let runner = AgentRunner::new(
        state.config.clone(),
        state.provider_registry.clone(),
        state.tool_registry.clone(),
    );

    match runner.run(system_prompt, messages).await {
        Ok(result) => {
            let response = ChatCompletionResponse {
                id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                object: "chat.completion".to_string(),
                created: chrono::Utc::now().timestamp() as u64,
                model: req.model.clone(),
                choices: vec![Choice {
                    index: 0,
                    message: ResponseMessage {
                        role: "assistant".to_string(),
                        content: result.content,
                    },
                    finish_reason: "stop".to_string(),
                }],
                usage: UsageInfo {
                    prompt_tokens: result.usage.prompt_tokens.unwrap_or(0),
                    completion_tokens: result.usage.completion_tokens.unwrap_or(0),
                    total_tokens: result.usage.total_tokens.unwrap_or(0),
                },
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            warn!("Agent error: {}", e);
            let msg = e.to_string();
            let status = if msg.contains("429") {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            let error = ErrorResponse {
                error: ErrorDetail {
                    message: format!("Agent processing error: {}", msg),
                    r#type: "server_error".to_string(),
                    code: None,
                },
            };
            (status, Json(error)).into_response()
        }
    }
}

/// Handle streaming completion via SSE.
async fn stream_completion(
    state: AppState,
    req: ChatCompletionRequest,
    system_prompt: String,
    messages: Vec<Message>,
) -> axum::response::Response {
    let completion_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
    let model = req.model.clone();
    let created = chrono::Utc::now().timestamp() as u64;

    let runner = AgentRunner::new(
        state.config.clone(),
        state.provider_registry.clone(),
        state.tool_registry.clone(),
    );

    // Run the agent and then stream the result as a single SSE event.
    // For true token-by-token streaming, the runner would need to yield incremental
    // results; this provides the OpenAI SSE wire format with the full response.
    let stream_result = runner.run(system_prompt, messages).await;

    let stream: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> =
        match stream_result {
            Ok(result) => {
                let usage = result.usage;
                let content = result.content;
                let id = completion_id;

                Box::pin(futures::stream::once(async move {
                    // First chunk: role announcement
                    let _chunk1 = serde_json::json!({
                        "id": id,
                        "object": "chat.completion.chunk",
                        "created": created,
                        "model": model,
                        "choices": [{
                            "index": 0,
                            "delta": {"role": "assistant"},
                            "finish_reason": null
                        }]
                    });

                    // Content chunk
                    let chunk2 = serde_json::json!({
                        "id": id,
                        "object": "chat.completion.chunk",
                        "created": created,
                        "model": model,
                        "choices": [{
                            "index": 0,
                            "delta": {"content": content},
                            "finish_reason": null
                        }],
                        "usage": null
                    });

                    // Final chunk with usage
                    let _chunk3 = serde_json::json!({
                        "id": id,
                        "object": "chat.completion.chunk",
                        "created": created,
                        "model": model,
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": "stop"
                        }],
                        "usage": {
                            "prompt_tokens": usage.prompt_tokens.unwrap_or(0),
                            "completion_tokens": usage.completion_tokens.unwrap_or(0),
                            "total_tokens": usage.total_tokens.unwrap_or(0)
                        }
                    });

                    // We'll return the content as a single event (simplifies the stream)
                    Ok(Event::default().data(chunk2.to_string()))
                }))
            }
            Err(e) => {
                let msg = e.to_string();
                Box::pin(futures::stream::once(async move {
                    let error = serde_json::json!({
                        "error": {
                            "message": format!("Agent error: {}", msg),
                            "type": "server_error",
                            "code": null
                        }
                    });
                    Ok(Event::default().data(error.to_string()))
                }))
            }
        };

    let sse = Sse::new(stream).keep_alive(KeepAlive::default());
    sse.into_response()
}

async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    let mut models = Vec::new();

    // Collect models from all registered providers
    for name in state.provider_registry.provider_names() {
        models.push(ModelInfo {
            id: name.clone(),
            object: "model".to_string(),
            created: 0,
            owned_by: "nanobot-rs".to_string(),
        });
    }

    // Also include the configured agent model
    let agent_model = &state.config.agent.model;
    if !agent_model.is_empty() && !models.iter().any(|m| m.id == *agent_model) {
        models.push(ModelInfo {
            id: agent_model.clone(),
            object: "model".to_string(),
            created: 0,
            owned_by: "nanobot-rs".to_string(),
        });
    }

    Json(ModelsResponse {
        object: "list".to_string(),
        data: models,
    })
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "version": nanobot_core::VERSION,
    }))
}

// ─── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use nanobot_providers::base::{BoxStream, CompletionChunk, CompletionRequest, CompletionResponse, LlmProvider};
    use nanobot_core::Usage;
    use tower::ServiceExt;

    /// Mock provider for testing.
    struct MockProvider;

    #[async_trait::async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &str { "mock" }
        async fn complete(&self, _req: CompletionRequest) -> anyhow::Result<CompletionResponse> {
            Ok(CompletionResponse {
                content: Some("Mock response".to_string()),
                tool_calls: None,
                usage: Some(Usage {
                    prompt_tokens: Some(5),
                    completion_tokens: Some(3),
                    total_tokens: Some(8),
                }),
                finish_reason: Some("stop".to_string()),
            })
        }
        async fn complete_stream(&self, req: CompletionRequest) -> anyhow::Result<BoxStream> {
            let resp = self.complete(req).await?;
            let chunk = CompletionChunk {
                delta: resp.content,
                tool_call_deltas: None,
                usage: resp.usage,
                done: true,
            };
            Ok(Box::pin(futures::stream::once(async move { Ok(chunk) })))
        }
        fn supports_model(&self, _model: &str) -> bool { true }
    }

    fn test_state() -> AppState {
        let config = Config::default();
        let bus = MessageBus::new();
        let tmp = tempfile::tempdir().unwrap();
        let session_manager = SessionManager::new(tmp.path().to_path_buf()).unwrap();
        AppState {
            config: Arc::new(config),
            bus: Arc::new(bus),
            session_manager: Arc::new(session_manager),
            provider_registry: Arc::new(ProviderRegistry::new()),
            tool_registry: Arc::new(ToolRegistry::new()),
            api_key: None,
        }
    }

    fn test_state_with_provider() -> AppState {
        let mut config = Config::default();
        config.agent.model = "mock-model".to_string();
        let bus = MessageBus::new();
        let tmp = tempfile::tempdir().unwrap();
        let session_manager = SessionManager::new(tmp.path().to_path_buf()).unwrap();
        let mut reg = ProviderRegistry::new();
        reg.register("mock", MockProvider);
        reg.set_default("mock");
        AppState {
            config: Arc::new(config),
            bus: Arc::new(bus),
            session_manager: Arc::new(session_manager),
            provider_registry: Arc::new(reg),
            tool_registry: Arc::new(ToolRegistry::new()),
            api_key: None,
        }
    }

    fn test_state_with_auth() -> AppState {
        let mut state = test_state_with_provider();
        state.api_key = Some("sk-secret".to_string());
        state
    }

    fn test_router() -> Router {
        Router::new()
            .route("/v1/chat/completions", post(chat_completions))
            .route("/v1/models", get(list_models))
            .route("/health", get(health))
            .with_state(test_state())
    }

    fn router_with_provider() -> Router {
        Router::new()
            .route("/v1/chat/completions", post(chat_completions))
            .route("/v1/models", get(list_models))
            .route("/health", get(health))
            .with_state(test_state_with_provider())
    }

    fn router_with_auth() -> Router {
        Router::new()
            .route("/v1/chat/completions", post(chat_completions))
            .route("/v1/models", get(list_models))
            .route("/health", get(health))
            .with_state(test_state_with_auth())
    }

    // ─── Health ─────────────────────────────────────────

    #[tokio::test]
    async fn test_health_endpoint() {
        let app = test_router();
        let req = Request::builder().uri("/health").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "ok");
        assert_eq!(v["version"], nanobot_core::VERSION);
    }

    // ─── Models ─────────────────────────────────────────

    #[tokio::test]
    async fn test_models_endpoint_basic() {
        let app = test_router();
        let req = Request::builder().uri("/v1/models").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["object"], "list");
        assert!(v["data"].is_array());
    }

    #[tokio::test]
    async fn test_models_lists_registered_providers() {
        let app = router_with_provider();
        let req = Request::builder().uri("/v1/models").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let ids = v["data"].as_array().unwrap().iter()
            .filter_map(|m| m["id"].as_str().map(String::from))
            .collect::<Vec<_>>();
        assert!(ids.contains(&"mock".to_string()), "Should list 'mock' provider");
        assert!(ids.contains(&"mock-model".to_string()), "Should list agent model");
    }

    // ─── Chat completions: validation ───────────────────

    #[tokio::test]
    async fn test_chat_completions_no_user_message() {
        let app = test_router();
        let req_body = serde_json::json!({
            "model": "test-model",
            "messages": [{"role": "system", "content": "You are helpful"}]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&req_body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v["error"]["message"].as_str().unwrap().contains("No user message"));
    }

    #[tokio::test]
    async fn test_chat_completions_empty_model() {
        let app = test_router();
        let req_body = serde_json::json!({
            "model": "",
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&req_body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_chat_completions_model_not_found() {
        // Use a registry with a provider but NO default set, and a model name
        // that doesn't match any keyword — so get_provider returns None.
        let mut config = Config::default();
        config.agent.model = "mock-model".to_string();
        let bus = MessageBus::new();
        let tmp = tempfile::tempdir().unwrap();
        let session_manager = SessionManager::new(tmp.path().to_path_buf()).unwrap();
        let mut reg = ProviderRegistry::new();
        reg.register("mock", MockProvider);
        // Intentionally do NOT call set_default — no fallback for unknown models
        let state = AppState {
            config: Arc::new(config),
            bus: Arc::new(bus),
            session_manager: Arc::new(session_manager),
            provider_registry: Arc::new(reg),
            tool_registry: Arc::new(ToolRegistry::new()),
            api_key: None,
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(chat_completions))
            .route("/v1/models", get(list_models))
            .route("/health", get(health))
            .with_state(state);

        let req_body = serde_json::json!({
            "model": "nonexistent-model",
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&req_body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v["error"]["message"].as_str().unwrap().contains("not found"));
        assert_eq!(v["error"]["code"].as_str(), Some("model_not_found"));
    }

    // ─── Chat completions: no provider (500) ────────────

    #[tokio::test]
    async fn test_chat_completions_no_provider() {
        let app = test_router();
        let req_body = serde_json::json!({
            "model": "test-model",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&req_body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // No provider registered → model not found (404) since registry is empty
        assert!(resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ─── Chat completions: success with mock provider ───

    #[tokio::test]
    async fn test_chat_completions_success() {
        let app = router_with_provider();
        let req_body = serde_json::json!({
            "model": "mock-model",
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "user", "content": "Hello"}
            ],
            "temperature": 0.7,
            "max_tokens": 100
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&req_body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["object"], "chat.completion");
        assert_eq!(v["model"], "mock-model");
        assert_eq!(v["choices"][0]["message"]["role"], "assistant");
        assert_eq!(v["choices"][0]["message"]["content"], "Mock response");
        assert_eq!(v["choices"][0]["finish_reason"], "stop");
        assert_eq!(v["usage"]["prompt_tokens"], 5);
        assert_eq!(v["usage"]["completion_tokens"], 3);
        assert_eq!(v["usage"]["total_tokens"], 8);
        assert!(v["id"].as_str().unwrap().starts_with("chatcmpl-"));
    }

    // ─── Chat completions: streaming ─────────────────────

    #[tokio::test]
    async fn test_chat_completions_streaming() {
        let app = router_with_provider();
        let req_body = serde_json::json!({
            "model": "mock-model",
            "messages": [{"role": "user", "content": "Hello"}],
            "stream": true
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&req_body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // SSE responses use 200 with text/event-stream
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert!(ct.contains("text/event-stream"), "Expected SSE content type, got: {}", ct);
    }

    // ─── Auth: 401 tests ─────────────────────────────────

    #[tokio::test]
    async fn test_auth_missing_key() {
        let app = router_with_auth();
        let req_body = serde_json::json!({
            "model": "mock-model",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&req_body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"]["code"], "invalid_api_key");
    }

    #[tokio::test]
    async fn test_auth_wrong_key() {
        let app = router_with_auth();
        let req_body = serde_json::json!({
            "model": "mock-model",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .header("authorization", "Bearer wrong-key")
            .body(Body::from(serde_json::to_string(&req_body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_valid_key() {
        let app = router_with_auth();
        let req_body = serde_json::json!({
            "model": "mock-model",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .header("authorization", "Bearer sk-secret")
            .body(Body::from(serde_json::to_string(&req_body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_health_needs_no_key() {
        // Health endpoint should work without auth even when api_key is set
        let app = router_with_auth();
        let req = Request::builder().uri("/health").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_models_needs_no_key() {
        // Models endpoint should work without auth (matches OpenAI behavior)
        let app = router_with_auth();
        let req = Request::builder().uri("/v1/models").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ─── Server construction ─────────────────────────────

    #[tokio::test]
    async fn test_server_new_and_router() {
        let config = Config::default();
        let bus = MessageBus::new();
        let tmp = tempfile::tempdir().unwrap();
        let session_manager = SessionManager::new(tmp.path().to_path_buf()).unwrap();
        let server = ApiServer::new(config, bus, session_manager, 8080);
        let _router = server.router();
    }

    #[tokio::test]
    async fn test_server_with_registries() {
        let config = Config::default();
        let bus = MessageBus::new();
        let tmp = tempfile::tempdir().unwrap();
        let session_manager = SessionManager::new(tmp.path().to_path_buf()).unwrap();
        let providers = ProviderRegistry::new();
        let tools = ToolRegistry::new();
        let server = ApiServer::with_registries(config, bus, session_manager, providers, tools, 9090);
        let _router = server.router();
    }

    #[tokio::test]
    async fn test_server_with_api_key() {
        let config = Config::default();
        let bus = MessageBus::new();
        let tmp = tempfile::tempdir().unwrap();
        let session_manager = SessionManager::new(tmp.path().to_path_buf()).unwrap();
        let server = ApiServer::new(config, bus, session_manager, 8080)
            .with_api_key("sk-test".to_string());
        assert_eq!(server.state.api_key, Some("sk-test".to_string()));
    }

    // ─── Serialization tests ─────────────────────────────

    #[test]
    fn test_chat_completion_request_deserialize() {
        let json = r#"{
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "Hi"}],
            "temperature": 0.7,
            "max_tokens": 100,
            "stream": false
        }"#;
        let req: ChatCompletionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "gpt-4");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.temperature, Some(0.7));
        assert_eq!(req.max_tokens, Some(100));
        assert!(!req.stream);
    }

    #[test]
    fn test_chat_completion_request_minimal() {
        let json = r#"{"model": "test", "messages": []}"#;
        let req: ChatCompletionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "test");
        assert!(req.messages.is_empty());
        assert!(req.temperature.is_none());
        assert!(req.max_tokens.is_none());
        assert!(!req.stream);
    }

    #[test]
    fn test_api_message_serde() {
        let msg = ApiMessage {
            role: "user".to_string(),
            content: "Hello world".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ApiMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, "user");
        assert_eq!(back.content, "Hello world");
    }
}
