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

            sqlx::raw_sql("CREATE FUNCTION reject_bootstrap_regions() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture region import rejection'; END $$; CREATE TRIGGER reject_bootstrap_regions BEFORE INSERT ON regions FOR EACH ROW EXECUTE FUNCTION reject_bootstrap_regions();")
                .execute(&pool).await?;
            ensure!(initialize_platform(&pool,"admin","Owned-Initial-Password9!").await.is_err(),"地区导入失败未使初始化失败");
            let partial:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM users)+(SELECT count(*) FROM tenants)+(SELECT count(*) FROM organizations)+(SELECT count(*) FROM region_versions)+(SELECT count(*) FROM regions)")
                .fetch_one(&pool).await?;
            ensure!(partial==0,"地区导入失败留下部分初始化数据");
            sqlx::query("DROP TRIGGER reject_bootstrap_regions ON regions").execute(&pool).await?;

            let (first,parallel) = tokio::join!(
                initialize_platform(&pool,"admin","Owned-Initial-Password9!"),
                initialize_platform(&pool,"admin","Owned-Initial-Password9!")
            );
            let first=first?;let parallel=parallel?;
            let catalog:(bool,i64,i64)=sqlx::query_as("SELECT v.imported,count(r.source_id),count(DISTINCT r.level) FROM region_versions v JOIN regions r ON r.version=v.version WHERE v.version='2023' GROUP BY v.imported")
                .fetch_one(&pool).await?;
            ensure!(catalog==(true,665_923,6),"默认行政区划版本、记录数或层级不完整: {catalog:?}");
            sqlx::query("UPDATE regions SET name='自定义地区验证' WHERE version='2023' AND source_id=(SELECT min(source_id) FROM regions WHERE version='2023')").execute(&pool).await?;
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
            for (path,permission,parent) in [
                ("/system/regions","system:region","平台配置"),
                ("/system/sessions","system:session","开放平台"),
            ] {
                let complete:bool=sqlx::query_scalar("SELECT count(*)=1 FROM menus m JOIN menus p ON p.id=m.parent_id WHERE m.path=$1 AND m.permission=$2 AND p.title=$3 AND m.type='MENU' AND m.visible AND m.status='ENABLED' AND EXISTS(SELECT 1 FROM tenant_menus WHERE tenant_id=$4 AND menu_id=m.id) AND EXISTS(SELECT 1 FROM organization_menus WHERE organization_id=$5 AND menu_id=m.id) AND EXISTS(SELECT 1 FROM role_menus rm JOIN roles r ON r.id=rm.role_id WHERE r.code='super_admin' AND r.organization_id=$5 AND rm.menu_id=m.id)")
                    .bind(path).bind(permission).bind(parent).bind(first.tenant_id).bind(first.organization_id).fetch_one(&pool).await?;
                ensure!(complete,"内置入口及租户、组织、角色授权不完整: {path}");
            }
            let revoke_granted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM menus m JOIN role_menus rm ON rm.menu_id=m.id JOIN roles r ON r.id=rm.role_id JOIN tenant_menus tm ON tm.menu_id=m.id AND tm.tenant_id=r.tenant_id JOIN organization_menus om ON om.menu_id=m.id AND om.organization_id=r.organization_id WHERE m.permission='system:session:revoke' AND r.code='super_admin' AND r.organization_id=$1)")
                .bind(first.organization_id).fetch_one(&pool).await?;
            ensure!(revoke_granted,"会话吊销按钮未完整授权");
            let auth_private_rows:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM auth_clients)+(SELECT count(*) FROM platform_sessions)+(SELECT count(*) FROM bff_sessions)")
                .fetch_one(&pool).await?;
            ensure!(auth_private_rows==0,"初始化意外创建客户端或登录会话");
            let again=initialize_platform(&pool,"admin","Different-Initial-Password9!").await?;
            ensure!(!again.created && again.admin_id==first.admin_id,"重复初始化未保留账号");
            let retained:String=sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1").bind(first.admin_id).fetch_one(&pool).await?;
            ensure!(hash==retained,"重复执行重置了密码");
            let retained_catalog:(i64,i64)=sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE name='自定义地区验证') FROM regions WHERE version='2023'").fetch_one(&pool).await?;
            ensure!(retained_catalog==(665_923,1),"重复初始化重导、重复或覆盖了已导入地区数据");
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
