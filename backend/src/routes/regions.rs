use std::collections::HashMap;

use axum::{extract::{Path, Query, State}, routing::get, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::{error::AppError, extractors::{ApiJson, CurrentUser}, state::AppState};
use crate::region_selection::{resolve_local_regions, resolve_regions, township_parent, RegionRecord};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/regions/versions", get(versions).post(create_version))
        .route("/api/regions/versions/{version}", axum::routing::delete(delete_version))
        .route("/api/regions/options", get(options))
        .route("/api/regions/resolve", get(resolve))
        .route("/api/regions", get(list).post(create))
        .route("/api/regions/{id}", axum::routing::put(update).delete(remove))
}

async fn manage(state: &AppState, user: &CurrentUser) -> Result<(), AppError> {
    state.require_platform(user).await?;
    state.require(user, "system:region").await
}

async fn versions(State(state): State<AppState>, _user: CurrentUser) -> Result<Json<Value>, AppError> {
    let rows = sqlx::query("SELECT v.version,v.label,v.source_version,v.imported, count(r.source_id) AS count FROM region_versions v LEFT JOIN regions r ON r.version=v.version WHERE v.deleted_at IS NULL GROUP BY v.version ORDER BY v.version DESC")
        .fetch_all(&state.pool).await?;
    Ok(Json(Value::Array(rows.iter().map(|r| json!({
        "version":r.get::<String,_>("version"), "label":r.get::<String,_>("label"),
        "sourceVersion":r.get::<Option<String>,_>("source_version"),
        "imported":r.get::<bool,_>("imported"), "count":r.get::<i64,_>("count")
    })).collect())))
}

#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct VersionBody { version: String, label: String, source_version: Option<String> }

async fn create_version(State(state): State<AppState>, user: CurrentUser, ApiJson(body): ApiJson<VersionBody>) -> Result<Json<Value>, AppError> {
    manage(&state,&user).await?;
    let version=body.version.trim();
    if version.is_empty() || version.len()>20 || !version.chars().all(|c| c.is_ascii_alphanumeric() || c=='-' || c=='_') {
        return Err(AppError::bad_request("版本标识仅支持字母、数字、横线和下划线，最多20位"));
    }
    let label=body.label.trim();
    if label.is_empty() || label.chars().count()>100 { return Err(AppError::bad_request("版本名称不能为空且最多100字")); }
    let mut tx=state.pool.begin().await?;
    if let Some(ref source)=body.source_version {
        let found: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM region_versions WHERE version=$1 AND deleted_at IS NULL)")
            .bind(source).fetch_one(&mut *tx).await?;
        if !found { return Err(AppError::bad_request("来源版本不存在")); }
    }
    let exists: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM region_versions WHERE version=$1 AND deleted_at IS NULL)")
        .bind(version).fetch_one(&mut *tx).await?;
    if exists { return Err(AppError::bad_request("版本已存在")); }
    let created=sqlx::query("INSERT INTO region_versions(version,label,source_version,imported) VALUES($1,$2,$3,true) ON CONFLICT(version) DO UPDATE SET label=EXCLUDED.label,source_version=EXCLUDED.source_version,imported=true,deleted_at=NULL,updated_at=now() WHERE region_versions.deleted_at IS NOT NULL")
        .bind(version).bind(label).bind(&body.source_version).execute(&mut *tx).await?;
    if created.rows_affected()!=1 { return Err(AppError::bad_request("版本已存在")); }
    if let Some(ref source)=body.source_version {
        sqlx::query("INSERT INTO regions(version,source_id,code,parent_code,name,name_pinyin,name_short,level,type,type_name,post_code,source_url) SELECT $1,source_id,code,parent_code,name,name_pinyin,name_short,level,type,type_name,post_code,source_url FROM regions WHERE version=$2")
            .bind(version).bind(source).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"version":version,"label":label,"sourceVersion":body.source_version})))
}

async fn delete_version(State(state): State<AppState>, user: CurrentUser, Path(version): Path<String>) -> Result<Json<Value>, AppError> {
    manage(&state,&user).await?;
    let mut tx=state.pool.begin().await?;
    let exists:Option<bool>=sqlx::query_scalar("SELECT imported FROM region_versions WHERE version=$1 AND deleted_at IS NULL FOR UPDATE")
        .bind(&version).fetch_optional(&mut *tx).await?;
    if exists.is_none() { return Err(AppError::not_found("地区版本不存在")); }
    let removed=sqlx::query("DELETE FROM regions WHERE version=$1")
        .bind(&version).execute(&mut *tx).await?.rows_affected();
    sqlx::query("UPDATE region_versions SET deleted_at=now(),updated_at=now(),imported=true WHERE version=$1")
        .bind(&version).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"version":version,"deleted":true,"removedRegions":removed})))
}

fn row_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({"id":r.get::<String,_>("source_id"), "version":r.get::<String,_>("version"),
        "code":r.get::<String,_>("code"), "parentCode":r.get::<String,_>("parent_code"),
        "name":r.get::<String,_>("name"), "namePinyin":r.get::<String,_>("name_pinyin"),
        "nameShort":r.get::<String,_>("name_short"), "level":r.get::<i32,_>("level"),
        "type":r.get::<Option<String>,_>("type"), "typeName":r.get::<Option<String>,_>("type_name"),
        "postCode":r.get::<Option<String>,_>("post_code"), "sourceUrl":r.get::<Option<String>,_>("source_url")})
}

fn selection_record(r: sqlx::postgres::PgRow) -> RegionRecord {
    RegionRecord {
        id: r.get("source_id"), version: r.get("version"), code: r.get("code"),
        parent_code: r.get("parent_code"), name: r.get("name"), name_pinyin: r.get("name_pinyin"),
        name_short: r.get("name_short"), level: r.get("level"), kind: r.get("type"),
        type_name: r.get("type_name"), post_code: r.get("post_code"), source_url: r.get("source_url"),
    }
}

async fn selection_version(state: &AppState, requested: Option<&str>) -> Result<String, AppError> {
    let requested = requested.map(str::trim).filter(|version| !version.is_empty());
    let version: Option<String> = sqlx::query_scalar(
        "SELECT v.version FROM region_versions v WHERE v.deleted_at IS NULL \
         AND ($1::text IS NULL OR v.version=$1) \
         AND EXISTS(SELECT 1 FROM regions r WHERE r.version=v.version) \
         ORDER BY v.version DESC LIMIT 1"
    ).bind(requested).fetch_optional(&state.pool).await?;
    version.ok_or_else(|| AppError::not_found("地区版本不存在或暂无地区数据"))
}

async fn options(State(state): State<AppState>, _user: CurrentUser, Query(p): Query<HashMap<String,String>>) -> Result<Json<Value>, AppError> {
    let version = selection_version(&state, p.get("version").map(String::as_str)).await?;
    let parent = p.get("parentCode").map(String::as_str);
    let rows = sqlx::query(
        "SELECT version,source_id,code,parent_code,name,name_pinyin,name_short,level,type,type_name,post_code,source_url \
         FROM regions WHERE version=$1 AND parent_code=COALESCE($2::text,'0') \
         AND level=CASE WHEN $2::text IS NULL THEN 2 ELSE \
             (SELECT CASE WHEN parent.level=3 AND NOT EXISTS \
                 (SELECT 1 FROM regions child WHERE child.version=$1 AND child.parent_code=parent.code AND child.level=4) \
                 THEN 5 ELSE parent.level+1 END FROM regions parent WHERE parent.version=$1 AND parent.code=$2) END \
         AND level BETWEEN 2 AND 6 ORDER BY code,source_id"
    ).bind(&version).bind(parent).fetch_all(&state.pool).await?;
    Ok(Json(json!({"version": version, "records": rows.into_iter().map(selection_record).collect::<Vec<_>>()})))
}

async fn resolve(State(state): State<AppState>, _user: CurrentUser, Query(p): Query<HashMap<String,String>>) -> Result<Json<Value>, AppError> {
    let version = selection_version(&state, p.get("version").map(String::as_str)).await?;
    // Follow parent indexes instead of scanning the 660k street/village rows.
    let rows = sqlx::query(
        "WITH RECURSIVE selection_regions AS ( \
         SELECT * FROM regions WHERE version=$1 AND parent_code='0' AND level=2 \
         UNION ALL SELECT r.* FROM regions r JOIN selection_regions parent \
         ON r.version=parent.version AND r.parent_code=parent.code \
         WHERE r.level>parent.level AND r.level<=4) \
         SELECT version,source_id,code,parent_code,name,name_pinyin,name_short,level,type,type_name,post_code,source_url \
         FROM selection_regions"
    ).bind(&version).fetch_all(&state.pool).await?;
    let mut records: Vec<_> = rows.into_iter().map(selection_record).collect();
    let province = p.get("province").map(String::as_str).unwrap_or("");
    let city = p.get("city").map(String::as_str).unwrap_or("");
    let district = p.get("district").map(String::as_str).unwrap_or("");
    let township = p.get("township").map(String::as_str).unwrap_or("");
    let village = p.get("village").map(String::as_str).unwrap_or("");
    let parent_code = township_parent(&records, &resolve_regions(&records, province, city, district))
        .map(|region| region.code.clone());
    if let Some(parent_code) = parent_code {
        records.extend(local_records(&state, &version, &parent_code, 5).await?);
        let township_code = resolve_local_regions(&records,
            resolve_regions(&records, province, city, district), township, "")
            .township.map(|region| region.code.clone());
        if let Some(township_code) = township_code {
            records.extend(local_records(&state, &version, &township_code, 6).await?);
        }
    }
    let path = resolve_local_regions(&records,
        resolve_regions(&records, province, city, district), township, village);
    Ok(Json(json!({"version": version, "province": path.province, "city": path.city,
        "district": path.district, "township": path.township, "village": path.village})))
}

async fn local_records(state: &AppState, version: &str, parent: &str, level: i32) -> Result<Vec<RegionRecord>, AppError> {
    let rows = sqlx::query(
        "SELECT version,source_id,code,parent_code,name,name_pinyin,name_short,level,type,type_name,post_code,source_url \
         FROM regions WHERE version=$1 AND parent_code=$2 AND level=$3 ORDER BY code,source_id"
    ).bind(version).bind(parent).bind(level).fetch_all(&state.pool).await?;
    Ok(rows.into_iter().map(selection_record).collect())
}

async fn list(State(state): State<AppState>, _user: CurrentUser, Query(p): Query<HashMap<String,String>>) -> Result<Json<Value>, AppError> {
    let version=p.get("version").map(String::as_str).unwrap_or("2023");
    let parent=p.get("parentCode").map(String::as_str).unwrap_or("-1");
    let search=p.get("search").map(|s|s.trim()).filter(|s|!s.is_empty());
    let (page,size,offset)=super::paging(&p);
    let pattern=search.map(|s|format!("%{}%",s));
    let total: i64=sqlx::query_scalar("SELECT count(*) FROM regions WHERE version=$1 AND ($2::text IS NOT NULL OR parent_code=$3) AND ($2::text IS NULL OR code ILIKE $2 OR name ILIKE $2 OR name_pinyin ILIKE $2 OR name_short ILIKE $2)")
        .bind(version).bind(&pattern).bind(parent).fetch_one(&state.pool).await?;
    let rows=sqlx::query("SELECT version,source_id,code,parent_code,name,name_pinyin,name_short,level,type,type_name,post_code,source_url FROM regions WHERE version=$1 AND ($2::text IS NOT NULL OR parent_code=$3) AND ($2::text IS NULL OR code ILIKE $2 OR name ILIKE $2 OR name_pinyin ILIKE $2 OR name_short ILIKE $2) ORDER BY code,source_id LIMIT $4 OFFSET $5")
        .bind(version).bind(&pattern).bind(parent).bind(size).bind(offset).fetch_all(&state.pool).await?;
    Ok(Json(super::page_json(rows.iter().map(row_json).collect(),total,page,size)))
}

#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct RegionBody {
    version: String, code: String, parent_code: String, name: String,
    #[serde(default)] name_pinyin: String, #[serde(default)] name_short: String,
    level: i32, #[serde(rename="type")] kind: Option<String>, type_name: Option<String>,
    post_code: Option<String>, source_url: Option<String>,
}

fn validate(b: &RegionBody) -> Result<(),AppError> {
    for (value,max,label) in [(&b.code,12,"编码"),(&b.parent_code,12,"父级编码"),(&b.name,128,"名称"),(&b.name_pinyin,512,"拼音"),(&b.name_short,64,"拼音简称")] {
        if value.is_empty() || value.chars().count()>max { return Err(AppError::bad_request(format!("{label}不能为空且不能超过{max}字"))); }
    }
    if b.level<1 || b.level>6 || b.code==b.parent_code { return Err(AppError::bad_request("层级或父级编码无效")); }
    for (value,max) in [(&b.kind,12),(&b.type_name,32),(&b.post_code,512),(&b.source_url,512)] {
        if value.as_ref().is_some_and(|v|v.chars().count()>max) { return Err(AppError::bad_request("字段长度超限")); }
    }
    Ok(())
}

async fn check_parent(state:&AppState,b:&RegionBody) -> Result<(),AppError> {
    let version: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM region_versions WHERE version=$1 AND deleted_at IS NULL)")
        .bind(&b.version).fetch_one(&state.pool).await?;
    if !version { return Err(AppError::bad_request("版本不存在")); }
    if b.parent_code!="0" && b.parent_code!="-1" {
        let parent: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM regions WHERE version=$1 AND code=$2 AND level<$3)")
            .bind(&b.version).bind(&b.parent_code).bind(b.level).fetch_one(&state.pool).await?;
        if !parent { return Err(AppError::bad_request("父级编码不存在或层级无效")); }
    }
    Ok(())
}

async fn create(State(state): State<AppState>, user: CurrentUser, ApiJson(b):ApiJson<RegionBody>) -> Result<Json<Value>,AppError> {
    manage(&state,&user).await?; validate(&b)?; check_parent(&state,&b).await?;
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM regions WHERE version=$1 AND code=$2)")
        .bind(&b.version).bind(&b.code).fetch_one(&state.pool).await?;
    if exists { return Err(AppError::bad_request("该版本中地区编码已存在")); }
    let id=Uuid::new_v4().simple().to_string();
    sqlx::query("INSERT INTO regions(version,source_id,code,parent_code,name,name_pinyin,name_short,level,type,type_name,post_code,source_url) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
        .bind(&b.version).bind(&id).bind(&b.code).bind(&b.parent_code).bind(&b.name)
        .bind(&b.name_pinyin).bind(&b.name_short).bind(b.level).bind(&b.kind).bind(&b.type_name)
        .bind(&b.post_code).bind(&b.source_url).execute(&state.pool).await?;
    Ok(Json(json!({"id":id})))
}

async fn update(State(state):State<AppState>,user:CurrentUser,Path(id):Path<String>,ApiJson(b):ApiJson<RegionBody>) -> Result<Json<Value>,AppError> {
    manage(&state,&user).await?; validate(&b)?; check_parent(&state,&b).await?;
    let old:Option<String>=sqlx::query_scalar("SELECT code FROM regions WHERE version=$1 AND source_id=$2")
        .bind(&b.version).bind(&id).fetch_optional(&state.pool).await?;
    let old=old.ok_or_else(||AppError::not_found("地区不存在"))?;
    if old!=b.code {
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM regions WHERE version=$1 AND code=$2)")
            .bind(&b.version).bind(&b.code).fetch_one(&state.pool).await?;
        if exists { return Err(AppError::bad_request("该版本中地区编码已存在")); }
        let children:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM regions WHERE version=$1 AND parent_code=$2)")
            .bind(&b.version).bind(&old).fetch_one(&state.pool).await?;
        if children { return Err(AppError::bad_request("该编码存在下级地区，不能修改")); }
    }
    sqlx::query("UPDATE regions SET code=$3,parent_code=$4,name=$5,name_pinyin=$6,name_short=$7,level=$8,type=$9,type_name=$10,post_code=$11,source_url=$12,updated_at=now() WHERE version=$1 AND source_id=$2")
        .bind(&b.version).bind(&id).bind(&b.code).bind(&b.parent_code).bind(&b.name)
        .bind(&b.name_pinyin).bind(&b.name_short).bind(b.level).bind(&b.kind).bind(&b.type_name)
        .bind(&b.post_code).bind(&b.source_url).execute(&state.pool).await?;
    Ok(Json(json!({"id":id})))
}

async fn remove(State(state):State<AppState>,user:CurrentUser,Path(id):Path<String>,Query(p):Query<HashMap<String,String>>) -> Result<Json<Value>,AppError> {
    manage(&state,&user).await?;
    let version=p.get("version").ok_or_else(||AppError::bad_request("缺少版本"))?;
    let code:Option<String>=sqlx::query_scalar("SELECT code FROM regions WHERE version=$1 AND source_id=$2")
        .bind(version).bind(&id).fetch_optional(&state.pool).await?;
    let code=code.ok_or_else(||AppError::not_found("地区不存在"))?;
    let children:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM regions WHERE version=$1 AND parent_code=$2)")
        .bind(version).bind(&code).fetch_one(&state.pool).await?;
    if children { return Err(AppError::bad_request("请先删除下级地区")); }
    sqlx::query("DELETE FROM regions WHERE version=$1 AND source_id=$2")
        .bind(version).bind(&id).execute(&state.pool).await?;
    Ok(Json(json!({"deleted":true})))
}
