//! The Metis daemon.
//!
//! `state` is the daemon's persistent store: one SQLite database that carries
//! the table the token hashes live in, and that the effort's later session and
//! message work extends with new migrations. `token` mints the per-surface
//! credential and hashes it. The binary beside this library stays a thin entry
//! point.

pub mod state;
pub mod token;
