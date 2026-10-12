//! Custom transports for the generated client.
//!
//! The generated `rust` client stores a `reqwest::Client` in a public field on
//! its `Configuration`, so a custom transport is injected by building the client
//! differently, with no wrapper needed.

use std::path::Path;

/// Linux/macOS: connect over a Unix domain socket instead of TCP.
#[cfg(unix)]
pub fn unix_client(socket: &Path) -> reqwest::Client {
    reqwest::Client::builder()
        .unix_socket(socket)
        .build()
        .expect("unix socket client")
}

/// Windows: connect over a named pipe instead of TCP.
///
/// Only compiled on Windows, so it is not verified on this Linux host.
#[cfg(windows)]
pub fn named_pipe_client(pipe: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .windows_named_pipe(pipe.to_owned())
        .build()
        .expect("windows named pipe client")
}
