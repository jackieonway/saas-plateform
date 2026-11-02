fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!(
            "usage: saas-recovery backup|backup-new|verify|age /absolute/private/config.json"
        );
        std::process::exit(2);
    }
    match saas_recovery::execute(&args[0], std::path::Path::new(&args[1])) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("recovery failed: {error}");
            if error.to_string() == "CANONICAL_IDENTITY_RECOVERY_REQUIRED" {
                eprintln!("Canonical identity recovery: backend/ops/identity-recovery/README.md");
            }
            std::process::exit(1);
        }
    }
}
