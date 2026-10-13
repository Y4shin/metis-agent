//! Serve the skeleton on the resolved local endpoint and call echo through it.
//!
//! This is the runnable half of the walking skeleton:
//!
//! ```text
//! cargo run -p client --example echo
//! ```
//!
//! It starts the minimal daemon on the platform default (or the
//! `METIS_ENDPOINT` override), calls the echo RPC through the wrapper, prints
//! the response, and exits non-zero when anything fails.

/// The message that travels to the skeleton and back.
const MESSAGE: &str = "hello over the local transport";

#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = client::resolve_endpoint();
    let server_endpoint = endpoint.clone();
    tokio::spawn(async move {
        if let Err(err) = client::skeleton::serve_uds(&server_endpoint).await {
            eprintln!("skeleton server error: {err}");
        }
    });
    if !client::skeleton::wait_for_socket(&endpoint).await {
        eprintln!(
            "[result] FAIL: the skeleton did not bind {}",
            endpoint.display()
        );
        std::process::exit(1);
    }

    let caller = client::Client::local().await?;
    let response = caller.echo(MESSAGE).await?;
    println!(
        "[echo] endpoint={} message={response:?}",
        endpoint.display()
    );

    if response != MESSAGE {
        eprintln!("[result] FAIL: unexpected echo {response:?}");
        std::process::exit(1);
    }
    println!("[result] PASS");
    Ok(())
}

/// The local transport is Unix-only in this build; the Windows front-end
/// arrives with its own ticket.
#[cfg(not(unix))]
fn main() {
    eprintln!("[result] FAIL: the local transport is Unix-only in this build");
    std::process::exit(1);
}
