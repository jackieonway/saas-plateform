use std::collections::BTreeMap;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The environment callback is only evaluated for remote commands. Never enumerate
    // the complete environment: it can contain unrelated secrets.
    let local = saas_platform_admin::run(args.clone(), &BTreeMap::new());
    let result = if local
        .as_ref()
        .is_err_and(|error| error.category() == "INVALID_ORIGIN")
    {
        let env = [
            "PLATFORM_API_ORIGIN",
            "PLATFORM_ADMIN_BEARER",
            "PLATFORM_CA_FILE",
        ]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok().map(|value| (key.to_owned(), value)))
        .collect();
        saas_platform_admin::run(args, &env)
    } else {
        local
    };
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{}", error.to_json());
            std::process::exit(1);
        }
    }
}
