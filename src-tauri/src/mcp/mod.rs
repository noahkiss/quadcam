//! MCP server on stdio (`quadcam-cli mcp`). A thin layer over the core: when the QuadCam app
//! is running it drives the app's session through the control socket, so the person sees
//! every change live; otherwise it runs a headless core on the shared session file.
//! Only MCP messages go to stdout; logs go to stderr.

mod render;
mod server;
mod tools;

pub use render::clip_views;
pub use server::{serve_stdio, AutoBackend, Backend, LocalBackend, Server};
pub use tools::tools;
