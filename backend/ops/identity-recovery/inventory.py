"""Strict public security inventory for the canonical version-1 recovery job.

The registry below is an explicit copy of the current 0001 column/PK contract,
not a schema discovery fallback. A database query returns raw values in memory;
``project_database`` immediately hashes secret-bearing fields before an inventory
can enter a witness. Errors contain fixed categories, never database row data.
The caller owns the database binding, privileges and write barrier. Each database
capture is one SELECT snapshot; two captures are not a cross-database snapshot.
"""
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import json
import re
import uuid


FORMAT = "canonical-recovery-inventory/1"
MAX_ROWS = 20000
MAX_BYTES = 16 * 1024 * 1024
MAX_TEXT = 16384
MAX_INTEGER = (1 << 63) - 2
HEX = re.compile(r"[0-9a-f]{64}\Z")
IDENTIFIER = re.compile(r"[a-z_][a-z0-9_]{0,62}\Z")


class InventoryError(RuntimeError):
    """A fixed public rejection category; contains no captured data."""


def require(condition, category):
    if not condition:
        raise InventoryError("INVENTORY_" + category)


def canonical(value):
    try:
        return json.dumps(value, ensure_ascii=False, sort_keys=True,
                          separators=(",", ":"), allow_nan=False).encode("utf-8")
    except (ValueError, TypeError, UnicodeError, RecursionError):
        raise InventoryError("INVENTORY_ENCODING_INVALID") from None


def digest(value):
    return hashlib.sha256(canonical(value)).hexdigest()


@dataclass(frozen=True)
class Field:
    source: str
    pg_type: str
    nullable: bool
    redacted: bool = False


@dataclass(frozen=True)
class TableSpec:
    pk: tuple
    fields: dict
    immutable: tuple
    append_only: bool = False


# PHC, recoverable ciphertext, secret-derived command material and arbitrary JSON
# do not enter an external witness. Existing one-way token/source/replay digests
# are public comparison facts and retain their actual column name.
SECRET_NAMES = frozenset((
    "password_hash", "secret_hash", "service_secret_hash", "secret_digest",
    "client_secret_hash", "code_challenge", "challenge", "verifier",
    "state_value", "nonce_value", "contact_phone", "contact_email",
    "jwt", "password", "token", "access_token", "refresh_token",
    "username", "real_name", "email", "phone", "avatar", "contact_name",
    "contact_channel", "recipient_ref", "detail", "ip", "user_agent",
    "position", "latitude", "longitude",
))


def _spec(pk, columns):
    fields = {}
    for encoded in columns.split():
        source, pg_type, null = encoded.split(":")
        redacted = (source in SECRET_NAMES or "ciphertext" in source
                    or pg_type in ("json", "jsonb", "bytea"))
        name = source + "_sha256" if redacted else source
        require(name not in fields and IDENTIFIER.fullmatch(source), "REGISTRY_INVALID")
        fields[name] = Field(source, pg_type, null == "?")
        if redacted:
            fields[name] = Field(source, pg_type, null == "?", True)
    keys = tuple(pk.split())
    require(keys and all(key in fields and not fields[key].nullable for key in keys),
            "REGISTRY_PK_INVALID")
    return TableSpec(keys, fields, keys)


# Explicit registry generated from reviewed current 0001 DDL; tests compare the
# contract against source and capture rejects live type/nullability/PK drift.
TABLES = {"platform": {}, "portal": {}}


TABLES['platform']['applications'] = _spec('id',
    'id:uuid:! code:varchar:! name:varchar:! icon:varchar:? base_path:varchar:? description:text:? sort_order:int4:! status:varchar:! created_at:timestamptz:! updated_at:timestamptz:! entry_type:varchar:! entry_url:varchar:! runtime_mode:varchar:! allowed_member_types:_text:! access_policy_version:int8:!')
TABLES['platform']['audit_logs'] = _spec('id',
    'id:int8:! user_id:uuid:? username:varchar:? tenant_id:uuid:? tenant_name:varchar:? organization_id:uuid:? organization_name:varchar:? action:varchar:! detail:text:? ip:varchar:? created_at:timestamptz:! user_agent:varchar:! request_id:uuid:? application_id:uuid:?')
TABLES['platform']['auth_access_token_scopes'] = _spec('token_id resource_id scope_id',
    'token_id:uuid:! resource_id:uuid:! scope_id:uuid:!')
TABLES['platform']['auth_access_tokens'] = _spec('id',
    'id:uuid:! grant_id:uuid:! token_digest:text:! expires_at:timestamptz:! revoked_at:timestamptz:? created_at:timestamptz:!')
TABLES['platform']['auth_application_admissions'] = _spec('application_id environment',
    'application_id:uuid:! environment:text:! status:text:! redirect_uris:_text:! post_logout_redirect_uris:_text:! scopes:_text:! requested_by:uuid:! reviewed_by:uuid:! published_by:uuid:? contact_reference:uuid:! domain_reference:uuid:! review_reference:uuid:! version:int8:! updated_at:timestamptz:! logout_mode:text:! backchannel_logout_uri:text:?')
TABLES['platform']['auth_applications'] = _spec('application_id',
    'application_id:uuid:! partner_id:uuid:? ownership:text:! status:text:! display_name:text:! purpose:text:! approved_scopes:_text:! projection_version:int8:! projected_at:timestamptz:!')
TABLES['platform']['auth_authorization_codes'] = _spec('id',
    'id:uuid:! request_id:uuid:! client_id:uuid:! user_id:uuid:! consent_id:uuid:! consent_generation:int8:! consent_group_version:int8:! code_digest:text:! expires_at:timestamptz:! consumed_at:timestamptz:? created_at:timestamptz:!')
TABLES['platform']['auth_authorization_requests'] = _spec('id',
    'id:uuid:! client_id:uuid:! user_id:uuid:? source_session_id:uuid:? account_version:int8:? auth_time:timestamptz:? client_policy_version:int8:! application_projection_version:int8:! redirect_uri:text:! state_digest:text:! state_value:text:! nonce_digest:text:? nonce_value:text:! code_challenge:text:! code_challenge_method:text:! scope_ids:_uuid:! browser_digest:text:! csrf_digest:text:! force_login:bool:! force_consent:bool:! max_age_seconds:int8:? prior_session_id:uuid:? status:text:! version:int8:! created_at:timestamptz:! expires_at:timestamptz:! consumed_at:timestamptz:?')
TABLES['platform']['auth_client_credentials'] = _spec('id',
    'id:uuid:! client_id:uuid:! client_type:text:! credential_version:int8:! secret_digest:text:? public_key_thumbprint:text:? created_at:timestamptz:! expires_at:timestamptz:? revoked_at:timestamptz:?')
TABLES['platform']['auth_client_permissions'] = _spec('client_id resource_id scope_id',
    'client_id:uuid:! resource_id:uuid:! scope_id:uuid:! expires_at:timestamptz:? version:int8:!')
TABLES['platform']['auth_clients'] = _spec('id',
    'id:uuid:! client_id:text:! display_name:text:! application_id:uuid:! environment:text:! client_type:text:! runtime_kind:text:! subject_mode:text:! context_mode:text:? allowed_member_types:_text:! grant_types:_text:! token_endpoint_auth_method:text:! redirect_uris:_text:! post_logout_redirect_uris:_text:! status:text:! version:int8:! policy_version:int8:! created_at:timestamptz:! updated_at:timestamptz:! logout_mode:text:! backchannel_logout_uri:text:? backchannel_target_id:uuid:?')
TABLES['platform']['auth_consent_group_clients'] = _spec('group_id client_id',
    'group_id:uuid:! client_id:uuid:! application_id:uuid:! environment:text:! subject_mode:text:! context_mode:text:!')
TABLES['platform']['auth_consent_groups'] = _spec('id',
    'id:uuid:! application_id:uuid:! environment:text:! display_name:text:! purpose:text:! subject_mode:text:! context_mode:text:! status:text:! version:int8:!')
TABLES['platform']['auth_consent_revocation_outbox'] = _spec('id',
    'id:uuid:! consent_id:uuid:! user_id:uuid:! group_id:uuid:! client_id:uuid:! consent_generation:int8:! event_kind:text:! created_at:timestamptz:! available_at:timestamptz:! delivered_at:timestamptz:? attempt_count:int4:! last_error_code:text:? expanded_at:timestamptz:? terminal_at:timestamptz:? delivery_state:text:!')
TABLES['platform']['auth_consent_scopes'] = _spec('consent_id resource_id scope_id',
    'consent_id:uuid:! resource_id:uuid:! scope_id:uuid:! granted_at:timestamptz:!')
TABLES['platform']['auth_consents'] = _spec('id',
    'id:uuid:! user_id:uuid:! group_id:uuid:! status:text:! generation:int8:! group_version:int8:! version:int8:! authorized_at:timestamptz:? revoked_at:timestamptz:? display_deleted_at:timestamptz:?')
TABLES['platform']['auth_context_interactions'] = _spec('transaction_id',
    'transaction_id:uuid:! request_hash:text:! client_id:uuid:! client_policy_version:int8:! application_id:uuid:! application_projection_version:int8:! redirect_uri:text:! state_digest:text:! nonce_digest:text:! code_challenge:text:! intent:jsonb:! authorization_request_id:uuid:? source_digest:text:? selected_membership_id:uuid:? credential_ciphertext:text:? selection_expires_at:timestamptz:? grant_id:uuid:? delivered_at:timestamptz:? created_at:timestamptz:! expires_at:timestamptz:!')
TABLES['platform']['auth_delivery_jobs'] = _spec('id',
    'id:uuid:! kind:text:! event_key:text:! target_id:uuid:? target_version:int8:? grant_id:uuid:? invitation_id:uuid:? security_event_id:uuid:? consent_event_id:uuid:? state:text:! available_at:timestamptz:! created_at:timestamptz:! attempt_count:int4:! max_attempts:int4:! lease_id:uuid:? lease_until:timestamptz:? last_jti:uuid:? last_error_code:text:? delivered_at:timestamptz:? terminal_at:timestamptz:?')
TABLES['platform']['auth_delivery_targets'] = _spec('id',
    'id:uuid:! purpose:text:! status:text:! client_id:uuid:? recipient_ref:text:? channel_purpose:text:? url:text:! allowed_ips:_text:! version:int8:! verified_reference:uuid:! verified_at:timestamptz:! expires_at:timestamptz:! created_at:timestamptz:!')
TABLES['platform']['auth_grant_scopes'] = _spec('grant_id resource_id scope_id',
    'grant_id:uuid:! resource_id:uuid:! scope_id:uuid:!')
TABLES['platform']['auth_grants'] = _spec('id',
    'id:uuid:! client_id:uuid:! subject_mode:text:! user_id:uuid:? consent_id:uuid:? consent_generation:int8:? consent_group_version:int8:? source_session_id:uuid:? context_mode:text:! context_ref:uuid:? account_version:int8:? client_policy_version:int8:! application_projection_version:int8:! credential_version:int8:? scope_ids:_uuid:! audiences:_text:! auth_time:timestamptz:? expires_at:timestamptz:! revoked_at:timestamptz:? created_at:timestamptz:! version:int8:!')
TABLES['platform']['auth_invitation_activation_receipts'] = _spec('operation_id',
    'operation_id:uuid:! invitation_id:uuid:! verification_id:uuid:! user_id:uuid:! invitation_version:int8:! verification_version:int8:! delivery_target_id:uuid:! delivery_target_version:int8:! request_digest:text:! key_version:int8:! result_json:jsonb:! created_at:timestamptz:!')
TABLES['platform']['auth_invitation_verifications'] = _spec('id',
    'id:uuid:! invitation_id:uuid:! recipient_ref:text:! purpose:text:! delivery_target_id:uuid:! delivery_target_version:int8:! verified_reference:uuid:! verified_at:timestamptz:! expires_at:timestamptz:! status:text:! version:int8:! created_at:timestamptz:!')
TABLES['platform']['auth_invitations'] = _spec('id',
    'id:uuid:! service_id:uuid:! operation_id:uuid:! request_digest:text:! recipient_ref:text:! purpose:text:! actor_user_id:uuid:! actor_session_id:uuid:! key_version:int8:! token_digest:text:! status:text:! version:int8:! created_at:timestamptz:! expires_at:timestamptz:! revoked_at:timestamptz:? consumed_at:timestamptz:? consumed_by_service_id:uuid:? verification_ref:uuid:? activated_account_id:uuid:? activation_operation_id:uuid:?')
TABLES['platform']['auth_logout_command_receipts'] = _spec('operation_id',
    'operation_id:uuid:! client_id:uuid:! grant_id:uuid:! user_id:uuid:! token_digest:text:! scope:text:! public_sid:text:! public_sub:text:! request_digest:text:! result_json:jsonb:! created_at:timestamptz:!')
TABLES['platform']['auth_mfa_recovery_approvals'] = _spec('operation_id approver_user_id',
    'operation_id:uuid:! approver_user_id:uuid:! primary_authenticated_at:timestamptz:! mfa_verified_at:timestamptz:! approval_digest:text:!')
TABLES['platform']['auth_mfa_recovery_receipts'] = _spec('operation_id',
    'operation_id:uuid:! target_user_id:uuid:! request_digest:text:! result_json:jsonb:! created_at:timestamptz:!')
TABLES['platform']['auth_refresh_families'] = _spec('id',
    'id:uuid:! grant_id:uuid:! absolute_expires_at:timestamptz:! idle_expires_at:timestamptz:! revoked_at:timestamptz:? created_at:timestamptz:! version:int8:!')
TABLES['platform']['auth_refresh_tokens'] = _spec('id',
    'id:uuid:! family_id:uuid:! token_digest:text:! generation:int8:! expires_at:timestamptz:! consumed_at:timestamptz:? revoked_at:timestamptz:? created_at:timestamptz:!')
TABLES['platform']['auth_resources'] = _spec('id',
    'id:uuid:! resource_key:text:! audience:text:! status:text:! version:int8:!')
TABLES['platform']['auth_rp_sessions'] = _spec('grant_id',
    'grant_id:uuid:! client_id:uuid:! user_id:uuid:! source_session_id:uuid:! public_sid:text:! public_sub:text:! created_at:timestamptz:!')
TABLES['platform']['auth_scopes'] = _spec('id',
    'id:uuid:! resource_id:uuid:! scope_name:text:! description:text:! status:text:! version:int8:!')
TABLES['platform']['auth_security_notification_outbox'] = _spec('id',
    'id:uuid:! user_id:uuid:! event_kind:text:! created_at:timestamptz:! available_at:timestamptz:! delivered_at:timestamptz:? attempt_count:int4:! last_error_code:text:? expanded_at:timestamptz:? terminal_at:timestamptz:? delivery_state:text:!')
TABLES['platform']['auth_subject_links'] = _spec('id',
    'id:uuid:! user_id:uuid:! sector_id:uuid:! subject:text:! retired_at:timestamptz:? created_at:timestamptz:!')
TABLES['platform']['auth_subject_sectors'] = _spec('id',
    'id:uuid:! application_id:uuid:! environment:text:! status:text:! version:int8:!')
TABLES['platform']['auth_totp_enrollments'] = _spec('user_id',
    'user_id:uuid:! secret_ciphertext:bytea:! created_at:timestamptz:! expires_at:timestamptz:!')
TABLES['platform']['auth_totp_factors'] = _spec('user_id',
    'user_id:uuid:! secret_ciphertext:bytea:! enabled_at:timestamptz:! last_accepted_step:int8:! version:int8:!')
TABLES['platform']['auth_totp_recovery_codes'] = _spec('id',
    'id:uuid:! user_id:uuid:! code_digest:text:! created_at:timestamptz:! consumed_at:timestamptz:?')
TABLES['platform']['bff_sessions'] = _spec('id',
    'id:uuid:! browser_id:uuid:! token_hash:text:! csrf_hash:text:! user_id:uuid:! account_version:int8:! global_session_id:uuid:! oidc_client_id:uuid:! oidc_grant_id:uuid:! oidc_client_policy_version:int8:! issuer:text:! auth_time:int8:! nonce_hash:text:! access_ciphertext:text:! refresh_ciphertext:text:! access_expires_at:timestamptz:! version:int8:! selected_session_id:uuid:? platform_refresh_ciphertext:text:? refresh_lease_id:uuid:? refresh_started_at:timestamptz:? refresh_uncertain:bool:! logout_request_id:uuid:? logout_scope:text:? created_at:timestamptz:! expires_at:timestamptz:! revoked_at:timestamptz:? oidc_public_sid:text:! oidc_public_sub:text:! logout_confirmed_at:timestamptz:?')
TABLES['platform']['console_browsers'] = _spec('id',
    'id:uuid:! token_hash:text:! generation:int8:! current_session_id:uuid:? created_at:timestamptz:! expires_at:timestamptz:!')
TABLES['platform']['console_login_flows'] = _spec('id',
    'context_selection_required:bool:! switch_session_id:uuid:? switch_version:int8:? id:uuid:! browser_id:uuid:! browser_generation:int8:! state_hash:text:! nonce_hash:text:! verifier_ciphertext:text:! return_path:text:! created_at:timestamptz:! expires_at:timestamptz:! consumed_at:timestamptz:?')
TABLES['platform']['context_selection_registrations'] = _spec('client_id',
    'client_id:uuid:! client_registry:text:! client_policy_version:int8:! member_type:text:! application_id:uuid:! application_policy_version:int8:! consumer_service_id:uuid:? enabled:bool:!')
TABLES['platform']['context_selection_services'] = _spec('id',
    'id:uuid:! secret_hash:text:! purpose:text:! allowed_auth_clients:_uuid:! status:text:!')
TABLES['platform']['context_selections'] = _spec('transaction_id',
    'transaction_id:uuid:! interaction_id:uuid:! client_id:uuid:! consumer_service_id:uuid:? credential_hash:text:! credential_ciphertext:text:? selection:jsonb:! created_at:timestamptz:! expires_at:timestamptz:! consumed_at:timestamptz:? result:jsonb:? result_ciphertext:text:? resource_revoked_at:timestamptz:?')
TABLES['platform']['customer_portal_contexts'] = _spec('id',
    'id:uuid:! token_hash:text:! issuer:text:! oidc_client_id:uuid:! oidc_grant_id:uuid:! global_session_id:uuid:! user_id:uuid:! account_version:int8:! oidc_client_policy_version:int8:! auth_time:int8:! source_expires_at:timestamptz:! profile_client_ref:uuid:! profile_client_policy_version:int8:! application_id:uuid:! application_policy_version:int8:! tenant_id:uuid:! organization_id:uuid:! membership_id:uuid:! member_type:text:! identity_version:int8:! expires_at:timestamptz:! revoked_at:timestamptz:? created_at:timestamptz:! bff_session_id:uuid:? bff_context_version:int8:?')
TABLES['platform']['identity_account_command_receipts'] = _spec('operation_id',
    'operation_id:uuid:! service_id:uuid:! target_user_id:uuid:! canonical_digest:text:! state:text:! before_version:int8:! after_version:int8:! revoked_sessions:int8:! revoked_tokens:int8:! created_at:timestamptz:!')
TABLES['platform']['identity_account_revocation_facts'] = _spec('user_id account_version',
    'user_id:uuid:! account_version:int8:! platform_session_count:int8:! created_at:timestamptz:!')
TABLES['platform']['identity_account_tombstones'] = _spec('user_id',
    'user_id:uuid:! operation_id:uuid:! retired_at:timestamptz:!')
TABLES['platform']['identity_audit_logs'] = _spec('id',
    'id:uuid:! user_id:uuid:? username:text:! action:text:! detail:text:! ip:text:! user_agent:text:! created_at:timestamptz:!')
TABLES['platform']['identity_consent_command_receipts'] = _spec('operation_id',
    'operation_id:uuid:! user_id:uuid:! consent_id:uuid:! request_digest:text:! action:text:! expected_version:int8:! result_json:jsonb:! created_at:timestamptz:!')
TABLES['platform']['identity_deployment_state'] = _spec('singleton',
    'singleton:bool:! deployment_id:uuid:! issuer:text:! schema_version:int8:! migration_owner:name:! identity_owner:name:! platform_owner:name:! revocation_bridge_owner:name:! projection_owner:name:! identity_runtime:name:! platform_runtime:name:! ready_at:timestamptz:!')
TABLES['platform']['identity_auth_command_receipts'] = _spec('operation_id',
    'operation_id:uuid:! service_id:uuid:! request_digest:text:! service_secret_hash:text:! method:text:! path:text:! result_json:jsonb:! returns_secret:bool:! created_at:timestamptz:!')
TABLES['platform']['identity_service_clients'] = _spec('id',
    'id:uuid:! secret_hash:text:! purpose:text:! status:text:! created_at:timestamptz:! allowed_auth_clients:_uuid:! allowed_auth_resources:_uuid:!')
TABLES['platform']['membership_data_grants'] = _spec('id',
    'id:uuid:! membership_id:uuid:! application_id:uuid:! resource_code:varchar:! action_code:varchar:! scope_type:varchar:! created_at:timestamptz:! updated_at:timestamptz:!')
TABLES['platform']['memberships'] = _spec('id',
    'id:uuid:! user_id:uuid:! organization_id:uuid:! department_id:uuid:? position:varchar:? created_at:timestamptz:! member_type:varchar:! status:varchar:! identity_version:int8:! updated_at:timestamptz:!')
TABLES['platform']['menus'] = _spec('id',
    'id:uuid:! application_id:uuid:! parent_id:uuid:? type:varchar:! title:varchar:! path:varchar:? component:varchar:? icon:varchar:? permission:varchar:? sort_order:int4:! visible:bool:! status:varchar:! created_at:timestamptz:! updated_at:timestamptz:! link_url:varchar:! link_target:varchar:!')
TABLES['platform']['auth_provider_settings'] = _spec('singleton',
    'singleton:bool:! issuer:text:! created_at:timestamptz:!')
TABLES['platform']['auth_signing_keys'] = _spec('kid',
    'kid:text:! n:text:! e:text:! published_at:timestamptz:! active:bool:! activated_at:timestamptz:? retired_at:timestamptz:? revoked_at:timestamptz:? last_signed_exp:timestamptz:?')
TABLES['platform']['organization_applications'] = _spec('organization_id application_id',
    'organization_id:uuid:! application_id:uuid:! created_at:timestamptz:!')
TABLES['platform']['organization_menus'] = _spec('organization_id menu_id',
    'organization_id:uuid:! menu_id:uuid:! created_at:timestamptz:!')
TABLES['platform']['organizations'] = _spec('id',
    'id:uuid:! tenant_id:uuid:! parent_id:uuid:? name:varchar:! code:varchar:? sort_order:int4:! description:text:? status:varchar:! created_at:timestamptz:! updated_at:timestamptz:! expires_at:timestamptz:? province:varchar:! city:varchar:! district:varchar:! address:varchar:! longitude:float8:? latitude:float8:? township:varchar:! village:varchar:! organization_type:varchar:! contact_name:varchar:! contact_phone:varchar:! contact_email:varchar:!')
TABLES['platform']['platform_account_command_intents'] = _spec('id',
    'id:uuid:! target_user_id:uuid:! actor_user_id:uuid:! actor_tenant_id:uuid:! actor_organization_id:uuid:! actor_session_id:uuid:! expected_account_version:int8:! canonical_digest:text:! command_kind:text:! expires_at:timestamptz:! state:text:! followup:jsonb:! receipt:jsonb:? created_at:timestamptz:! completed_at:timestamptz:?')
TABLES['platform']['platform_app_capabilities'] = _spec('token_hash',
    'token_hash:text:! id:uuid:! jti:uuid:! user_id:uuid:! tenant_id:uuid:! organization_id:uuid:! application_id:uuid:! audience:text:! scope:text:! platform_session_id:uuid:! membership_id:uuid:! member_type:text:! identity_version:int8:! account_version:int8:! application_policy_version:int8:! origin_issuer:text:! origin_client_ref:text:! origin_client_policy_version:int8:! issued_at:timestamptz:! expires_at:timestamptz:! revoked:bool:!')
TABLES['platform']['platform_auth_application_operators'] = _spec('application_id user_id',
    'application_id:uuid:! user_id:uuid:! assigned_by:uuid:! assigned_at:timestamptz:!')
TABLES['platform']['platform_auth_application_requests'] = _spec('id',
    'id:uuid:! application_id:uuid:! environment:text:! manifest:jsonb:! requested_by:uuid:! submitted_by:uuid:! status:text:! reviewed_by:uuid:? published_by:uuid:? reviewed_contact_reference:uuid:? reviewed_domain_reference:uuid:? test_evidence_reference:uuid:? reason:text:! version:int8:! synced_version:int8:! identity_projection_version:int8:? identity_admission_version:int8:? remote_operation_id:uuid:? created_at:timestamptz:! updated_at:timestamptz:!')
TABLES['platform']['platform_auth_applications'] = _spec('application_id',
    'application_id:uuid:! partner_id:uuid:! display_name:text:! created_by:uuid:! created_at:timestamptz:!')
TABLES['platform']['platform_auth_partners'] = _spec('id',
    'id:uuid:! name:text:! contact_name:text:! contact_channel:text:! subject_reference:uuid:! status:text:! created_by:uuid:! version:int8:! created_at:timestamptz:! updated_at:timestamptz:!')
TABLES['platform']['platform_auth_release_bindings'] = _spec('singleton',
    'singleton:bool:! enabled:bool:! reviewer_user_ids:_uuid:! publisher_user_ids:_uuid:! policy_reference:text:! operations_reference:text:! verified_at:timestamptz:?')
TABLES['platform']['platform_auth_review_operations'] = _spec('operation_id',
    'operation_id:uuid:! actor_user_id:uuid:! actor_session_id:uuid:! request_digest:text:! action:text:! management_permission:text:! application_id:uuid:? request_id:uuid:? remote_required:bool:! result_json:jsonb:! remote_result:jsonb:? created_at:timestamptz:! remote_applied_at:timestamptz:?')
TABLES['platform']['platform_backchannel_logout_receipts'] = _spec('id',
    'id:uuid:! issuer:text:! client_id:uuid:! jti_digest:text:! issued_at:timestamptz:! expires_at:timestamptz:! received_at:timestamptz:!')
TABLES['platform']['platform_backchannel_logout_sources'] = _spec('id',
    'id:uuid:! issuer:text:! client_id:uuid:! sid_digest:text:! sub_digest:text:! revoked_at:timestamptz:!')
TABLES['platform']['platform_machine_delegations'] = _spec('id',
    'id:uuid:! client_id:uuid:! source_application_id:uuid:! credential_version:int8:! application_id:uuid:! tenant_id:uuid:! organization_id:uuid:? resource:text:! action:text:! scope_type:text:! organization_ids:_uuid:! version:int8:! status:text:! created_at:timestamptz:! updated_at:timestamptz:! expires_at:timestamptz:?')
TABLES['platform']['platform_auth_command_intents'] = _spec('id',
    'id:uuid:! actor_user_id:uuid:! tenant_id:uuid:! organization_id:uuid:! session_id:uuid:! service_client_id:uuid:! request_digest:text:! method:text:! path:text:! management_permission:text:! application_ref:uuid:? accepted_at:timestamptz:!')
TABLES['platform']['platform_resource_bindings'] = _spec('application_id resource_code action_code',
    'application_id:uuid:! resource_code:text:! action_code:text:! canonical_resource:text:! audience:text:! oauth_scope:text:! enabled:bool:!')
TABLES['platform']['platform_resource_services'] = _spec('id',
    'id:uuid:! service_id:text:! secret_hash:text:! credential_version:int8:! purpose:text:! application_id:uuid:! status:text:! allowed_member_types:_text:! allowed_operations:_text:! audience:text:! created_at:timestamptz:! updated_at:timestamptz:!')
TABLES['platform']['platform_session_refresh_tokens'] = _spec('id',
    'id:uuid:! session_id:uuid:! token_hash:varchar:! session_version:int8:! issued_at:timestamptz:! expires_at:timestamptz:! consumed_at:timestamptz:?')
TABLES['platform']['platform_sessions'] = _spec('id',
    'id:uuid:! user_id:uuid:! tenant_id:uuid:! organization_id:uuid:! issued_at:timestamptz:! expires_at:timestamptz:! revoked_at:timestamptz:? ip:varchar:! user_agent:text:! account_version:int8:? membership_id:uuid:? member_type:text:? identity_version:int8:? origin_issuer:text:? origin_client_ref:text:? origin_client_policy_version:int8:? bff_session_id:uuid:? bff_context_version:int8:?')
TABLES['platform']['role_menus'] = _spec('role_id menu_id',
    'role_id:uuid:! menu_id:uuid:!')
TABLES['platform']['roles'] = _spec('id',
    'id:uuid:! tenant_id:uuid:! organization_id:uuid:! name:varchar:! code:varchar:? description:text:? status:varchar:! built_in:bool:! created_at:timestamptz:! updated_at:timestamptz:!')
TABLES['platform']['sso_login_challenges'] = _spec('id',
    'id:uuid:! token_hash:text:! csrf_hash:text:! created_at:timestamptz:! expires_at:timestamptz:! consumed_at:timestamptz:?')
TABLES['platform']['sso_sessions'] = _spec('id',
    'id:uuid:! user_id:uuid:! token_hash:text:! csrf_hash:text:! account_version:int8:! authenticated_at:timestamptz:! mfa_verified_at:timestamptz:? mfa_method:text:? expires_at:timestamptz:! revoked_at:timestamptz:?')
TABLES['platform']['tenant_applications'] = _spec('tenant_id application_id',
    'tenant_id:uuid:! application_id:uuid:! created_at:timestamptz:!')
TABLES['platform']['tenant_menus'] = _spec('tenant_id menu_id',
    'tenant_id:uuid:! menu_id:uuid:! created_at:timestamptz:!')
TABLES['platform']['tenants'] = _spec('id',
    'id:uuid:! name:varchar:! code:varchar:! description:text:? status:varchar:! created_at:timestamptz:! updated_at:timestamptz:! is_platform:bool:! expires_at:timestamptz:? province:varchar:! city:varchar:! district:varchar:! address:varchar:! longitude:float8:? latitude:float8:? township:varchar:! village:varchar:! tenant_type:varchar:! contact_name:varchar:! contact_phone:varchar:! contact_email:varchar:!')
TABLES['platform']['user_roles'] = _spec('user_id role_id',
    'user_id:uuid:! role_id:uuid:! organization_id:uuid:! created_at:timestamptz:!')
TABLES['platform']['users'] = _spec('id',
    'id:uuid:! username:varchar:! password_hash:text:! real_name:varchar:? email:varchar:? phone:varchar:? avatar:text:? gender:varchar:? status:varchar:! is_super_admin:bool:! last_login_at:timestamptz:? created_at:timestamptz:! updated_at:timestamptz:! session_version:int8:!')
TABLES['portal']['portal_backchannel_logout_receipts'] = _spec('id',
    'id:uuid:! issuer:text:! client_id:uuid:! jti_digest:text:! issued_at:timestamptz:! expires_at:timestamptz:! received_at:timestamptz:!')
TABLES['portal']['portal_backchannel_logout_sources'] = _spec('id',
    'id:uuid:! issuer:text:! client_id:uuid:! sid_digest:text:! sub_digest:text:! revoked_at:timestamptz:!')
TABLES['portal']['portal_browsers'] = _spec('id',
    'id:uuid:! token_hash:text:! generation:int8:! current_session_id:uuid:? created_at:timestamptz:! expires_at:timestamptz:!')
TABLES['portal']['portal_login_flows'] = _spec('id',
    'context_selection_required:bool:! switch_session_id:uuid:? switch_version:int8:? id:uuid:! browser_id:uuid:! browser_generation:int8:! state_hash:text:! binder_hash:text:! nonce_hash:text:! verifier_ciphertext:text:! return_path:text:! created_at:timestamptz:! expires_at:timestamptz:! consumed_at:timestamptz:?')
TABLES['portal']['portal_sessions'] = _spec('id',
    'pending_selection_ciphertext:text:? id:uuid:! browser_id:uuid:! token_hash:text:! user_id:uuid:! account_version:int8:! global_session_id:uuid:! oidc_client_id:uuid:! oidc_grant_id:uuid:! oidc_client_policy_version:int8:! issuer:text:! auth_time:int8:! nonce_hash:text:! access_ciphertext:text:! refresh_ciphertext:text:! access_expires_at:timestamptz:! version:int8:! membership_id:uuid:? organization_id:uuid:? identity_version:int8:? customer_ciphertext:text:? customer_expires_at:timestamptz:? customer_needs_renewal:bool:! refresh_lease_id:uuid:? refresh_started_at:timestamptz:? refresh_uncertain:bool:! context_lease_id:uuid:? context_started_at:timestamptz:? logout_request_id:uuid:? logout_scope:text:? logout_confirmed_at:timestamptz:? created_at:timestamptz:! expires_at:timestamptz:! revoked_at:timestamptz:? oidc_public_sid:text:! oidc_public_sub:text:!')


APPEND_ONLY = frozenset((
    "identity_account_command_receipts", "identity_account_revocation_facts",
    "identity_account_tombstones", "identity_audit_logs", "audit_logs",
    "identity_auth_command_receipts", "identity_consent_command_receipts",
    "auth_mfa_recovery_receipts", "auth_mfa_recovery_approvals",
    "auth_logout_command_receipts", "auth_invitation_activation_receipts",
    "auth_rp_sessions", "auth_subject_links", "auth_signing_keys",
    "auth_consents", "auth_grants", "auth_refresh_families", "auth_refresh_tokens",
    "auth_access_tokens", "auth_delivery_jobs", "auth_consent_revocation_outbox",
    "auth_security_notification_outbox", "auth_invitations", "auth_invitation_verifications",
    "platform_backchannel_logout_sources", "platform_backchannel_logout_receipts",
    "portal_backchannel_logout_sources", "portal_backchannel_logout_receipts",
))
IMMUTABLE = {
    "auth_signing_keys": ("kid", "n", "e", "published_at"),
    "auth_subject_links": ("id", "user_id", "sector_id", "subject", "created_at"),
    "auth_rp_sessions": "*",
    "auth_logout_command_receipts": "*",
    "auth_invitation_activation_receipts": "*",
    "identity_auth_command_receipts": "*",
    "identity_account_command_receipts": "*",
    "identity_account_revocation_facts": "*",
    "identity_account_tombstones": "*",
    "identity_consent_command_receipts": "*",
    "auth_mfa_recovery_receipts": "*",
    "auth_mfa_recovery_approvals": "*",
    "identity_audit_logs": "*", "audit_logs": "*",
    "platform_backchannel_logout_sources": "*",
    "platform_backchannel_logout_receipts": "*",
    "portal_backchannel_logout_sources": "*",
    "portal_backchannel_logout_receipts": "*",
    "auth_delivery_jobs": ("id", "kind", "event_key", "target_id", "target_version",
                           "grant_id", "invitation_id", "security_event_id",
                           "created_at", "max_attempts"),
    "bff_sessions": ("id", "browser_id", "user_id", "global_session_id",
                     "oidc_client_id", "oidc_grant_id", "issuer", "oidc_public_sid",
                     "oidc_public_sub", "account_version", "oidc_client_policy_version",
                     "auth_time", "token_hash", "csrf_hash", "nonce_hash", "created_at"),
    "portal_sessions": ("id", "browser_id", "user_id", "global_session_id",
                        "oidc_client_id", "oidc_grant_id", "issuer", "oidc_public_sid",
                        "oidc_public_sub", "account_version", "oidc_client_policy_version",
                        "auth_time", "token_hash", "nonce_hash", "created_at"),
    "platform_app_capabilities": ("id", "jti", "token_hash", "platform_session_id",
                                  "membership_id", "user_id", "tenant_id", "organization_id",
                                  "application_id", "audience", "scope", "member_type", "account_version",
                                  "identity_version", "application_policy_version", "origin_issuer",
                                  "origin_client_ref", "origin_client_policy_version", "expires_at", "issued_at"),
    "auth_grants": ("id", "client_id", "subject_mode", "user_id", "consent_id",
                    "consent_generation", "consent_group_version", "source_session_id",
                    "context_mode", "context_ref", "account_version", "client_policy_version",
                    "application_projection_version", "credential_version", "scope_ids",
                    "audiences", "auth_time", "created_at", "expires_at"),
    "auth_client_credentials": ("id", "client_id", "client_type", "credential_version",
                                "secret_digest_sha256", "public_key_thumbprint", "created_at"),
    "auth_access_tokens": ("id", "grant_id", "token_digest", "created_at", "expires_at"),
    "auth_refresh_tokens": ("id", "family_id", "token_digest", "generation", "created_at", "expires_at"),
    "auth_refresh_families": ("id", "grant_id", "created_at", "absolute_expires_at"),
    "auth_authorization_codes": ("id", "request_id", "client_id", "user_id", "consent_id",
                                 "consent_generation", "consent_group_version", "code_digest",
                                 "created_at", "expires_at"),
    "auth_invitations": ("id", "service_id", "operation_id", "request_digest", "recipient_ref_sha256",
                         "purpose", "actor_user_id", "actor_session_id", "key_version",
                         "token_digest", "created_at", "expires_at"),
    "auth_invitation_verifications": ("id", "invitation_id", "recipient_ref_sha256", "purpose",
                                      "delivery_target_id", "delivery_target_version", "verified_reference",
                                      "verified_at", "expires_at", "created_at"),
    "context_selections": ("transaction_id", "interaction_id", "client_id", "consumer_service_id",
                           "credential_hash", "selection_sha256", "created_at", "expires_at"),
}


def _finish_registry():
    for kind, tables in TABLES.items():
        for name, spec in list(tables.items()):
            immutable = IMMUTABLE.get(name, spec.pk)
            if immutable == "*":
                immutable = tuple(spec.fields)
            # Some internal-capability columns differ from their legacy names;
            # registry construction must not infer a column absent from 0001.
            require(all(field in spec.fields for field in immutable), "IMMUTABLE_REGISTRY_INVALID")
            tables[name] = TableSpec(spec.pk, spec.fields, tuple(immutable), name in APPEND_ONLY)


_finish_registry()
SCHEMA_SHA256 = digest({kind: {name: {"pk": spec.pk,
    "fields": {field: vars(value) for field, value in spec.fields.items()},
    "immutable": spec.immutable, "append_only": spec.append_only}
    for name, spec in sorted(tables.items())} for kind, tables in sorted(TABLES.items())})


def empty_inventory():
    return {"format": FORMAT, "schema_sha256": SCHEMA_SHA256,
            "databases": {kind: {name: [] for name in sorted(tables)}
                          for kind, tables in sorted(TABLES.items())}}


def _timestamp(value):
    require(type(value) is str and 10 <= len(value) <= 40, "TIMESTAMP_INVALID")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        require(parsed.tzinfo is not None, "TIMESTAMP_ZONE_REQUIRED")
        return parsed.astimezone(timezone.utc)
    except (ValueError, OverflowError):
        raise InventoryError("INVENTORY_TIMESTAMP_INVALID") from None


def _value(field, value):
    if value is None:
        require(field.nullable, "NULL_INVALID")
        return
    if field.redacted:
        require(type(value) is str and HEX.fullmatch(value), "REDACTED_DIGEST_INVALID")
        return
    kind = field.pg_type
    if kind.startswith("_"):
        require(type(value) is list and len(value) <= 2048, "ARRAY_INVALID")
        for item in value:
            _value(Field(field.source, kind[1:], False), item)
    elif kind == "uuid":
        try:
            parsed = uuid.UUID(value) if type(value) is str else None
        except (ValueError, AttributeError):
            parsed = None
        require(parsed is not None and str(parsed) == value and parsed.version == 7
                and parsed.variant == uuid.RFC_4122, "UUID_INVALID")
    elif kind == "bool":
        require(type(value) is bool, "BOOLEAN_INVALID")
    elif kind in ("int2", "int4", "int8"):
        require(type(value) is int and -(1 << 63) < value <= MAX_INTEGER, "INTEGER_INVALID")
        if _is_counter(field.source):
            require(value >= 0, "COUNTER_INVALID")
    elif kind in ("timestamptz", "timestamp"):
        _timestamp(value)
    else:
        require(kind in ("text", "varchar", "bpchar", "name") and type(value) is str
                and len(value) <= MAX_TEXT and "\x00" not in value, "TEXT_INVALID")


def _is_counter(name):
    return (name in ("generation", "version", "attempt_count", "max_attempts",
                     "last_accepted_step", "schema_version", "synced_version")
            or name.endswith(("_version", "_generation", "_count")))


def row_key(spec, row):
    return canonical([row[key] for key in spec.pk]).decode("utf-8")


def _check_semantics(name, row):
    if "created_at" in row and "expires_at" in row and row["expires_at"] is not None:
        require(_timestamp(row["expires_at"]) > _timestamp(row["created_at"]), "DEADLINE_INVALID")
    for field, item in row.items():
        if item is not None and field.endswith(("_hash", "_digest")):
            require(type(item) is str and HEX.fullmatch(item), "DIGEST_INVALID")
        if field in ("version", "policy_version", "identity_version", "credential_version",
                     "group_version", "client_policy_version", "application_projection_version",
                     "application_policy_version", "origin_client_policy_version", "key_version"):
            require(item is None or item >= 1, "VERSION_INVALID")
    enums = {
        "auth_clients": {"client_type": ("CONFIDENTIAL", "PUBLIC"),
                         "runtime_kind": ("WEB_BFF", "SPA", "NATIVE", "SERVICE", "DEVICE"),
                         "subject_mode": ("USER", "MACHINE"),
                         "context_mode": ("ACCOUNT_ONLY", "PLATFORM_CONTEXT", "NONE"),
                         "token_endpoint_auth_method": ("client_secret_basic", "none"),
                         "status": ("ENABLED", "DISABLED", "RETIRED")},
        "auth_grants": {"subject_mode": ("USER", "MACHINE"),
                        "context_mode": ("ACCOUNT_ONLY", "PLATFORM_CONTEXT", "NONE")},
        "auth_invitations": {"status": ("ACTIVE", "REVOKED", "CONSUMED"),
                             "purpose": ("ACCOUNT_ONBOARDING",)},
        "auth_authorization_requests": {"status": ("PENDING", "AUTHORIZED", "DENIED", "EXPIRED", "CONSUMED"),
                                        "code_challenge_method": ("S256",)},
        "auth_delivery_targets": {"status": ("ACTIVE", "REVOKED"),
                                  "purpose": ("BACK_CHANNEL", "VERIFIED_CHANNEL")},
        "auth_invitation_verifications": {"status": ("ACTIVE", "REVOKED"),
                                         "purpose": ("ACCOUNT_ONBOARDING",)},
        "auth_subject_sectors": {"status": ("ACTIVE", "RETIRED")},
        "auth_resources": {"status": ("ACTIVE", "SUSPENDED", "RETIRED")},
        "auth_scopes": {"status": ("ACTIVE", "SUSPENDED", "RETIRED")},
        "auth_consent_groups": {"status": ("ACTIVE", "SUSPENDED", "RETIRED"),
                                "subject_mode": ("USER",), "context_mode": ("ACCOUNT_ONLY", "PLATFORM_CONTEXT")},
        "auth_applications": {"ownership": ("FIRST_PARTY", "THIRD_PARTY"),
                              "status": ("DRAFT", "PENDING_REVIEW", "APPROVED", "SUSPENDED", "RETIRED")},
        "auth_application_admissions": {"status": ("APPROVED", "SUSPENDED"),
                                        "environment": ("test", "production")},
    }
    for field, allowed in enums.get(name, {}).items():
        require(row[field] in allowed, "ENUM_INVALID")
    for value in ("revoked_at", "retired_at", "consumed_at", "display_deleted_at"):
        if value in row and row[value] is not None:
            _timestamp(row[value])
    if name == "auth_signing_keys":
        require(row["e"] == "AQAB" and bool(re.fullmatch(r"[A-Za-z0-9_-]{1,80}", row["kid"])),
                "KEY_INVALID")
        require(not row["active"] or (row["retired_at"] is None and row["revoked_at"] is None),
                "KEY_STATE_INVALID")
    if name == "auth_delivery_jobs":
        require(row["state"] in ("PENDING", "LEASED", "DELIVERED", "TERMINAL", "DEGRADED")
                and 1 <= row["max_attempts"] <= 8
                and row["attempt_count"] <= row["max_attempts"], "DELIVERY_BUDGET_INVALID")
        require((row["state"] == "LEASED") == (row["lease_id"] is not None)
                and (row["lease_id"] is None) == (row["lease_until"] is None), "DELIVERY_LEASE_INVALID")
        require((row["target_id"] is None) == (row["target_version"] is None)
                and (row["state"] == "DEGRADED" or row["target_id"] is not None), "DELIVERY_TARGET_INVALID")
        require((row["kind"] == "LOGOUT" and row["grant_id"] is not None
                 and row["invitation_id"] is None and row["security_event_id"] is None)
                or (row["kind"] == "INVITATION" and row["invitation_id"] is not None
                    and row["grant_id"] is None and row["security_event_id"] is None
                    and row["consent_event_id"] is None)
                or (row["kind"] == "SECURITY" and row["security_event_id"] is not None
                    and row["grant_id"] is None and row["invitation_id"] is None
                    and row["consent_event_id"] is None), "DELIVERY_SOURCE_INVALID")
    if name == "auth_consents":
        require(row["status"] in ("ACTIVE", "REVOKED") and row["generation"] >= 1
                and row["group_version"] >= 1 and row["version"] >= 1,
                "CONSENT_STATE_INVALID")
        require(row["authorized_at"] is not None
                and (row["status"] == "REVOKED") == (row["revoked_at"] is not None)
                and (row["display_deleted_at"] is None or row["status"] == "REVOKED"),
                "CONSENT_STATE_INVALID")
    if name == "auth_grants":
        if row["subject_mode"] == "MACHINE":
            require(row["context_mode"] == "NONE" and row["credential_version"] is not None
                    and all(row[key] is None for key in ("user_id", "consent_id", "consent_generation",
                        "consent_group_version", "source_session_id", "context_ref", "account_version", "auth_time")),
                    "GRANT_BINDING_INVALID")
        else:
            require(row["context_mode"] != "NONE" and row["credential_version"] is None
                    and all(row[key] is not None for key in ("user_id", "consent_id", "consent_generation",
                                                             "consent_group_version")), "GRANT_BINDING_INVALID")
        require((row["context_mode"] == "PLATFORM_CONTEXT") == (row["context_ref"] is not None),
                "GRANT_BINDING_INVALID")
    if name in ("bff_sessions", "portal_sessions"):
        require((row["refresh_lease_id"] is None) == (row["refresh_started_at"] is None)
                and (row["logout_request_id"] is None) == (row["logout_scope"] is None)
                and (row["logout_confirmed_at"] is None or row["logout_request_id"] is not None),
                "BFF_PAIR_INVALID")
        if name == "portal_sessions":
            require((row["context_lease_id"] is None) == (row["context_started_at"] is None)
                    and all((row["membership_id"] is None) == (row[key] is None)
                            for key in ("organization_id", "identity_version", "customer_ciphertext_sha256",
                                        "customer_expires_at")), "BFF_PAIR_INVALID")
    if name == "identity_deployment_state":
        require(row["singleton"] and row["schema_version"] == 1
                and row["issuer"] == "https://auth.saas.test", "DEPLOYMENT_INVALID")
    if name == "auth_provider_settings":
        require(row["singleton"] and row["issuer"] == "https://auth.saas.test", "ISSUER_INVALID")
    if name.endswith("backchannel_logout_sources"):
        require(all(HEX.fullmatch(row[key]) for key in ("sid_digest", "sub_digest")),
                "SOURCE_DIGEST_INVALID")
    if name.endswith("backchannel_logout_receipts"):
        require(HEX.fullmatch(row["jti_digest"]) and _timestamp(row["expires_at"]) > _timestamp(row["issued_at"]),
                "REPLAY_FACT_INVALID")


def validate_inventory(value, previous=None):
    require(type(value) is dict and set(value) == {"format", "schema_sha256", "databases"}
            and value["format"] == FORMAT and value["schema_sha256"] == SCHEMA_SHA256,
            "FORMAT_INVALID")
    databases = value["databases"]
    require(type(databases) is dict and set(databases) == set(TABLES), "DATABASES_INVALID")
    total = 0
    for kind, tables in TABLES.items():
        require(type(databases[kind]) is dict and set(databases[kind]) == set(tables), "FAMILIES_INVALID")
        for name, spec in tables.items():
            rows = databases[kind][name]
            require(type(rows) is list and len(rows) <= MAX_ROWS, "ROWS_INVALID")
            total += len(rows)
            require(total <= MAX_ROWS, "ROW_LIMIT_EXCEEDED")
            seen = set()
            for row in rows:
                require(type(row) is dict and set(row) == set(spec.fields), "COLUMNS_INVALID")
                for column, field in spec.fields.items():
                    _value(field, row[column])
                key = row_key(spec, row)
                require(key not in seen, "DUPLICATE_PK")
                seen.add(key)
                _check_semantics(name, row)
    require(len(canonical(value)) <= MAX_BYTES, "BYTE_LIMIT_EXCEEDED")
    if previous is not None:
        validate_inventory(previous)
        _validate_transition(previous, value)
    return value


def _validate_transition(previous, current):
    for kind, tables in TABLES.items():
        for name, spec in tables.items():
            before = {row_key(spec, row): row for row in previous["databases"][kind][name]}
            after = {row_key(spec, row): row for row in current["databases"][kind][name]}
            require(not spec.append_only or before.keys() <= after.keys(), "DURABLE_FACT_OMITTED")
            for key in before.keys() & after.keys():
                old, new = before[key], after[key]
                require(all(old[field] == new[field] for field in spec.immutable), "IMMUTABLE_FACT_CHANGED")
                for field, value in old.items():
                    if _is_counter(spec.fields[field].source) and value is not None and new[field] is not None:
                        require(new[field] >= value, "HIGHWATER_REGRESSION")
                    if field in ("revoked_at", "retired_at", "consumed_at", "display_deleted_at",
                                 "logout_confirmed_at", "resource_revoked_at") and value is not None:
                        # Repeated revocations may carry a later timestamp, but
                        # clearing an already committed deny fact is forbidden.
                        reauthorized = (name == "auth_consents" and field == "revoked_at"
                                        and new["generation"] > old["generation"]
                                        and new["version"] > old["version"] and new["status"] == "ACTIVE")
                        require(new[field] is not None or reauthorized, "DENY_FACT_CLEARED")
                if name == "auth_delivery_jobs":
                    if old["consent_event_id"] is not None:
                        require(new["consent_event_id"] == old["consent_event_id"], "CONSENT_EVENT_CHANGED")
                    if old["state"] in ("DELIVERED", "TERMINAL", "DEGRADED"):
                        require(all(old[field] == new[field] for field in ("state", "available_at",
                            "attempt_count", "lease_id", "lease_until", "last_jti", "last_error_code",
                            "delivered_at", "terminal_at")), "TERMINAL_FACT_CHANGED")
                if name == "auth_signing_keys" and old["last_signed_exp"] is not None:
                    require(new["last_signed_exp"] is not None
                            and _timestamp(new["last_signed_exp"]) >= _timestamp(old["last_signed_exp"]),
                            "KEY_WINDOW_REGRESSION")
                if name in ("bff_sessions", "portal_sessions"):
                    if old["logout_request_id"] is not None:
                        require(new["logout_request_id"] == old["logout_request_id"]
                                and new["logout_scope"] == old["logout_scope"], "LOGOUT_OPERATION_CHANGED")
                    if old["logout_confirmed_at"] is not None:
                        require(new["logout_confirmed_at"] == old["logout_confirmed_at"], "LOGOUT_CONFIRMATION_CHANGED")
                if name == "context_selections" and old["consumed_at"] is not None:
                    require(all(old[field] == new[field] for field in ("consumed_at", "result_sha256",
                                "result_ciphertext_sha256")), "COMMITTED_SELECTION_CHANGED")
                if name == "platform_app_capabilities" and old["revoked"]:
                    require(new["revoked"], "CAPABILITY_REVIVED")


def highwater(value):
    validate_inventory(value)
    result = {}
    for kind, tables in TABLES.items():
        for name, spec in tables.items():
            for row in value["databases"][kind][name]:
                prefix = kind + "/" + name + "/" + row_key(spec, row) + "/"
                for field, item in row.items():
                    source = spec.fields[field].source
                    if _is_counter(source) and item is not None:
                        result[prefix + field] = item
                    elif source == "last_signed_exp" and item is not None:
                        stamp = _timestamp(item)
                        delta = stamp - datetime(1970, 1, 1, tzinfo=timezone.utc)
                        result[prefix + field] = ((delta.days * 86400 + delta.seconds) * 1000000
                                                 + delta.microseconds)
    require(all(len(key) <= 2048 and 0 <= item <= MAX_INTEGER for key, item in result.items()),
            "HIGHWATER_INVALID")
    return result


def capture_sql(kind):
    require(kind in TABLES, "DATABASE_KIND_INVALID")
    names = sorted(TABLES[kind])
    quoted = ",".join("'" + name + "'" for name in names)
    # LIMIT+1 bounds every aggregate and signals an exceeded rather than silently
    # truncated inventory. ORDER BY PK makes hashes stable across physical order.
    scans = ",".join("('" + name + "',(SELECT COALESCE(jsonb_agg(to_jsonb(t)), '[]'::jsonb) FROM "
        "(SELECT * FROM public.\"" + name + "\" ORDER BY "
        + ",".join('"' + key + '"' for key in TABLES[kind][name].pk)
        + " LIMIT " + str(MAX_ROWS + 1) + ") t))" for name in names)
    return """SELECT jsonb_build_object('catalog',(
      SELECT jsonb_object_agg(c.relname,jsonb_build_object('columns',(
        SELECT jsonb_object_agg(a.attname,jsonb_build_object('type',ty.typname,'nullable',NOT a.attnotnull))
        FROM pg_attribute a JOIN pg_type ty ON ty.oid=a.atttypid
        WHERE a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped),'pk',(
        SELECT jsonb_agg(a.attname ORDER BY k.position)
        FROM pg_constraint p CROSS JOIN LATERAL unnest(p.conkey) WITH ORDINALITY k(attnum,position)
        JOIN pg_attribute a ON a.attrelid=c.oid AND a.attnum=k.attnum
        WHERE p.conrelid=c.oid AND p.contype='p')))
      FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
      WHERE n.nspname='public' AND c.relkind='r' AND
      (c.relname IN (""" + quoted + ") OR left(c.relname,5)='auth_'))," \
      "'tables',(SELECT jsonb_object_agg(name,rows) FROM (VALUES " + scans + ") captured(name,rows)));"


def project_database(kind, raw):
    require(kind in TABLES, "DATABASE_KIND_INVALID")
    if type(raw) is str:
        require(len(raw.encode("utf-8")) <= MAX_BYTES * 4, "RAW_BYTE_LIMIT_EXCEEDED")
        try:
            def pairs(items):
                result = {}
                for key, item in items:
                    require(key not in result, "DUPLICATE_JSON_KEY")
                    result[key] = item
                return result
            raw = json.loads(raw, object_pairs_hook=pairs)
        except (ValueError, UnicodeError, RecursionError):
            raise InventoryError("INVENTORY_RAW_JSON_INVALID") from None
    require(type(raw) is dict and set(raw) == {"catalog", "tables"}, "CAPTURE_SHAPE_INVALID")
    tables = TABLES[kind]
    require(type(raw["catalog"]) is dict and set(raw["catalog"]) == set(tables)
            and type(raw["tables"]) is dict and set(raw["tables"]) == set(tables), "SCHEMA_TABLE_DRIFT")
    result = {}
    for name, spec in tables.items():
        expected = {field.source: {"type": field.pg_type, "nullable": field.nullable}
                    for field in spec.fields.values()}
        require(raw["catalog"][name] == {"columns": expected, "pk": list(spec.pk)}, "SCHEMA_COLUMN_DRIFT")
        rows = raw["tables"][name]
        require(type(rows) is list and len(rows) <= MAX_ROWS, "ROWS_INVALID")
        projected = []
        for row in rows:
            projected.append(project_row(kind, name, row))
        result[name] = sorted(projected, key=lambda row: row_key(spec, row))
    value = empty_inventory()
    value["databases"][kind] = result
    validate_inventory(value)
    return result


def project_row(kind, table, raw_row):
    """Hash a protected raw row exactly as capture does, without exposing it.

    Reconciliation uses this to compare encrypted latest-fact material with its
    independently authenticated public witness before using any original value.
    """
    require(kind in TABLES and table in TABLES[kind], "TABLE_INVALID")
    spec = TABLES[kind][table]
    expected = {field.source for field in spec.fields.values()}
    require(type(raw_row) is dict and set(raw_row) == expected, "RAW_COLUMNS_INVALID")
    row = {column: (digest(raw_row[field.source])
        if field.redacted and raw_row[field.source] is not None else raw_row[field.source])
        for column, field in spec.fields.items()}
    for column, field in spec.fields.items():
        _value(field, row[column])
    _check_semantics(table, row)
    return row


def capture_database(kind, query):
    """Capture one database using a caller-owned parsed-JSON query callable."""
    return project_database(kind, query(capture_sql(kind)))


def capture(query):
    """``query(kind, SQL)`` must query bound, stopped-writer source databases."""
    value = empty_inventory()
    for kind in sorted(TABLES):
        value["databases"][kind] = capture_database(kind, lambda sql: query(kind, sql))
    return validate_inventory(value)
