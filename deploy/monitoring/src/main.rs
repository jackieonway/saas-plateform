use std::io::Read;

#[tokio::main]
async fn main() {
    let result = async {
        let file = std::env::args_os().nth(1).ok_or("CONFIG_REQUIRED")?;
        let mut bytes = Vec::new();
        std::fs::File::open(file)
            .map_err(|_| "CONFIG_UNREADABLE")?
            .take(16385)
            .read_to_end(&mut bytes)
            .map_err(|_| "CONFIG_UNREADABLE")?;
        saas_monitor::run(saas_monitor::Config::parse(&bytes)?).await
    }
    .await;
    if let Err(code) = result {
        eprintln!("{code}");
        std::process::exit(1)
    }
}
