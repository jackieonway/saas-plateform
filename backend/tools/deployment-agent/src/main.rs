fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 || !["--once", "--watch"].contains(&args[1].as_str()) {
        eprintln!("Usage: saas-deployment-agent /absolute/config.json --once|--watch");
        std::process::exit(2);
    }
    if let Err(code) =
        saas_deployment_agent::run(std::path::Path::new(&args[0]), args[1] == "--watch")
    {
        eprintln!("DEPLOYMENT_AGENT {code}");
        std::process::exit(1);
    }
}
