use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match saas_platform_backend::database::run_migration_command(std::env::args_os().skip(1)).await
    {
        Ok(()) => {
            println!("PLATFORM_MIGRATIONS_COMPLETED");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
