//! For raw SQL in tests outside this crate (`rusqlite::params!` needs the crate name in
//! scope; `Conn`'s delegators are `pub` under this same feature).

pub use rusqlite;
