//! Reusable firmware analysis, with no frontend or GUI dependencies.
mod aggregate;
pub mod build;
pub mod compare;
pub mod dependencies;
mod dwarf;
mod elf;
mod model;
pub mod regions;
pub mod stack;

pub use elf::{analyze_bytes, analyze_path, validate_options};
pub use model::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Cannot read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("Invalid ELF: {0}")]
    Invalid(String),
    #[error("Unsupported firmware: {0}")]
    Unsupported(String),
    #[error("Invalid memory configuration: {0}")]
    Configuration(String),
}

/// Shared presentation helper. JSON always contains exact integer bytes.
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MiB", bytes as f64 / 1048576.0)
    }
}
