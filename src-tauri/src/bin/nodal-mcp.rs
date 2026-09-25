//! MCP server over stdio for agents: `claude mcp add nodal -- nodal-mcp`. Forwards to the
//! running app (see `nodal_lib::mcp`).
//! Build: `cargo build --release --features mcp-cli --bin nodal-mcp`.

fn main() -> std::process::ExitCode {
    nodal_lib::mcp::stdio::main()
}
