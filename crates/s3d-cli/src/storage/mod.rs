//! ストレージプラグイン — mod.rs

pub mod credentials;
pub mod r2;
pub mod sign;

pub use r2::R2Storage;
