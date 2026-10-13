//! Generated-code mounting and transport glue for the ConnectRPC spike.
//!
//! The generated trees are checked in under `spikes/connectrpc/generated/`.
//! `buf generate` ran both plugins with `file_per_package`, so each tree is a
//! single `<dotted.package>.rs`; the `pub mod` nesting below is the hand-written
//! module tree that replaces the local-only `protoc-gen-buffa-packaging` plugin.
//!
//! `crate::proto` is the mount point named by the connect plugin's
//! `buffa_module=crate::proto` option, so the service stubs can resolve their
//! message types.

pub mod proto {
    pub mod metis {
        pub mod spike {
            pub mod v1 {
                // Raw buffa output has a hand-written `Default` for enums.
                #![allow(clippy::derivable_impls)]
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../generated/buffa/metis.spike.v1.rs"
                ));
            }
        }
    }
}

pub mod connect {
    pub mod metis {
        pub mod spike {
            pub mod v1 {
                // The fallback dispatcher emits single-arm `match method` blocks.
                #![allow(clippy::match_single_binding)]
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../generated/connect/metis.spike.v1.rs"
                ));
            }
        }
    }
}

#[cfg(unix)]
use std::path::Path;

use connectrpc::{ConnectRpcService, Router};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use hyper_util::service::TowerToHyperService;

/// Serve a Connect router over a Unix domain socket with a hand-written accept
/// loop and hyper's auto (HTTP/1.1 or HTTP/2) connection driver.
///
/// `ConnectRpcService` is the push style the guide documents for a custom
/// accept loop; there is no `Server::serve_connection` on the 0.9.1 `Server`
/// (see findings.md Q1).
#[cfg(unix)]
pub async fn serve_uds(path: &Path, router: Router) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    let listener = tokio::net::UnixListener::bind(path)?;
    let service = ConnectRpcService::new(router);
    let builder = AutoBuilder::new(TokioExecutor::new());
    loop {
        let (stream, _) = listener.accept().await?;
        let conn = builder
            .serve_connection(
                TokioIo::new(stream),
                TowerToHyperService::new(service.clone()),
            )
            .into_owned();
        tokio::spawn(async move {
            if let Err(err) = conn.await {
                eprintln!("connection ended with error: {err}");
            }
        });
    }
}

/// Build an h2c client connection over the same Unix socket, lazily, through
/// the generic `lazy_with_connector` escape hatch.
#[cfg(unix)]
pub fn uds_transport(path: impl Into<std::path::PathBuf>) -> connectrpc::client::Http2Connection {
    let path = path.into();
    let connector = tower::service_fn(move |_uri: http::Uri| {
        let path = path.clone();
        async move {
            let stream = tokio::net::UnixStream::connect(path).await?;
            Ok::<_, std::io::Error>(TokioIo::new(stream))
        }
    });
    connectrpc::client::Http2Connection::lazy_with_connector(connector, local_authority())
}

/// The `:authority` value used for the local IPC transport. The server does not
/// validate it.
pub fn local_authority() -> http::Uri {
    "http://localhost".parse().expect("static authority")
}

/// Serve a Connect router over a Windows named pipe, compile-only on this host.
#[cfg(windows)]
pub async fn serve_named_pipe(pipe_name: &str, router: Router) -> std::io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let service = ConnectRpcService::new(router);
    let builder = AutoBuilder::new(TokioExecutor::new());
    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(false)
            .create(pipe_name)?;
        server.connect().await?;
        let io = TokioIo::new(server);
        let conn = builder
            .serve_connection(io, TowerToHyperService::new(service.clone()))
            .into_owned();
        tokio::spawn(async move {
            let _ = conn.await;
        });
    }
}

/// Build an h2c client connection over a Windows named pipe, compile-only here.
#[cfg(windows)]
pub fn named_pipe_transport(pipe_name: &str) -> connectrpc::client::Http2Connection {
    use tokio::net::windows::named_pipe::ClientOptions;

    let name = pipe_name.to_owned();
    let connector = tower::service_fn(move |_uri: http::Uri| {
        let name = name.clone();
        async move {
            let stream = ClientOptions::new().open(&name)?;
            Ok::<_, std::io::Error>(TokioIo::new(stream))
        }
    });
    connectrpc::client::Http2Connection::lazy_with_connector(connector, local_authority())
}
