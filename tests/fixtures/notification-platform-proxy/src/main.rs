#[tokio::main]
async fn main() -> Result<(), phase23_notification_platform_proxy::Error> {
    let config = phase23_notification_platform_proxy::Config::from_env()?;
    phase23_notification_platform_proxy::run(config).await
}
