//! Wire protocol and stdio transport shared by the `nodal-mcp` binary and the shell's
//! MCP command dispatch. No tauri dependency, so `nodal-mcp` never links it.
#![forbid(unsafe_code)]

pub mod paths;
pub mod protocol;
pub mod stdio;
pub mod tools;
