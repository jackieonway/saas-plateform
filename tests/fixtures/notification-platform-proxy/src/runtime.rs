use std::io::{self, Read};

use crate::{proxy::Proxy, Config, Error};

/// Bind loopback, emit one non-secret JSON readiness line, and serve until stdin
/// EOF, Ctrl+C, or Unix SIGTERM. Shutdown cancels gate waits and upstream work
/// before allowing Axum's graceful connection drain to finish.
pub async fn run(config: Config) -> Result<(), Error> {
    let proxy = Proxy::new(&config)?;
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|error| Error::with_source("cannot bind fixture listener", error))?;
    let address = listener
        .local_addr()
        .map_err(|error| Error::with_source("cannot inspect fixture listener", error))?;
    let (eof_tx, eof_rx) = tokio::sync::oneshot::channel();
    // An uncancellable Tokio stdin blocking task would delay process shutdown.
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut byte = [0];
        while input.read(&mut byte).unwrap_or(0) > 0 {}
        let _ = eof_tx.send(());
    });
    println!(
        "{}",
        serde_json::json!({"event":"ready", "port":address.port()})
    );
    let app = proxy.router();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = termination() => {},
                _ = eof_rx => {},
            }
            proxy.stopping.send_replace(true);
        })
        .await
        .map_err(|error| Error::with_source("fixture server failed", error))
}

#[cfg(unix)]
async fn termination() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut signal) => {
            signal.recv().await;
        }
        Err(_) => std::future::pending::<()>().await,
    }
}

#[cfg(not(unix))]
async fn termination() {
    std::future::pending::<()>().await;
}
