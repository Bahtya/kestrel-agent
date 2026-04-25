//! Mock LLM provider for deterministic testing.
//!
//! Replaces the ~10 duplicate `MockProvider` implementations across the codebase.
//!
//! # Usage
//!
//! ```
//! use kestrel_test_utils::MockProvider;
//! use kestrel_providers::LlmProvider;
//!
//! // Simple text response
//! let provider = MockProvider::simple("Hello!");
//!
//! // Multiple responses in sequence
//! let provider = MockProvider::multi(vec!["first", "second", "third"]);
//!
//! // Fail first N calls, then succeed
//! let provider = MockProvider::simple("ok").with_fail_n(2);
//!
//! // Always fail
//! let provider = MockProvider::always_fail("API error");
//!
//! // Slow response (simulates timeout)
//! let provider = MockProvider::simple("ok").with_delay(std::time::Duration::from_secs(60));
//! ```

use async_trait::async_trait;
use futures::stream;
use kestrel_core::Usage;
use kestrel_providers::base::{
    BoxStream, CompletionChunk, CompletionRequest, CompletionResponse, LlmProvider,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Mock LLM provider that returns deterministic, preconfigured responses.
///
/// State is shared via `Arc` so clones (as happens during `ProviderRegistry::register`)
/// all point to the same counters.
#[derive(Clone)]
pub struct MockProvider {
    state: Arc<MockProviderState>,
}

struct MockProviderState {
    responses: Vec<CompletionResponse>,
    call_count: AtomicU32,
    fail_until: AtomicU32,
    fail_message: String,
    delay: Option<Duration>,
}

impl MockProvider {
    /// Create a provider that returns a simple text response on every call.
    pub fn simple(text: &str) -> Self {
        Self::from_response(CompletionResponse {
            content: Some(text.to_string()),
            tool_calls: None,
            usage: Some(Usage {
                prompt_tokens: Some(10),
                completion_tokens: Some(5),
                total_tokens: Some(15),
            }),
            finish_reason: Some("stop".to_string()),
        })
    }

    /// Create a provider that returns different text per call.
    ///
    /// After exhausting all responses, returns a default fallback.
    pub fn multi(texts: Vec<&str>) -> Self {
        Self {
            state: Arc::new(MockProviderState {
                responses: texts
                    .into_iter()
                    .map(|text| CompletionResponse {
                        content: Some(text.to_string()),
                        tool_calls: None,
                        usage: Some(Usage {
                            prompt_tokens: Some(10),
                            completion_tokens: Some(5),
                            total_tokens: Some(15),
                        }),
                        finish_reason: Some("stop".to_string()),
                    })
                    .collect(),
                call_count: AtomicU32::new(0),
                fail_until: AtomicU32::new(0),
                fail_message: String::new(),
                delay: None,
            }),
        }
    }

    /// Create a provider from a single explicit response.
    pub fn from_response(response: CompletionResponse) -> Self {
        Self {
            state: Arc::new(MockProviderState {
                responses: vec![response],
                call_count: AtomicU32::new(0),
                fail_until: AtomicU32::new(0),
                fail_message: String::new(),
                delay: None,
            }),
        }
    }

    /// Create a provider from multiple explicit responses.
    pub fn from_responses(responses: Vec<CompletionResponse>) -> Self {
        Self {
            state: Arc::new(MockProviderState {
                responses,
                call_count: AtomicU32::new(0),
                fail_until: AtomicU32::new(0),
                fail_message: String::new(),
                delay: None,
            }),
        }
    }

    /// Create a provider that always fails with the given message.
    pub fn always_fail(msg: &str) -> Self {
        Self {
            state: Arc::new(MockProviderState {
                responses: vec![],
                call_count: AtomicU32::new(0),
                fail_until: AtomicU32::new(u32::MAX),
                fail_message: msg.to_string(),
                delay: None,
            }),
        }
    }

    /// Configure the provider to fail the first `n` calls, then succeed.
    pub fn with_fail_n(mut self, n: u32) -> Self {
        let state =
            Arc::get_mut(&mut self.state).expect("with_fail_n called on shared MockProvider");
        state.fail_until.store(n, Ordering::SeqCst);
        self
    }

    /// Add an artificial delay to every completion call.
    pub fn with_delay(mut self, delay: Duration) -> Self {
        let state =
            Arc::get_mut(&mut self.state).expect("with_delay called on shared MockProvider");
        state.delay = Some(delay);
        self
    }

    /// Configure a custom failure message.
    pub fn with_fail_message(mut self, msg: &str) -> Self {
        let state =
            Arc::get_mut(&mut self.state).expect("with_fail_message called on shared MockProvider");
        state.fail_message = msg.to_string();
        self
    }

    /// Number of times `complete()` has been called.
    pub fn call_count(&self) -> u32 {
        self.state.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    async fn complete(&self, _request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        if let Some(delay) = self.state.delay {
            tokio::time::sleep(delay).await;
        }

        let n = self.state.call_count.fetch_add(1, Ordering::SeqCst);
        let fail_until = self.state.fail_until.load(Ordering::SeqCst);
        if n < fail_until {
            return Err(anyhow::anyhow!(
                "{}",
                if self.state.fail_message.is_empty() {
                    "transient provider error"
                } else {
                    &self.state.fail_message
                }
            ));
        }

        let resp = self
            .state
            .responses
            .get(n as usize)
            .cloned()
            .unwrap_or(CompletionResponse {
                content: Some("default mock response".to_string()),
                tool_calls: None,
                usage: None,
                finish_reason: None,
            });
        Ok(resp)
    }

    async fn complete_stream(&self, request: CompletionRequest) -> anyhow::Result<BoxStream> {
        let resp = self.complete(request).await?;
        let chunk = CompletionChunk {
            delta: resp.content,
            tool_call_deltas: None,
            usage: resp.usage,
            done: true,
        };
        Ok(Box::pin(stream::once(async move { Ok(chunk) })))
    }

    fn supports_model(&self, _model: &str) -> bool {
        true
    }
}

/// Builder for creating complex mock scenarios.
pub struct MockProviderBuilder {
    responses: Vec<CompletionResponse>,
    fail_until: u32,
    fail_message: String,
    delay: Option<Duration>,
}

impl MockProviderBuilder {
    pub fn new() -> Self {
        Self {
            responses: vec![],
            fail_until: 0,
            fail_message: String::new(),
            delay: None,
        }
    }

    pub fn response(mut self, response: CompletionResponse) -> Self {
        self.responses.push(response);
        self
    }

    pub fn text(self, text: &str) -> Self {
        self.response(CompletionResponse {
            content: Some(text.to_string()),
            tool_calls: None,
            usage: Some(Usage {
                prompt_tokens: Some(10),
                completion_tokens: Some(5),
                total_tokens: Some(15),
            }),
            finish_reason: Some("stop".to_string()),
        })
    }

    pub fn fail_n(mut self, n: u32) -> Self {
        self.fail_until = n;
        self
    }

    pub fn fail_message(mut self, msg: &str) -> Self {
        self.fail_message = msg.to_string();
        self
    }

    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    pub fn build(self) -> MockProvider {
        MockProvider {
            state: Arc::new(MockProviderState {
                responses: self.responses,
                call_count: AtomicU32::new(0),
                fail_until: AtomicU32::new(self.fail_until),
                fail_message: self.fail_message,
                delay: self.delay,
            }),
        }
    }
}

impl Default for MockProviderBuilder {
    fn default() -> Self {
        Self::new()
    }
}
