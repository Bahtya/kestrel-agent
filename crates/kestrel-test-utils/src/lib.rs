//! Shared test utilities and mock implementations for kestrel crates.
//!
//! Provides reusable mock implementations of core traits
//! (`LlmProvider`, `MemoryStore`, `HealthCheck`) to avoid duplication
//! across crate-level test modules.

pub mod mock_check;
pub mod mock_memory;
pub mod mock_provider;

pub use mock_check::{MockCheck, MockHealthCheck};
pub use mock_memory::MockMemoryStore;
pub use mock_provider::{MockProvider, MockProviderBuilder};
