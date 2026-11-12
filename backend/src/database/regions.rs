//! Embedded reference catalog, imported by an explicit administrative initializer.
use anyhow::Context;
use flate2::read::GzDecoder;
use sqlx::{Postgres, Transaction};
use std::io::Read;

const DATA_2023: &[u8] = include_bytes!("../../assets/regions-2023.copy.gz");

/// Keep the catalog in the caller's account/menu initialization transaction.
pub async fn initialize_in_tx(tx: &mut Transaction<'_, Postgres>) -> anyhow::Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(764351204988214)")
        .execute(&mut **tx)
        .await?;
    let imported: Option<bool> =
        sqlx::query_scalar("SELECT imported FROM region_versions WHERE version='2023'")
            .fetch_optional(&mut **tx)
            .await?;
    if imported.is_none() {
        sqlx::query(
            "INSERT INTO region_versions(version,label,imported) VALUES('2023','2023版',false)",
        )
        .execute(&mut **tx)
        .await?;
    }
    if imported == Some(true) {
        return Ok(());
    }
    let mut decoder = Box::new(GzDecoder::new(DATA_2023));
    let mut copy=tx.copy_in_raw("COPY regions (version,source_id,code,parent_code,name,name_pinyin,name_short,level,type,type_name,post_code,source_url) FROM STDIN WITH (FORMAT text)").await?;
    let mut chunk = vec![0u8; 65536];
    loop {
        let count = decoder.read(&mut chunk).context("decompress region seed")?;
        if count == 0 {
            break;
        }
        copy.send(chunk[..count].to_vec()).await?;
    }
    let rows = copy.finish().await?;
    anyhow::ensure!(rows == 665_923, "region seed row count mismatch: {rows}");
    sqlx::query("UPDATE region_versions SET imported=true,updated_at=now() WHERE version='2023'")
        .execute(&mut **tx)
        .await?;
    tracing::info!(rows, "2023 region catalog imported");
    Ok(())
}
