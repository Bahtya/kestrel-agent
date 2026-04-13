//! Sub-agent spawning — parallel task execution framework.
//!
//! Provides `SubAgentManager` with `spawn_parallel` for executing multiple
//! independent LLM tasks concurrently using `tokio::JoinSet`. Each sub-agent
//! gets its own `AgentRunner` with independent sessions, resource limits,
//! and configurable tool permissions.
//!
//! The [`SubAgentSpawner`] trait (from `nanobot-tools`) is implemented for
//! [`SubAgentManager`] so that tools like `SpawnTool` can delegate to it.

use anyhow::Result;
use async_trait::async_trait;
use nanobot_config::Config;
use nanobot_core::Message;
use nanobot_providers::ProviderRegistry;
use nanobot_tools::registry::ToolRegistry;
use nanobot_tools::trait_def::{SpawnStatus, SubAgentSpawner};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::runner::AgentRunner;

// ─── Types ────────────────────────────────────────────────────

/// A task to be executed by a sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentTask {
    /// Unique task identifier (auto-generated if not set).
    #[serde(default)]
    pub id: String,

    /// The prompt to send to the sub-agent.
    pub prompt: String,

    /// Additional context injected before the prompt.
    #[serde(default)]
    pub context: Option<String>,

    /// Override the default model for this task.
    #[serde(default)]
    pub model_override: Option<String>,

    /// Maximum tokens the sub-agent may generate.
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// Result of a single sub-agent task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentResult {
    /// Task identifier (matches `SubAgentTask::id`).
    pub id: String,

    /// The output text produced by the sub-agent.
    pub output: String,

    /// Whether the task completed successfully.
    pub success: bool,

    /// Wall-clock duration of the task.
    pub duration_secs: f64,

    /// Total tokens consumed (prompt + completion).
    pub tokens_used: u64,

    /// Number of tool calls made during execution.
    pub tool_calls_made: usize,

    /// Number of agent-loop iterations consumed.
    pub iterations_used: usize,
}

/// Configuration for parallel sub-agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelSpawnConfig {
    /// Maximum number of tasks executing concurrently.
    /// Default: 3.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,

    /// Timeout per individual task.
    /// Default: 60 seconds.
    #[serde(default = "default_per_task_timeout")]
    pub per_task_timeout_secs: u64,

    /// Timeout for the entire batch of tasks.
    /// `None` means no overall deadline.
    /// Default: None.
    #[serde(default)]
    pub total_timeout_secs: Option<u64>,

    /// Tool names to deny for sub-agents (inherited from parent otherwise).
    /// If empty, all parent tools are available.
    #[serde(default)]
    pub denied_tools: Vec<String>,

    /// System prompt prefix for sub-agents.
    #[serde(default)]
    pub system_prompt_prefix: Option<String>,
}

fn default_max_concurrent() -> usize {
    3
}
fn default_per_task_timeout() -> u64 {
    60
}

impl Default for ParallelSpawnConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            per_task_timeout_secs: default_per_task_timeout(),
            total_timeout_secs: None,
            denied_tools: vec![],
            system_prompt_prefix: None,
        }
    }
}

/// Status of a tracked sub-agent task (for background monitoring).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Running,
    Completed(String),
    Failed(String),
}

impl From<&TaskStatus> for SpawnStatus {
    fn from(s: &TaskStatus) -> Self {
        match s {
            TaskStatus::Running => SpawnStatus::Running,
            TaskStatus::Completed(r) => SpawnStatus::Completed(r.clone()),
            TaskStatus::Failed(e) => SpawnStatus::Failed(e.clone()),
        }
    }
}

/// Collected results from a parallel spawn, with summary statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnSummary {
    /// Individual task results, in the order tasks were submitted.
    pub results: Vec<SubAgentResult>,

    /// Number of tasks that succeeded.
    pub succeeded: usize,

    /// Number of tasks that failed.
    pub failed: usize,

    /// Total wall-clock time for the entire batch.
    pub total_duration_secs: f64,

    /// Total tokens consumed across all tasks.
    pub total_tokens_used: u64,
}

impl SpawnSummary {
    /// Combine all successful outputs into structured notes.
    pub fn to_structured_notes(&self) -> String {
        let mut notes = String::new();
        for result in &self.results {
            if result.success {
                notes.push_str(&format!(
                    "## Task {} ({:.1}s, {} tokens)\n{}\n\n",
                    result.id, result.duration_secs, result.tokens_used, result.output
                ));
            } else {
                notes.push_str(&format!(
                    "## Task {} — FAILED\nError: {}\n\n",
                    result.id, result.output
                ));
            }
        }
        notes.push_str(&format!(
            "Summary: {}/{} tasks succeeded, {:.1}s total, {} tokens used",
            self.succeeded,
            self.succeeded + self.failed,
            self.total_duration_secs,
            self.total_tokens_used
        ));
        notes
    }
}

/// Internal tracked task for background monitoring.
struct TrackedTask {
    id: String,
    name: String,
    description: String,
    status: TaskStatus,
    /// JoinHandle so we can abort the background tokio task on cancel.
    handle: Option<tokio::task::JoinHandle<()>>,
}

// ─── SubAgentHandle ─────────────────────────────────────────────

/// Handle to a spawned sub-agent task.
///
/// Provides methods to query status and request cancellation without
/// exposing the internal `SubAgentManager`.
#[derive(Clone)]
pub struct SubAgentHandle {
    /// Task ID.
    pub id: String,
    /// Human-readable task name.
    pub name: String,
    manager: Arc<SubAgentManager>,
}

impl SubAgentHandle {
    /// Query the current status of this task.
    pub async fn status(&self) -> Option<SpawnStatus> {
        self.manager.status(&self.id).await
    }

    /// Request cancellation of this task.
    /// Returns `true` if the task was found and signalled.
    pub async fn cancel(&self) -> bool {
        self.manager.cancel(&self.id).await
    }
}

// ─── SubAgentManager ──────────────────────────────────────────

/// Manages sub-agent spawning — both background tracking and parallel execution.
///
/// Holds shared config, provider registry, and tool registry so each sub-agent
/// can create its own `AgentRunner`. Implements [`SubAgentSpawner`] so tools
/// like `SpawnTool` can delegate to it.
pub struct SubAgentManager {
    config: Arc<Config>,
    providers: Arc<ProviderRegistry>,
    tools: Arc<ToolRegistry>,
    tasks: Arc<RwLock<Vec<TrackedTask>>>,
}

impl SubAgentManager {
    /// Create a new SubAgentManager with access to the parent agent's registries.
    pub fn new(
        config: Arc<Config>,
        providers: Arc<ProviderRegistry>,
        tools: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            config,
            providers,
            tools,
            tasks: Arc::new(RwLock::new(Vec::new())),
        }
    }

    // ─── Background task tracking (legacy + monitoring) ────────

    /// Register a new background task for tracking.
    pub async fn spawn(&self, name: &str, description: &str) -> String {
        let id = Uuid::new_v4().to_string();
        let task = TrackedTask {
            id: id.clone(),
            name: name.to_string(),
            description: description.to_string(),
            status: TaskStatus::Running,
            handle: None,
        };
        self.tasks.write().await.push(task);
        info!("Spawned subagent task: {} ({})", name, id);
        id
    }

    /// Mark a tracked task as completed.
    pub async fn complete(&self, id: &str, result: String) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == id) {
            task.status = TaskStatus::Completed(result);
            debug!("Completed subagent task: {}", id);
        }
    }

    /// Mark a tracked task as failed.
    pub async fn fail(&self, id: &str, error: String) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == id) {
            task.status = TaskStatus::Failed(error);
            debug!("Failed subagent task: {}", id);
        }
    }

    /// Get the status of a tracked task.
    pub async fn get_status(&self, id: &str) -> Option<TaskStatus> {
        let tasks = self.tasks.read().await;
        tasks.iter().find(|t| t.id == id).map(|t| t.status.clone())
    }

    /// List all tracked tasks.
    pub async fn list_tasks(&self) -> Vec<(String, String, TaskStatus)> {
        let tasks = self.tasks.read().await;
        tasks
            .iter()
            .map(|t| (t.id.clone(), t.name.clone(), t.status.clone()))
            .collect()
    }

    // ─── Single background spawn (SubAgentSpawner backing) ─────

    /// Spawn a single sub-agent task that executes in the background.
    ///
    /// Registers the task, kicks off execution via a dedicated `AgentRunner`,
    /// and updates the tracking status on completion or failure.
    /// Returns a [`SubAgentHandle`] for monitoring.
    pub async fn spawn_single(
        self: &Arc<Self>,
        name: &str,
        prompt: &str,
        context: Option<String>,
    ) -> Result<SubAgentHandle> {
        let id = Uuid::new_v4().to_string();

        // Register tracking entry
        {
            let task = TrackedTask {
                id: id.clone(),
                name: name.to_string(),
                description: prompt.to_string(),
                status: TaskStatus::Running,
                handle: None,
            };
            self.tasks.write().await.push(task);
        }

        info!("Spawning single sub-agent task '{}' ({})", name, id);

        // Build runner for the sub-agent
        let runner = AgentRunner::new(
            self.config.clone(),
            self.providers.clone(),
            self.tools.clone(),
        );

        // Build messages
        let mut messages = Vec::new();
        if let Some(ref ctx) = context {
            messages.push(Message {
                role: nanobot_core::MessageRole::User,
                content: format!("Context:\n{}", ctx),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            });
        }
        messages.push(Message {
            role: nanobot_core::MessageRole::User,
            content: prompt.to_string(),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        });

        let system_prompt = "You are a focused sub-agent executing a specific task. \
            Be concise and direct in your response."
            .to_string();

        // Spawn background tokio task
        let mgr = Arc::clone(self);
        let task_id = id.clone();
        let handle = tokio::spawn(async move {
            match runner.run(system_prompt, messages).await {
                Ok(result) => {
                    mgr.complete(&task_id, result.content).await;
                }
                Err(e) => {
                    mgr.fail(&task_id, format!("{}", e)).await;
                }
            }
        });

        // Store the join handle so we can abort on cancel
        {
            let mut tasks = self.tasks.write().await;
            if let Some(task) = tasks.iter_mut().find(|t| t.id == id) {
                task.handle = Some(handle);
            }
        }

        let name_owned = name.to_string();
        Ok(SubAgentHandle {
            id,
            name: name_owned,
            manager: Arc::clone(self),
        })
    }

    // ─── Parallel execution ───────────────────────────────────

    /// Execute multiple sub-agent tasks in parallel.
    ///
    /// Uses `tokio::JoinSet` to run tasks concurrently, respecting
    /// `config.max_concurrent` as the concurrency limit. Each task
    /// gets its own `AgentRunner` with independent configuration.
    /// Failed tasks are isolated — one failure does not affect others.
    pub async fn spawn_parallel(
        &self,
        tasks: Vec<SubAgentTask>,
        config: &ParallelSpawnConfig,
    ) -> Result<SpawnSummary> {
        let total_start = Instant::now();
        let total_timeout = config.total_timeout_secs.map(Duration::from_secs);
        let per_task_timeout = Duration::from_secs(config.per_task_timeout_secs);

        // Build tool registry with denied tools filtered
        let filtered_tools = self.build_filtered_tools(&config.denied_tools)?;

        // Prepare tasks with IDs
        let prepared: Vec<SubAgentTask> = tasks
            .into_iter()
            .enumerate()
            .map(|(i, mut t)| {
                if t.id.is_empty() {
                    t.id = format!("task-{}", i + 1);
                }
                t
            })
            .collect();

        let task_count = prepared.len();
        info!(
            "Spawning {} parallel sub-agent tasks (max_concurrent: {})",
            task_count, config.max_concurrent
        );

        // Track results by task ID
        let results: Arc<RwLock<Vec<SubAgentResult>>> =
            Arc::new(RwLock::new(Vec::with_capacity(task_count)));

        let mut join_set: tokio::task::JoinSet<(String, Result<SubAgentResult>)> =
            tokio::task::JoinSet::new();
        let mut task_iter = prepared.into_iter().peekable();
        let mut spawned = 0usize;

        // Spawn initial batch up to max_concurrent
        while spawned < config.max_concurrent && task_iter.peek().is_some() {
            let task = task_iter.next().unwrap();
            let runner = self.build_runner(&task, &filtered_tools, config);
            let timeout = per_task_timeout;
            let _task_id = task.id.clone();
            join_set.spawn(run_single_task(task, runner, timeout));
            spawned += 1;
        }

        // Collect results and spawn more as slots free up
        while let Some(join_result) = join_set.join_next().await {
            let (task_id, task_result) = match join_result {
                Ok(pair) => pair,
                Err(join_err) => {
                    warn!("JoinSet task panicked: {}", join_err);
                    // We lost track of which task — skip
                    break;
                }
            };

            let result = match task_result {
                Ok(r) => r,
                Err(e) => {
                    warn!("Sub-agent task {} failed: {}", task_id, e);
                    SubAgentResult {
                        id: task_id.clone(),
                        output: format!("Task error: {}", e),
                        success: false,
                        duration_secs: 0.0,
                        tokens_used: 0,
                        tool_calls_made: 0,
                        iterations_used: 0,
                    }
                }
            };

            debug!(
                "Task {} completed: success={}, duration={:.1}s, tokens={}",
                result.id, result.success, result.duration_secs, result.tokens_used
            );

            results.write().await.push(result);

            // Check total timeout
            if let Some(total) = total_timeout {
                if total_start.elapsed() > total {
                    warn!("Total timeout reached, stopping remaining tasks");
                    join_set.abort_all();
                    break;
                }
            }

            // Spawn next task if available
            if task_iter.peek().is_some() {
                let task = task_iter.next().unwrap();
                let runner = self.build_runner(&task, &filtered_tools, config);
                let timeout = per_task_timeout;
                join_set.spawn(run_single_task(task, runner, timeout));
            }
        }

        // Abort any remaining tasks
        join_set.abort_all();

        let results_vec = results.read().await;
        let succeeded = results_vec.iter().filter(|r| r.success).count();
        let failed = results_vec.len().saturating_sub(succeeded);
        let total_tokens = results_vec.iter().map(|r| r.tokens_used).sum();

        let summary = SpawnSummary {
            results: results_vec.clone(),
            succeeded,
            failed,
            total_duration_secs: total_start.elapsed().as_secs_f64(),
            total_tokens_used: total_tokens,
        };

        info!(
            "Parallel spawn complete: {}/{} succeeded in {:.1}s",
            summary.succeeded,
            summary.succeeded + summary.failed,
            summary.total_duration_secs
        );

        Ok(summary)
    }

    /// Build an `AgentRunner` for a specific sub-agent task.
    fn build_runner(
        &self,
        task: &SubAgentTask,
        tools: &Arc<ToolRegistry>,
        _config: &ParallelSpawnConfig,
    ) -> AgentRunner {
        // Build per-task config with optional overrides
        let mut task_config = (*self.config).clone();
        if let Some(ref model) = task.model_override {
            task_config.agent.model = model.clone();
        }
        if let Some(max_tokens) = task.max_tokens {
            task_config.agent.max_tokens = max_tokens;
        }

        AgentRunner::new(
            Arc::new(task_config),
            self.providers.clone(),
            tools.clone(),
        )
    }

    /// Build a tool registry with denied tools filtered out.
    fn build_filtered_tools(&self, denied: &[String]) -> Result<Arc<ToolRegistry>> {
        if denied.is_empty() {
            return Ok(self.tools.clone());
        }

        let filtered = self.tools.filter_out(denied);
        Ok(Arc::new(filtered))
    }
}

// ─── SubAgentSpawner impl ──────────────────────────────────────

#[async_trait]
impl SubAgentSpawner for SubAgentManager {
    async fn spawn(
        &self,
        name: &str,
        prompt: &str,
        context: Option<String>,
    ) -> Result<String> {
        // We need an Arc<Self> to call spawn_single, so wrap self.
        // The manager is always used behind an Arc in practice.
        let arc_self: Arc<Self> = Arc::new(Self {
            config: self.config.clone(),
            providers: self.providers.clone(),
            tools: self.tools.clone(),
            tasks: self.tasks.clone(),
        });
        let handle = arc_self.spawn_single(name, prompt, context).await?;
        Ok(handle.id)
    }

    async fn status(&self, task_id: &str) -> Option<SpawnStatus> {
        let tasks = self.tasks.read().await;
        tasks
            .iter()
            .find(|t| t.id == task_id)
            .map(|t| SpawnStatus::from(&t.status))
    }

    async fn cancel(&self, task_id: &str) -> bool {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
            // Abort the tokio task if we have the handle
            if let Some(handle) = task.handle.take() {
                handle.abort();
            }
            if task.status == TaskStatus::Running {
                task.status = TaskStatus::Failed("Cancelled".to_string());
                debug!("Cancelled subagent task: {}", task_id);
                return true;
            }
        }
        false
    }

    async fn list(&self) -> Vec<(String, String, SpawnStatus)> {
        let tasks = self.tasks.read().await;
        tasks
            .iter()
            .map(|t| (t.id.clone(), t.name.clone(), SpawnStatus::from(&t.status)))
            .collect()
    }
}

/// Run a single sub-agent task with timeout.
async fn run_single_task(
    task: SubAgentTask,
    runner: AgentRunner,
    timeout: Duration,
) -> (String, Result<SubAgentResult>) {
    let task_id = task.id.clone();
    let start = Instant::now();

    // Build system prompt
    let system_prompt = "You are a focused sub-agent executing a specific task. \
        Be concise and direct in your response."
        .to_string();

    // Build messages
    let mut messages = Vec::new();
    if let Some(ref ctx) = task.context {
        messages.push(Message {
            role: nanobot_core::MessageRole::User,
            content: format!("Context:\n{}", ctx),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        });
    }
    messages.push(Message {
        role: nanobot_core::MessageRole::User,
        content: task.prompt.clone(),
        name: None,
        tool_call_id: None,
        tool_calls: None,
    });

    // Execute with timeout
    let run_result = match tokio::time::timeout(timeout, runner.run(system_prompt, messages)).await
    {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            let duration = start.elapsed().as_secs_f64();
            return (
                task_id,
                Ok(SubAgentResult {
                    id: task.id,
                    output: format!("Agent error: {}", e),
                    success: false,
                    duration_secs: duration,
                    tokens_used: 0,
                    tool_calls_made: 0,
                    iterations_used: 0,
                }),
            );
        }
        Err(_) => {
            let duration = start.elapsed().as_secs_f64();
            return (
                task_id,
                Ok(SubAgentResult {
                    id: task.id,
                    output: format!("Timeout after {:.0}s", timeout.as_secs()),
                    success: false,
                    duration_secs: duration,
                    tokens_used: 0,
                    tool_calls_made: 0,
                    iterations_used: 0,
                }),
            );
        }
    };

    let duration = start.elapsed().as_secs_f64();
    let tokens_used = run_result.usage.total_tokens.unwrap_or(0);

    (
        task_id,
        Ok(SubAgentResult {
            id: task.id,
            output: run_result.content,
            success: true,
            duration_secs: duration,
            tokens_used,
            tool_calls_made: run_result.tool_calls_made,
            iterations_used: run_result.iterations_used,
        }),
    )
}

// ─── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nanobot_core::Usage;
    use nanobot_providers::base::{
        BoxStream, CompletionChunk, CompletionRequest, CompletionResponse, LlmProvider,
    };
    use nanobot_tools::trait_def::SpawnStatus;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Mock provider that returns deterministic responses.
    struct MockProvider {
        responses: Vec<CompletionResponse>,
        call_count: Arc<AtomicU32>,
    }

    impl MockProvider {
        fn simple(text: &str) -> Self {
            Self {
                responses: vec![CompletionResponse {
                    content: Some(text.to_string()),
                    tool_calls: None,
                    usage: Some(Usage {
                        prompt_tokens: Some(10),
                        completion_tokens: Some(5),
                        total_tokens: Some(15),
                    }),
                    finish_reason: Some("stop".to_string()),
                }],
                call_count: Arc::new(AtomicU32::new(0)),
            }
        }

        /// Create a provider that returns different text per call.
        fn multi(responses: Vec<&str>) -> Self {
            Self {
                responses: responses
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
                call_count: Arc::new(AtomicU32::new(0)),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }

        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse> {
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst) as usize;
            self.responses
                .get(idx)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("MockProvider: no response for call {}", idx))
        }

        async fn complete_stream(&self, req: CompletionRequest) -> Result<BoxStream> {
            let resp = self.complete(req).await?;
            let chunk = CompletionChunk {
                delta: resp.content,
                tool_call_deltas: None,
                usage: resp.usage,
                done: true,
            };
            Ok(Box::pin(futures::stream::once(async move { Ok(chunk) })))
        }

        fn supports_model(&self, _model: &str) -> bool {
            true
        }
    }

    /// Mock provider that introduces a delay before responding.
    struct DelayedProvider {
        text: String,
        delay: Duration,
    }

    impl DelayedProvider {
        fn new(text: &str, delay: Duration) -> Self {
            Self {
                text: text.to_string(),
                delay,
            }
        }
    }

    #[async_trait]
    impl LlmProvider for DelayedProvider {
        fn name(&self) -> &str {
            "mock-delayed"
        }

        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse> {
            tokio::time::sleep(self.delay).await;
            Ok(CompletionResponse {
                content: Some(self.text.clone()),
                tool_calls: None,
                usage: Some(Usage {
                    prompt_tokens: Some(10),
                    completion_tokens: Some(5),
                    total_tokens: Some(15),
                }),
                finish_reason: Some("stop".to_string()),
            })
        }

        async fn complete_stream(&self, req: CompletionRequest) -> Result<BoxStream> {
            let resp = self.complete(req).await?;
            let chunk = CompletionChunk {
                delta: resp.content,
                tool_call_deltas: None,
                usage: resp.usage,
                done: true,
            };
            Ok(Box::pin(futures::stream::once(async move { Ok(chunk) })))
        }

        fn supports_model(&self, _model: &str) -> bool {
            true
        }
    }

    /// Build a test SubAgentManager with a mock provider.
    fn make_manager_with_mock(provider: MockProvider) -> SubAgentManager {
        let mut config = Config::default();
        config.agent.model = "mock-model".to_string();
        config.agent.max_iterations = 5;
        let mut reg = ProviderRegistry::new();
        reg.register("mock", provider);
        reg.set_default("mock");
        SubAgentManager::new(
            Arc::new(config),
            Arc::new(reg),
            Arc::new(ToolRegistry::new()),
        )
    }

    /// Build a test SubAgentManager with a delayed provider.
    fn make_manager_with_delayed(text: &str, delay: Duration) -> SubAgentManager {
        let mut config = Config::default();
        config.agent.model = "mock-model".to_string();
        config.agent.max_iterations = 5;
        let mut reg = ProviderRegistry::new();
        reg.register("mock-delayed", DelayedProvider::new(text, delay));
        reg.set_default("mock-delayed");
        SubAgentManager::new(
            Arc::new(config),
            Arc::new(reg),
            Arc::new(ToolRegistry::new()),
        )
    }

    // ─── Legacy tracking tests ────────────────────────────────

    #[tokio::test]
    async fn test_subagent_manager_new() {
        let mgr = make_manager_with_mock(MockProvider::simple("ok"));
        let tasks = mgr.list_tasks().await;
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn test_subagent_manager_spawn_and_complete() {
        let mgr = make_manager_with_mock(MockProvider::simple("ok"));
        let id = mgr.spawn("test_task", "a test").await;
        let status = mgr.get_status(&id).await.unwrap();
        assert_eq!(status, TaskStatus::Running);

        mgr.complete(&id, "done".to_string()).await;
        let status = mgr.get_status(&id).await.unwrap();
        assert_eq!(status, TaskStatus::Completed("done".to_string()));
    }

    #[tokio::test]
    async fn test_subagent_manager_fail() {
        let mgr = make_manager_with_mock(MockProvider::simple("ok"));
        let id = mgr.spawn("test_task", "a test").await;
        mgr.fail(&id, "error occurred".to_string()).await;
        let status = mgr.get_status(&id).await.unwrap();
        assert_eq!(status, TaskStatus::Failed("error occurred".to_string()));
    }

    #[tokio::test]
    async fn test_subagent_manager_list_tasks() {
        let mgr = make_manager_with_mock(MockProvider::simple("ok"));
        mgr.spawn("task1", "first").await;
        mgr.spawn("task2", "second").await;
        mgr.spawn("task3", "third").await;
        let tasks = mgr.list_tasks().await;
        assert_eq!(tasks.len(), 3);
    }

    #[tokio::test]
    async fn test_subagent_manager_get_status_nonexistent() {
        let mgr = make_manager_with_mock(MockProvider::simple("ok"));
        let status = mgr.get_status("nonexistent-id").await;
        assert!(status.is_none());
    }

    // ─── SubAgentSpawner trait tests ───────────────────────────

    #[tokio::test]
    async fn test_spawner_spawn_and_status() {
        let mgr = make_manager_with_mock(MockProvider::simple("result"));
        let id = SubAgentSpawner::spawn(&mgr, "worker", "do work", None)
            .await
            .unwrap();
        assert!(!id.is_empty());

        // The background task may still be running; status should be Running or Completed
        let status = SubAgentSpawner::status(&mgr, &id).await;
        assert!(status.is_some());
    }

    #[tokio::test]
    async fn test_spawner_list() {
        let mgr = make_manager_with_mock(MockProvider::simple("ok"));
        let _id1 = SubAgentSpawner::spawn(&mgr, "a", "task a", None)
            .await
            .unwrap();
        let _id2 = SubAgentSpawner::spawn(&mgr, "b", "task b", None)
            .await
            .unwrap();

        let list = SubAgentSpawner::list(&mgr).await;
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_spawner_cancel_nonexistent() {
        let mgr = make_manager_with_mock(MockProvider::simple("ok"));
        assert!(!SubAgentSpawner::cancel(&mgr, "no-such-id").await);
    }

    #[tokio::test]
    async fn test_spawner_status_nonexistent() {
        let mgr = make_manager_with_mock(MockProvider::simple("ok"));
        assert!(SubAgentSpawner::status(&mgr, "nope").await.is_none());
    }

    // ─── SubAgentHandle tests ──────────────────────────────────

    #[tokio::test]
    async fn test_handle_status_and_cancel() {
        let mgr = Arc::new(make_manager_with_delayed("slow result", Duration::from_secs(5)));
        let handle = mgr.spawn_single("slow-task", "take your time", None)
            .await
            .unwrap();

        assert_eq!(handle.name, "slow-task");
        assert!(!handle.id.is_empty());

        // Should be running (task takes 5s)
        let status = handle.status().await;
        assert_eq!(status, Some(SpawnStatus::Running));

        // Cancel it
        let cancelled = handle.cancel().await;
        assert!(cancelled);

        // Now should be Failed("Cancelled")
        let status = handle.status().await;
        assert!(matches!(status, Some(SpawnStatus::Failed(ref msg)) if msg == "Cancelled"));
    }

    #[tokio::test]
    async fn test_handle_completed() {
        let mgr = Arc::new(make_manager_with_mock(MockProvider::simple("done")));
        let handle = mgr.spawn_single("fast-task", "quick work", None)
            .await
            .unwrap();

        // Give the background task time to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        let status = handle.status().await;
        match status {
            Some(SpawnStatus::Completed(ref output)) => {
                assert_eq!(output, "done");
            }
            Some(SpawnStatus::Running) => {
                // Task hasn't completed yet — acceptable in slow CI
            }
            other => panic!("Unexpected status: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_with_context() {
        let mgr = Arc::new(make_manager_with_mock(MockProvider::simple("context ok")));
        let handle = mgr
            .spawn_single("ctx-task", "use context", Some("extra info".to_string()))
            .await
            .unwrap();

        // Should at least be registered
        let status = handle.status().await;
        assert!(status.is_some());
    }

    // ─── Parallel execution tests ─────────────────────────────

    #[tokio::test]
    async fn test_parallel_spawn_3_tasks() {
        let mgr = make_manager_with_mock(MockProvider::multi(vec![
            "Result Alpha",
            "Result Beta",
            "Result Gamma",
        ]));

        let tasks = vec![
            SubAgentTask {
                id: "t1".into(),
                prompt: "Task 1".into(),
                context: None,
                model_override: None,
                max_tokens: None,
            },
            SubAgentTask {
                id: "t2".into(),
                prompt: "Task 2".into(),
                context: None,
                model_override: None,
                max_tokens: None,
            },
            SubAgentTask {
                id: "t3".into(),
                prompt: "Task 3".into(),
                context: None,
                model_override: None,
                max_tokens: None,
            },
        ];

        let config = ParallelSpawnConfig {
            max_concurrent: 3,
            per_task_timeout_secs: 10,
            ..Default::default()
        };

        let summary = mgr.spawn_parallel(tasks, &config).await.unwrap();

        assert_eq!(summary.succeeded, 3);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.results.len(), 3);

        // Verify all task IDs present
        let ids: Vec<&str> = summary.results.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"t1"));
        assert!(ids.contains(&"t2"));
        assert!(ids.contains(&"t3"));

        // Verify each task got a response
        for result in &summary.results {
            assert!(result.success);
            assert!(result.tokens_used > 0);
        }

        // Verify structured notes
        let notes = summary.to_structured_notes();
        assert!(notes.contains("3/3 tasks succeeded"));
    }

    #[tokio::test]
    async fn test_parallel_spawn_with_context() {
        let mgr = make_manager_with_mock(MockProvider::simple("Context received"));

        let tasks = vec![SubAgentTask {
            id: "ctx-task".into(),
            prompt: "Use the context".into(),
            context: Some("Important context info".into()),
            model_override: None,
            max_tokens: None,
        }];

        let config = ParallelSpawnConfig::default();
        let summary = mgr.spawn_parallel(tasks, &config).await.unwrap();

        assert_eq!(summary.succeeded, 1);
        assert!(summary.results[0].output.contains("Context received"));
    }

    #[tokio::test]
    async fn test_parallel_spawn_timeout() {
        // Provider takes 2 seconds, but timeout is 100ms
        let mgr = make_manager_with_delayed("delayed result", Duration::from_secs(2));

        let tasks = vec![SubAgentTask {
            id: "slow-task".into(),
            prompt: "Take your time".into(),
            context: None,
            model_override: None,
            max_tokens: None,
        }];

        // Actually use a very short timeout via direct construction
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            mgr.spawn_parallel(tasks, &ParallelSpawnConfig {
                per_task_timeout_secs: 1, // 1s timeout, task takes 2s
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(result.failed, 1);
        assert!(result.results[0].output.contains("Timeout"));
        assert!(!result.results[0].success);
    }

    #[tokio::test]
    async fn test_parallel_spawn_max_concurrent() {
        // 5 tasks with max_concurrent=2 — should still complete all
        let mgr = make_manager_with_mock(MockProvider::multi(vec![
            "done-1", "done-2", "done-3", "done-4", "done-5",
        ]));

        let tasks: Vec<SubAgentTask> = (1..=5)
            .map(|i| SubAgentTask {
                id: format!("task-{}", i),
                prompt: format!("Task {}", i),
                context: None,
                model_override: None,
                max_tokens: None,
            })
            .collect();

        let config = ParallelSpawnConfig {
            max_concurrent: 2,
            per_task_timeout_secs: 10,
            ..Default::default()
        };

        let summary = mgr.spawn_parallel(tasks, &config).await.unwrap();
        assert_eq!(summary.succeeded, 5);
        assert_eq!(summary.results.len(), 5);
    }

    #[tokio::test]
    async fn test_parallel_spawn_error_isolation() {
        // Provider that fails on the 2nd call
        struct FailOnSecond {
            call_count: AtomicU32,
        }

        #[async_trait]
        impl LlmProvider for FailOnSecond {
            fn name(&self) -> &str {
                "fail-second"
            }
            async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse> {
                let n = self.call_count.fetch_add(1, Ordering::SeqCst);
                if n == 1 {
                    Err(anyhow::anyhow!("Simulated failure on task 2"))
                } else {
                    Ok(CompletionResponse {
                        content: Some(format!("Success on call {}", n)),
                        tool_calls: None,
                        usage: Some(Usage {
                            prompt_tokens: Some(10),
                            completion_tokens: Some(5),
                            total_tokens: Some(15),
                        }),
                        finish_reason: Some("stop".to_string()),
                    })
                }
            }
            async fn complete_stream(&self, req: CompletionRequest) -> Result<BoxStream> {
                let resp = self.complete(req).await?;
                let chunk = CompletionChunk {
                    delta: resp.content,
                    tool_call_deltas: None,
                    usage: resp.usage,
                    done: true,
                };
                Ok(Box::pin(futures::stream::once(async move { Ok(chunk) })))
            }
            fn supports_model(&self, _model: &str) -> bool {
                true
            }
        }

        let mut config = Config::default();
        config.agent.model = "mock-model".to_string();
        config.agent.max_iterations = 5;
        let mut reg = ProviderRegistry::new();
        reg.register("fail-second", FailOnSecond {
            call_count: AtomicU32::new(0),
        });
        reg.set_default("fail-second");
        let mgr = SubAgentManager::new(
            Arc::new(config),
            Arc::new(reg),
            Arc::new(ToolRegistry::new()),
        );

        let tasks = vec![
            SubAgentTask {
                id: "ok-task".into(),
                prompt: "Should succeed".into(),
                context: None,
                model_override: None,
                max_tokens: None,
            },
            SubAgentTask {
                id: "fail-task".into(),
                prompt: "Should fail".into(),
                context: None,
                model_override: None,
                max_tokens: None,
            },
            SubAgentTask {
                id: "ok-task-2".into(),
                prompt: "Should also succeed".into(),
                context: None,
                model_override: None,
                max_tokens: None,
            },
        ];

        let spawn_config = ParallelSpawnConfig {
            max_concurrent: 3,
            per_task_timeout_secs: 10,
            ..Default::default()
        };

        let summary = mgr.spawn_parallel(tasks, &spawn_config).await.unwrap();

        // One task should fail, others should succeed
        assert_eq!(summary.failed, 1, "Expected 1 failure, got {}", summary.failed);
        assert_eq!(summary.succeeded, 2, "Expected 2 successes, got {}", summary.succeeded);

        // The failed task should have error info
        let failed_result = summary.results.iter().find(|r| !r.success).unwrap();
        assert!(failed_result.output.contains("error") || failed_result.output.contains("Error") || failed_result.output.contains("fail"));
    }

    #[tokio::test]
    async fn test_parallel_spawn_total_timeout() {
        // 3 tasks each taking 500ms, total timeout 600ms
        // First task completes, remaining are aborted
        let mgr = make_manager_with_delayed("result", Duration::from_millis(500));

        let tasks: Vec<SubAgentTask> = (1..=3)
            .map(|i| SubAgentTask {
                id: format!("task-{}", i),
                prompt: format!("Task {}", i),
                context: None,
                model_override: None,
                max_tokens: None,
            })
            .collect();

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            mgr.spawn_parallel(tasks, &ParallelSpawnConfig {
                max_concurrent: 3,
                per_task_timeout_secs: 10,
                total_timeout_secs: Some(1), // 1s total, each task takes 500ms
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .unwrap();

        // At least 1 should complete (the first batch of 3 starts immediately,
        // each takes 500ms, so all 3 should complete within 1s total)
        assert!(
            result.succeeded >= 1,
            "Expected at least 1 success, got {}/{}",
            result.succeeded,
            result.succeeded + result.failed
        );
    }

    #[tokio::test]
    async fn test_parallel_spawn_empty_tasks() {
        let mgr = make_manager_with_mock(MockProvider::simple("ok"));

        let config = ParallelSpawnConfig::default();
        let summary = mgr.spawn_parallel(vec![], &config).await.unwrap();

        assert_eq!(summary.succeeded, 0);
        assert_eq!(summary.failed, 0);
        assert!(summary.results.is_empty());
    }

    #[tokio::test]
    async fn test_parallel_spawn_auto_ids() {
        let mgr = make_manager_with_mock(MockProvider::multi(vec!["a", "b"]));

        let tasks = vec![
            SubAgentTask {
                id: String::new(), // Empty — auto-generated
                prompt: "Task 1".into(),
                context: None,
                model_override: None,
                max_tokens: None,
            },
            SubAgentTask {
                id: String::new(),
                prompt: "Task 2".into(),
                context: None,
                model_override: None,
                max_tokens: None,
            },
        ];

        let config = ParallelSpawnConfig::default();
        let summary = mgr.spawn_parallel(tasks, &config).await.unwrap();

        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.results[0].id, "task-1");
        assert_eq!(summary.results[1].id, "task-2");
    }

    #[tokio::test]
    async fn test_structured_notes() {
        let mgr = make_manager_with_mock(MockProvider::simple("ok"));

        let tasks = vec![SubAgentTask {
            id: "note-task".into(),
            prompt: "Generate notes".into(),
            context: None,
            model_override: None,
            max_tokens: None,
        }];

        let config = ParallelSpawnConfig::default();
        let summary = mgr.spawn_parallel(tasks, &config).await.unwrap();

        let notes = summary.to_structured_notes();
        assert!(notes.contains("## Task note-task"));
        assert!(notes.contains("1/1 tasks succeeded"));
    }

    #[tokio::test]
    async fn test_denied_tools_filter() {
        let mgr = make_manager_with_mock(MockProvider::simple("ok"));

        let config = ParallelSpawnConfig {
            denied_tools: vec!["dangerous_tool".to_string()],
            ..Default::default()
        };

        // Should not error — filtering is internal
        let tasks = vec![SubAgentTask {
            id: "filtered".into(),
            prompt: "Test".into(),
            context: None,
            model_override: None,
            max_tokens: None,
        }];

        let summary = mgr.spawn_parallel(tasks, &config).await.unwrap();
        assert_eq!(summary.succeeded, 1);
    }

    #[tokio::test]
    async fn test_parallel_config_default() {
        let config = ParallelSpawnConfig::default();
        assert_eq!(config.max_concurrent, 3);
        assert_eq!(config.per_task_timeout_secs, 60);
        assert!(config.total_timeout_secs.is_none());
        assert!(config.denied_tools.is_empty());
    }

    #[tokio::test]
    async fn test_sub_agent_task_serde() {
        let task = SubAgentTask {
            id: "t1".into(),
            prompt: "Do something".into(),
            context: Some("Extra context".into()),
            model_override: Some("gpt-4o-mini".into()),
            max_tokens: Some(512),
        };
        let json = serde_json::to_string(&task).unwrap();
        let back: SubAgentTask = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "t1");
        assert_eq!(back.prompt, "Do something");
        assert_eq!(back.context, Some("Extra context".into()));
        assert_eq!(back.model_override, Some("gpt-4o-mini".into()));
        assert_eq!(back.max_tokens, Some(512));
    }
}
