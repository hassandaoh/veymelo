//! The MCP server: how an agent works in Veymelo.
//!
//! Two ways: on its own (`veymelo mcp`), with a session of its own and no window, or bridged
//! (`veymelo mcp --bridge`) to the open app, where the person sees every change as it is made.
//! Either way the same few tools find, describe and run the program's commands, read the
//! project and show frames.

pub mod activity;
mod base64;
pub mod library;
pub mod channel;
pub mod server;
pub mod tools;

pub use channel::{Bridge, Channel, bridged, default_file};
pub use library::{Agent, Library};
pub use server::{Backend, Local, PROTOCOL, handle, serve};
pub use tools::{Content, Reply, TOOLS};
