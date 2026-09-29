//! Moved to `nodal_app::chats` (the `Chats` facade, `ops`, `context`, `hooks` and `transcript`)
//! and `nodal_host::claude::chats` (process supervision), reached from `SRC/commands/chats.rs`.
//! Nothing in this crate calls this module anymore (`ChatState` isn't managed or read by any
//! other file either — see the wave 3b report): left empty because `SRC/lib.rs` still declares
//! `mod chats;`.
