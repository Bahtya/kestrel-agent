//! # nanobot-tools
//!
//! Tool system with trait definition, registry, built-in tools, and skills.

pub mod builtins;
pub mod registry;
pub mod schema;
pub mod skills;
pub mod trait_def;

pub use registry::ToolRegistry;
pub use skills::{Skill, SkillParameter, SkillStore};
pub use trait_def::{Tool, ToolError};
