//! CRUD for projects, repos, tasks, relations and hidden executors. Synchronous functions
//! over `&Conn` (or `&Tx`, which derefs to `Conn`): callers run them with `with_db`.
//! Business validations live in `nodal-app`; here there's only SQL and integrity errors.

pub mod hidden_executors;
pub mod projects;
pub mod relations;
pub mod repos;
pub mod tasks;
