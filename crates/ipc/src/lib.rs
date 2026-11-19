//! Versioned UI-to-host protocol and bounded named-pipe transport.

mod error;

pub mod client;
pub mod protocol;
pub mod server;

pub use error::IpcError;
