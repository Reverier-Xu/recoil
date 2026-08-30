//! Typed errors for the headless domain model.

use thiserror::Error;

/// Errors produced by configuration and persistence operations.
#[derive(Debug, Error)]
pub enum Error {
  /// The configuration document is not valid TOML or violates its schema.
  #[error("invalid configuration: {0}")]
  Parse(#[from] toml::de::Error),
  /// A semantic validation rule failed.
  #[error("validation failed: {0}")]
  Validation(String),
  /// A filesystem operation failed.
  #[error("io error: {0}")]
  Io(#[from] std::io::Error),
  /// A value failed to serialize (schema export, validation projection).
  #[error("serialization error: {0}")]
  Serialize(#[from] serde_json::Error),
  /// The configuration document failed to serialize to TOML.
  #[error("toml serialization error: {0}")]
  TomlSerialize(#[from] toml::ser::Error),
}
