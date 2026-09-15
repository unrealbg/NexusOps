//! SSH provider with explicit endpoint trust and bounded, typed read-only execution.

mod bounded_io;
mod error;
mod handler;
mod known_hosts;
mod provider;
mod session;

pub use known_hosts::KnownHosts;
pub use provider::SshProvider;
pub use session::SshSession;
