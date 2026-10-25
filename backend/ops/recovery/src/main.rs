fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("usage: saas-recovery backup|backup-new|verify|age|restore|uuid-inventory|uuid-export|uuid-transform|uuid-verify-package|uuid-import|uuid-config-rekey|uuid-seal-evidence|uuid-open-evidence /absolute/private/config.json");
        std::process::exit(2);
    }
    match saas_recovery::execute(&args[0], std::path::Path::new(&args[1])) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("recovery failed: {error}");
            std::process::exit(1);
        }
    }
}
