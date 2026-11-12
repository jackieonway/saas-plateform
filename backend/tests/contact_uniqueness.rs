use anyhow::{ensure, Result};
use saas_platform_backend::database::run_migrations;
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    Connection, PgConnection,
};
use std::str::FromStr;
use uuid::Uuid;

#[tokio::test]
#[ignore = "使用 TEST_DATABASE_URL 创建并删除专属 PostgreSQL 测试库"]
async fn contacts_are_globally_unique_after_normalization() -> Result<()> {
    let options = PgConnectOptions::from_str(&std::env::var("TEST_DATABASE_URL")?)?;
    let mut admin = PgConnection::connect_with(&options.clone().database("postgres")).await?;
    let name = format!("contact_unique_test_{}", Uuid::now_v7().simple());
    sqlx::query(&format!("CREATE DATABASE \"{name}\""))
        .execute(&mut admin)
        .await?;
    let result: Result<()> = async {
        let pool = PgPoolOptions::new().max_connections(4).connect_with(options.database(&name)).await?;
        let result: Result<()> = async {
            run_migrations(&pool).await?;
            sqlx::query("INSERT INTO users(id,username,password_hash,email,phone) VALUES($1,'first','test-only','Owner@Example.test','138 0000-0001')")
                .bind(Uuid::now_v7()).execute(&pool).await?;
            for (email,phone,index) in [
                ("other@example.test","(138)00000001","idx_users_login_phone"),
                (" owner@example.TEST ","13800000002","idx_users_login_email"),
            ] {
                let error=sqlx::query("INSERT INTO users(id,username,password_hash,email,phone) VALUES($1,$2,'test-only',$3,$4)")
                    .bind(Uuid::now_v7()).bind(index).bind(email).bind(phone).execute(&pool).await.err().ok_or_else(||anyhow::anyhow!("重复联系方式未被拒绝"))?;
                ensure!(error.as_database_error().and_then(|e|e.constraint())==Some(index),"唯一约束不匹配");
            }
            sqlx::query("UPDATE users SET status='DISABLED' WHERE username='first'").execute(&pool).await?;
            ensure!(sqlx::query("INSERT INTO users(id,username,password_hash,phone) VALUES($1,'disabled-conflict','test-only','13800000001')")
                .bind(Uuid::now_v7()).execute(&pool).await.is_err(),"停用账号释放了占用电话");
            for username in ["blank-a","blank-b"] {
                sqlx::query("INSERT INTO users(id,username,password_hash,email,phone) VALUES($1,$2,'test-only','','')")
                    .bind(Uuid::now_v7()).bind(username).execute(&pool).await?;
            }
            let insert=|username:&'static str,phone:&'static str| {
                let pool=&pool;
                async move { sqlx::query("INSERT INTO users(id,username,password_hash,phone) VALUES($1,$2,'test-only',$3)")
                    .bind(Uuid::now_v7()).bind(username).bind(phone).execute(pool).await }
            };
            let (a,b)=tokio::join!(insert("race-a","13800000003"),insert("race-b","138 0000-0003"));
            ensure!(a.is_ok()!=b.is_ok(),"并发写入没有唯一胜者");
            let error=sqlx::query("UPDATE users SET email='OWNER@example.test' WHERE username='blank-a'").execute(&pool).await;
            ensure!(error.is_err(),"修改资料绕过邮箱唯一性");
            Ok(())
        }.await;
        pool.close().await;
        result
    }.await;
    let cleanup = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
        .execute(&mut admin)
        .await;
    cleanup?;
    println!("联系方式唯一性测试库已清理");
    result
}
