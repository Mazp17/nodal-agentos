//! MCP server over stdio for agents: `claude mcp add nodal -- nodal-mcp`. Forwards to the
//! running app (see `nodal_mcp_proto::stdio`).

fn main() -> std::process::ExitCode {
    nodal_mcp_proto::stdio::main(env!("CARGO_PKG_VERSION"))
}
