//! Verify current restored schema fingerprints without historical DDL repair.
use crate::{
    config::Config,
    native::{self, Scratch},
    Error, Result,
};
pub(crate) fn capture(c: &Config, s: &Scratch, service: &str, db: Option<&str>) -> Result<String> {
    native::sql(c, s, service, db, include_str!("../catalog.sql"))
}
pub(crate) fn canonicalize(c: &Config, s: &Scratch, kind: &str, target: &str) -> Result<()> {
    let sql = match kind {
        "stores" => include_str!("../canonical-stores.sql"),
        "work_orders" => include_str!("../canonical-work-orders.sql"),
        _ => return Err(Error("unsupported schema reconstruction kind")),
    };
    native::sql(c, s, &c.admin_service, Some(target), sql)?;
    if kind == "stores" {
        let healthy=native::sql(c,s,&c.admin_service,Some(target),"SELECT count(*)=1 AND COALESCE(bool_and(fingerprint=public.stores_schema_fingerprint()),false) FROM public._stores_schema_contract WHERE singleton")?;
        if healthy.trim() != "t" {
            return Err(Error("restored stored schema contract differs"));
        }
    }
    Ok(())
}
