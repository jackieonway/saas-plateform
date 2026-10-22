use std::io::Read;

use anyhow::Context;
use flate2::read::GzDecoder;
use sqlx::{PgPool, Row};

const DATA_2023: &[u8] = include_bytes!("../assets/regions-2023.copy.gz");

pub async fn initialize(pool: &PgPool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(764351204988214)")
        .execute(&mut *tx)
        .await?;
    let imported: Option<bool> =
        sqlx::query_scalar("SELECT imported FROM region_versions WHERE version='2023'")
            .fetch_optional(&mut *tx)
            .await?;
    if imported.is_none() {
        sqlx::query(
            "INSERT INTO region_versions(version,label,imported) VALUES('2023','2023版',false)",
        )
        .execute(&mut *tx)
        .await?;
    }
    if imported != Some(true) {
        let mut decoder = GzDecoder::new(DATA_2023);
        let mut copy = tx.copy_in_raw("COPY regions (version,source_id,code,parent_code,name,name_pinyin,name_short,level,type,type_name,post_code,source_url) FROM STDIN WITH (FORMAT text)").await?;
        let mut chunk = [0u8; 65536];
        loop {
            let count = decoder.read(&mut chunk).context("decompress region seed")?;
            if count == 0 {
                break;
            }
            copy.send(chunk[..count].to_vec()).await?;
        }
        let rows = copy.finish().await?;
        anyhow::ensure!(rows == 665_923, "region seed row count mismatch: {rows}");
        sqlx::query(
            "UPDATE region_versions SET imported=true,updated_at=now() WHERE version='2023'",
        )
        .execute(&mut *tx)
        .await?;
        tracing::info!(rows, "2023 region catalog imported");
    }

    // This runs after ordinary bootstrap, including on existing installations.
    let app = sqlx::query("SELECT id FROM applications WHERE code='system'")
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(app) = app {
        let app_id: uuid::Uuid = app.try_get("id")?;
        let parent: Option<uuid::Uuid> = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
            "SELECT parent_id FROM menus WHERE application_id=$1 AND path='/system/menus' LIMIT 1",
        )
        .bind(app_id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        let existing: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT id FROM menus WHERE application_id=$1 AND path='/system/regions' LIMIT 1",
        )
        .bind(app_id)
        .fetch_optional(&mut *tx)
        .await?;
        let menu_id = if let Some(id) = existing {
            id
        } else {
            sqlx::query_scalar("INSERT INTO menus(application_id,parent_id,type,title,path,component,icon,permission,sort_order,visible) VALUES($1,$2,'MENU','地区管理','/system/regions','','MapPinned','system:region',3,true) RETURNING id")
                .bind(app_id).bind(parent).fetch_one(&mut *tx).await?
        };
        // Existing platform grants and built-in administrator roles receive the new menu.
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) SELECT id,$1 FROM tenants WHERE is_platform=true ON CONFLICT DO NOTHING")
            .bind(menu_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) SELECT o.id,$1 FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform=true ON CONFLICT DO NOTHING")
            .bind(menu_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) SELECT r.id,$1 FROM roles r JOIN tenants t ON t.id=r.tenant_id WHERE t.is_platform=true AND r.built_in=true ON CONFLICT DO NOTHING")
            .bind(menu_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
