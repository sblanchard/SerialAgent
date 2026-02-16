//! `sa-node-sdk` — Reusable SDK for building SerialAgent nodes.
//!
//! A "node" is any process that connects to the SerialAgent gateway via
//! WebSocket, advertises capabilities, and executes tool requests.  This
//! crate provides the building blocks so node authors don't need to
//! re-implement connection management, authentication, heartbeat, or the
//! request/response multiplexer.
//!
//! # Architecture
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────┐
//! │  Your Node (Tauri / CLI / mobile / embedded)              │
//! │                                                           │
//! │   let mut reg = ToolRegistry::new();                      │
//! │   reg.register("macos.notes.search", NotesSearch);        │
//! │   reg.derive_capabilities_from_tools();                   │
//! │                                                           │
//! │   NodeClientBuilder::new()                                │
//! │       .gateway_ws_url("ws://gw:3210/v1/nodes/ws")         │
//! │       .node_id("mac1")                                    │
//! │       .token("secret")                                    │
//! │       .build()?                                           │
//! │       .run(reg, shutdown)                                 │
//! │       .await;                                             │
//! └───────────────────────────────────────────────────────────┘
//! ```
//!
//! # Connection flow (hard-coded by the SDK)
//!
//! 1. Connect WS (with `token=<SA_NODE_TOKEN>` query param)
//! 2. Send `node_hello { node: { id, name, node_type, version, tags }, capabilities }`
//! 3. Wait for `gateway_welcome { gateway_version }`
//! 4. Main loop:
//!    - On `tool_request`: dispatch to registered handler, always send `tool_response`
//!    - On `ping`: reply `pong`
//!    - Emit periodic `ping` to keep `last_seen` fresh
//! 5. On disconnect: reconnect with jittered exponential back-off
//!
//! # Naming conventions
//!
//! - Tool names are **lowercase dotted namespaces**: `macos.notes.search`
//! - Capability prefixes are namespace roots: `macos.notes` (prefix match)
//! - Never advertise a capability without at least one registered tool

pub mod builder;
pub mod client;
pub mod reconnect;
pub mod registry;
pub mod types;

// ── Re-exports for ergonomic imports ─────────────────────────────────

pub use builder::NodeClientBuilder;
pub use client::NodeClient;
pub use reconnect::ReconnectBackoff;
pub use registry::{NodeTool, ToolRegistry};
pub use types::{NodeSdkError, ToolContext, ToolError, ToolResult};

// Re-export the entire protocol crate so downstream nodes never need a
// direct sa-protocol dependency.
pub use sa_protocol as protocol;

// Convenience re-exports of the most commonly used protocol types.
pub use sa_protocol::{NodeInfo, ToolResponseError, WsMessage, MAX_TOOL_RESPONSE_BYTES};
