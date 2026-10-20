use std::{
    io::{self, Read},
    net::Ipv4Addr,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = std::env::var("PHASE22_PORT")
        .unwrap_or_else(|_| "0".into())
        .parse::<u16>()?;
    let origin = std::env::var("PHASE22_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:3000".into());
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let (app, socket_io) = phase22_socketio_fixture::router(origin)?;
    let (eof_tx, eof_rx) = tokio::sync::oneshot::channel();
    // Tokio stdin's uncancellable blocking read otherwise delays Ctrl+C shutdown.
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut byte = [0];
        while input.read(&mut byte).unwrap_or(0) > 0 {}
        let _ = eof_tx.send(());
    });
    println!(
        "{}",
        serde_json::json!({"event":"ready", "port": listener.local_addr()?.port()})
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = eof_rx => {},
            }
            socket_io.close().await;
        })
        .await
        .map_err(io::Error::other)?;
    Ok(())
}
