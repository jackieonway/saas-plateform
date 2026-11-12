use sqlx::{PgPool, Row};

pub async fn initialize(pool: &PgPool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    crate::database::regions::initialize_in_tx(&mut tx).await?;
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
            sqlx::query_scalar("INSERT INTO menus(application_id,parent_id,type,title,path,component,icon,permission,sort_order,visible, id) VALUES($1,$2,'MENU','地区管理','/system/regions','','MapPinned','system:region',3,true, $3) RETURNING id")
                .bind(app_id).bind(parent).bind(crate::util::uuid7::new_uuid()).fetch_one(&mut *tx).await?
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
