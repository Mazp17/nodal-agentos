//! Linear GraphQL client and `TaskProvider` implementation. No tauri dependency.
#![forbid(unsafe_code)]

mod client;
mod detail;
mod error;
mod model;
pub mod provider;

pub use client::{http_client, LinearClient};
/// The shared HTTP client type, so the shell can hold one without depending on `reqwest`.
pub use reqwest::Client as HttpClient;
pub use detail::IssueDetail;
pub use error::LinearError;
pub use provider::{LinearFactory, LinearProvider};

#[cfg(test)]
mod live_tests;
