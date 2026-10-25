use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match saas_platform_backend::database::run_membership_rollout_command(
        std::env::args_os().skip(1),
    )
    .await
    {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
