use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::args_os().len() != 1 {
        eprintln!("用法：platform-bootstrap；通过 DATABASE_URL、BOOTSTRAP_ADMIN_PASSWORD 配置，BOOTSTRAP_ADMIN_USERNAME 默认 admin");
        return ExitCode::FAILURE;
    }
    match saas_platform_backend::database::bootstrap::run_bootstrap_command().await {
        Ok(result) => {
            println!(
                "{}：平台租户 {}，根组织 {}，超管 {}",
                if result.created {
                    "初始化完成"
                } else {
                    "已初始化，保留原有账号与密码"
                },
                result.tenant_id,
                result.organization_id,
                result.admin_id
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
