use anyhow::{ensure, Context, Result};
use argon2::{
    password_hash::{PasswordHash, PasswordVerifier},
    Argon2,
};
use saas_platform_backend::database::{bootstrap::initialize_platform, run_migrations};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    Connection, PgConnection,
};
use std::str::FromStr;
use uuid::Uuid;

#[tokio::test]
#[ignore = "使用 TEST_DATABASE_URL 创建并删除专属 PostgreSQL 测试库"]
async fn platform_bootstrap_is_atomic_complete_and_idempotent() -> Result<()> {
    let options = PgConnectOptions::from_str(&std::env::var("TEST_DATABASE_URL")?)?;
    let mut admin = PgConnection::connect_with(&options.clone().database("postgres")).await?;
    let name = format!("platform_bootstrap_test_{}", Uuid::now_v7().simple());
    sqlx::query(&format!("CREATE DATABASE \"{name}\""))
        .execute(&mut admin)
        .await?;
    let result: Result<()> = async {
        let pool = PgPoolOptions::new().max_connections(4).connect_with(options.database(&name)).await?;
        let result: Result<()> = async {
            run_migrations(&pool).await?;
            ensure!(initialize_platform(&pool,"admin","admin123").await.is_err(),"弱密码未被拒绝");
            sqlx::raw_sql("CREATE FUNCTION reject_bootstrap_member() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture rejection'; END $$; \
                CREATE TRIGGER reject_bootstrap_member BEFORE INSERT ON memberships FOR EACH ROW EXECUTE FUNCTION reject_bootstrap_member();")
                .execute(&pool).await?;
            ensure!(initialize_platform(&pool,"admin","Owned-Initial-Password9!").await.is_err(),"部分初始化未失败");
            let count: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM tenants)+(SELECT count(*) FROM organizations)+(SELECT count(*) FROM users)")
                .fetch_one(&pool).await?;
            ensure!(count==0,"初始化失败留下部分数据");
            sqlx::query("DROP TRIGGER reject_bootstrap_member ON memberships").execute(&pool).await?;

            let (first,parallel) = tokio::join!(
                initialize_platform(&pool,"admin","Owned-Initial-Password9!"),
                initialize_platform(&pool,"admin","Owned-Initial-Password9!")
            );
            let first=first?;let parallel=parallel?;
            ensure!(first.created != parallel.created,"并发初始化未序列化");
            ensure!(first.admin_id==parallel.admin_id && first.tenant_id==parallel.tenant_id && first.organization_id==parallel.organization_id,"并发初始化产生不同实体");
            let row:(i64,i64,i64,i64)=sqlx::query_as("SELECT \
                (SELECT count(*) FROM tenants WHERE is_platform), \
                (SELECT count(*) FROM organizations WHERE parent_id IS NULL), \
                (SELECT count(*) FROM users WHERE is_super_admin AND status='ENABLED'), \
                (SELECT count(*) FROM memberships WHERE member_type='EMPLOYEE' AND status='ENABLED')")
                .fetch_one(&pool).await?;
            ensure!(row==(1,1,1,1),"平台租户、根组织、超管或成员关系不完整");
            let hash:String=sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1").bind(first.admin_id).fetch_one(&pool).await?;
            ensure!(Argon2::default().verify_password(b"Owned-Initial-Password9!",&PasswordHash::new(&hash).map_err(|_|anyhow::anyhow!("密码摘要无效"))?).is_ok(),"超管密码无法验证");
            let invalid_ids:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT id FROM tenants UNION ALL SELECT id FROM organizations UNION ALL SELECT id FROM users UNION ALL SELECT id FROM memberships UNION ALL SELECT id FROM roles UNION ALL SELECT id FROM applications UNION ALL SELECT id FROM menus) records WHERE NOT is_uuid_v7(id)")
                .fetch_one(&pool).await?;
            ensure!(invalid_ids==0,"初始化未使用 UUIDv7");
            let grants:(i64,i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM menus), (SELECT count(*) FROM role_menus), (SELECT count(*) FROM tenant_menus), (SELECT count(*) FROM organization_menus), (SELECT count(*) FROM user_roles)")
                .fetch_one(&pool).await?;
            ensure!(grants.0>=32 && grants.0==grants.1 && grants.0==grants.2 && grants.0==grants.3 && grants.4==1,"菜单和角色授权不完整");
            let auth_private_rows:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM auth_clients)+(SELECT count(*) FROM platform_sessions)+(SELECT count(*) FROM bff_sessions)")
                .fetch_one(&pool).await?;
            ensure!(auth_private_rows==0,"初始化意外创建客户端或登录会话");
            let again=initialize_platform(&pool,"admin","Different-Initial-Password9!").await?;
            ensure!(!again.created && again.admin_id==first.admin_id,"重复初始化未保留账号");
            let retained:String=sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1").bind(first.admin_id).fetch_one(&pool).await?;
            ensure!(hash==retained,"重复执行重置了密码");
            ensure!(initialize_platform(&pool,"another-admin","Owned-Initial-Password9!").await.is_err(),"已有数据被接管");
            Ok(())
        }.await;
        pool.close().await;
        result
    }.await;
    let cleanup = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
        .execute(&mut admin)
        .await
        .context("测试库清理失败");
    cleanup?;
    println!("专属初始化测试库已清理");
    result
}
