//! Thin process entrypoint for the isolated chat reference.

use saas_chat_reference::{ServerConfig, serve};
use std::io::{self, Read};
use tokio::{net::TcpListener, sync::oneshot};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ServerConfig::from_env()?;
    let listener = TcpListener::bind(config.bind_addr()).await?;
    let address = listener.local_addr()?;
    let (eof_tx, eof_rx) = oneshot::channel();
    // A detached OS thread avoids Tokio's uncancellable stdin task delaying shutdown.
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut buffer = [0_u8; 256];
        while let Ok(read) = input.read(&mut buffer) {
            if read == 0 {
                break;
            }
        }
        let _ = eof_tx.send(());
    });
    println!(
        "{}",
        serde_json::json!({
            "event": "ready", "service": "chat-reference", "address": address.to_string(),
            "port": address.port(), "path": "/"
        })
    );
    serve(listener, config, async {
        tokio::select! {
            _ = eof_rx => {},
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result { eprintln!("shutdown signal error: {error}"); }
            }
        }
    })
    .await?;
    Ok(())
}
