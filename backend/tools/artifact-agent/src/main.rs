#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        eprintln!("Usage: saas-artifact-agent /absolute/private/agent.json");
        std::process::exit(2);
    }
    if let Err(error) = saas_artifact_agent::run(std::path::Path::new(&args[0])).await {
        eprintln!("ARTIFACT_AGENT {}", error.code());
        std::process::exit(1);
    }
}
