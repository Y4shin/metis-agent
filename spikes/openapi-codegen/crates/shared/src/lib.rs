//! Glue shared by every alternative in the spike: the Unix-socket serving loop,
//! the custom reqwest transports, and the handwritten SSE client. None of this
//! depends on which HTTP layers were generated.

pub mod sse_client;
pub mod transport;

use std::path::Path;

use axum::Router;
use hyper_util::rt::TokioIo;
use tokio::net::UnixListener;
use tower::ServiceExt;

pub async fn serve_uds(path: &Path, app: Router) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let app = app.clone();
        tokio::spawn(async move {
            let service =
                hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                    let app = app.clone();
                    async move { app.oneshot(req.map(axum::body::Body::new)).await }
                });
            if let Err(err) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await
            {
                eprintln!("connection error: {err}");
            }
        });
    }
}
