//! Small serde helpers shared by the DTOs.

use serde::{Deserialize, Deserializer};

/// For patches: tells "missing field" (`None`) apart from `null` (`Some(None)`).
/// Usage: `#[serde(default, deserialize_with = "double_option")] x: Option<Option<T>>`.
pub fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}
