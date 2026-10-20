use oauth_playground::{app, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    let listener = tokio::net::TcpListener::bind(config.bind_addr()).await?;
    let router = app(config)?;
    // Deliberately no HTTP trace logger: callback query strings, credentials,
    // request/response bodies and upstream errors must never enter logs.
    eprintln!("OAuth demonstration API ready");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
