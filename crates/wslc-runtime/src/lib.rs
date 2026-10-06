//! Scoped process execution and Windows container sessions.

#[cfg(windows)]
pub mod container;
pub mod process;
#[cfg(windows)]
pub mod sdk;
#[cfg(windows)]
pub mod session;

pub type Result<T> = std::result::Result<T, String>;
