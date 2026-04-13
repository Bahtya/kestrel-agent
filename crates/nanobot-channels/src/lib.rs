//! # nanobot-channels
//!
//! Channel system — base trait, manager, registry, and platform implementations.

pub mod base;
pub mod commands;
pub mod manager;
pub mod platforms;
pub mod registry;

pub use base::BaseChannel;
pub use commands::CommandResponse;
pub use commands::{
    handle_history, handle_history_callback, handle_settings, handle_settings_callback,
    HISTORY_PER_PAGE, SETTINGS_PER_PAGE,
};
pub use manager::ChannelManager;
pub use platforms::telegram::{
    CallbackAction, CallbackContext, CallbackResponse, CallbackRouter, InlineKeyboardBuilder,
    InlineKeyboardButton, InlineKeyboardMarkup,
};
pub use platforms::websocket::WebSocketChannel;
pub use registry::ChannelRegistry;
