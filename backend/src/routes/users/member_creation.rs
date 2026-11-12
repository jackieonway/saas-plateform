use super::*;

/// Contacts select an already manageable account, never authorize changing its credentials.
pub(super) async fn try_bind_existing(
    state: &AppState,
    actor: &CurrentUser,
    headers: &HeaderMap,
    body: &UserBody,
) -> Result<Option<Value>, AppError> {
    let Some(input) = &body.membership else {
        return Ok(None);
    };
    crate::membership_lifecycle::validate_member_phone(&body.phone)?;
    let existing: Option<(Uuid,String,String)> = sqlx::query_as(
        "SELECT id,username,status FROM users WHERE \
         regexp_replace(COALESCE(phone,''),'[[:space:]()\\-]','','g')=regexp_replace($1,'[[:space:]()\\-]','','g')",
    ).bind(&body.phone).fetch_optional(&state.pool).await?;
    let email_owner: Option<Uuid> = if body.email.trim().is_empty() {
        None
    } else {
        sqlx::query_scalar("SELECT id FROM users WHERE lower(btrim(email))=lower(btrim($1))")
            .bind(&body.email)
            .fetch_optional(&state.pool)
            .await?
    };
    let Some((id, username, status)) = existing else {
        if email_owner.is_some() {
            return Err(AppError::conflict("邮箱已被其他账号占用，无法与该电话合并"));
        }
        return Ok(None);
    };
    if email_owner.is_some_and(|owner| owner != id) {
        return Err(AppError::conflict("电话和邮箱属于不同账号，不能自动合并"));
    }
    if status != "ENABLED" {
        return Err(AppError::conflict("电话所属账号已停用，不能创建成员"));
    }
    if let Some(scope) = state.scoped_org_ids(actor).await? {
        let accessible: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM memberships WHERE user_id=$1 AND organization_id=ANY($2))",
        )
        .bind(id)
        .bind(scope)
        .fetch_one(&state.pool)
        .await?;
        if !accessible {
            return Err(AppError::conflict("联系方式已被占用，当前无权关联该账号"));
        }
    }
    let membership = crate::membership_lifecycle::bind_account_by_phone(
        state,
        actor,
        headers,
        id,
        &body.phone,
        input,
    )
    .await?;
    Ok(Some(
        json!({"id":id,"username":username,"accountReused":true,"membership":membership}),
    ))
}
