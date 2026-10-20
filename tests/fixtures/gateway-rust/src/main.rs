use gateway_rust_transport_fixture::{router, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    let listener = tokio::net::TcpListener::bind(config.bind_addr()).await?;
    println!(
        "TRANSPORT_FIXTURE_ONLY listening={} instance={}",
        listener.local_addr()?,
        config.instance_id()
    );
    axum::serve(listener, router(config))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
