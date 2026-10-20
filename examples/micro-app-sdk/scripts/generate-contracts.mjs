import { readFileSync, mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const ownSdkRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const ownRoot = path.resolve(ownSdkRoot, '../..')
const slash = (value) => value.split(path.sep).join('/')
const snake = (value) => value.replace(/[A-Z]/g, (letter) => '_' + letter.toLowerCase()).replace(/^_/, '')
const variant = (value) => value === 'SELF' ? 'SelfScope' : /^[0-9]/.test(value) ? 'V' + value.replaceAll('.', 'Dot')
  : value.toLowerCase().split(/[^a-z0-9]+/).map((part) => part[0].toUpperCase() + part.slice(1)).join('')
const nullable = (schema) => Array.isArray(schema.type) && schema.type.includes('null')
const withoutNull = (schema) => ({ ...schema, type: Array.isArray(schema.type) ? schema.type.find((type) => type !== 'null') : schema.type })

function resolveRelease(schema, release) {
  if (Array.isArray(schema)) return schema.map((value) => resolveRelease(value, release))
  if (!schema || typeof schema !== 'object') return schema
  const result = Object.fromEntries(Object.entries(schema).filter(([key]) => !key.startsWith('x-')).map(([key, value]) => [key, resolveRelease(value, release)]))
  if (schema['x-release-contract']) {
    const value = release.contracts?.[schema['x-release-contract']]
    if (typeof value !== 'string' || !/^\d+\.\d+$/.test(value)) throw new Error('Missing registered contract version: ' + schema['x-release-contract'])
    result.const = value
  }
  if (schema['x-release-sdk-version']) {
    if (typeof release.sdkVersion !== 'string' || !/^\d+\.\d+\.\d+$/.test(release.sdkVersion)) throw new Error('Missing registered SDK version')
    result.const = release.sdkVersion
  }
  for (const [constraint, key] of Object.entries(schema['x-release-limit'] ?? {})) {
    const value = release.limits?.[key]
    if (!Number.isSafeInteger(value) || value < 1) throw new Error('Missing registered contract limit: ' + key)
    result[constraint] = value
  }
  return result
}

function tsType(schema) {
  if (schema.$ref) return schema.$ref.split('/').at(-1)
  if (schema.const !== undefined) return JSON.stringify(schema.const)
  if (schema.enum) return schema.enum.map((value) => JSON.stringify(value)).join(' | ')
  if (schema.oneOf) return schema.oneOf.map(tsType).join(' | ')
  if (nullable(schema)) return tsType(withoutNull(schema)) + ' | null'
  if (schema.type === 'array') return '(' + tsType(schema.items) + ')[]'
  if (schema.type === 'object') {
    const required = new Set(schema.required ?? [])
    return '{ ' + Object.entries(schema.properties).map(([key, value]) => key + (required.has(key) ? '' : '?') + ': ' + tsType(value)).join('; ') + ' }'
  }
  return ({ string: 'string', integer: 'number', boolean: 'boolean' })[schema.type] ?? (() => { throw new Error('Unsupported TS schema type') })()
}

function rustType(schema) {
  if (schema.$ref) return schema.$ref.split('/').at(-1)
  if (nullable(schema)) return 'Option<' + rustType(withoutNull(schema)) + '>'
  if (schema.type === 'array') return 'Vec<' + rustType(schema.items) + '>'
  return ({ string: 'String', integer: 'i64', boolean: 'bool' })[schema.type] ?? (() => { throw new Error('Unsupported Rust schema type') })()
}

function rustFields(schema, indent = '    ', omit = []) {
  const required = new Set(schema.required ?? [])
  return Object.entries(schema.properties).filter(([key]) => !omit.includes(key)).map(([key, value]) => {
    const field = snake(key) === 'type' ? 'r#type' : snake(key)
    let type = rustType(value), attrs = ''
    if (!required.has(key)) {
      if (nullable(value)) {
        type = 'OptionalNullable<' + rustType(withoutNull(value)) + '>'
        attrs = '#[serde(default, skip_serializing_if = "OptionalNullable::is_missing")]\n' + indent
      } else {
        type = 'Option<' + type + '>'
        attrs = '#[serde(default, skip_serializing_if = "Option::is_none")]\n' + indent
      }
    } else if (nullable(value)) {
      attrs = '#[serde(deserialize_with = "required_nullable")]\n' + indent
    }
    return indent + attrs + 'pub ' + field + ': ' + type + ','
  }).join('\n')
}

function rustDefinition(name, schema, source) {
  const docs = '/// Public wire type generated from `$defs.' + name + '`; validation is separate from authorization.\n'
  if (schema.enum || schema.const !== undefined) {
    const values = schema.enum ?? [schema.const]
    return docs + '#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]\npub enum ' + name + ' {\n' + values.map((value) => '    #[serde(rename = ' + JSON.stringify(value) + ')]\n    ' + variant(value) + ',').join('\n') + '\n}\n'
  }
  if (source['x-rust-tag']) {
    const tag = source['x-rust-tag']
    return docs + '#[derive(Clone, PartialEq, Serialize, Deserialize)]\n#[serde(tag = ' + JSON.stringify(tag) + ', deny_unknown_fields)]\npub enum ' + name + ' {\n' + schema.oneOf.map((item) => {
      const value = item.properties[tag].const
      const fields = Object.entries(item.properties).filter(([key]) => key !== tag).map(([key, property]) => '        #[serde(rename = ' + JSON.stringify(key) + ')]\n        ' + snake(key) + ': ' + rustType(property) + ',').join('\n')
      return '    #[serde(rename = ' + JSON.stringify(value) + ')]\n    ' + variant(value) + (fields ? ' {\n' + fields + '\n    },' : ' {},')
    }).join('\n') + '\n}\n'
  }
  if (schema.type !== 'object') throw new Error('Unsupported top-level Rust schema: ' + name)
  const extra = source['x-preserve-additional'] ? '\n    #[serde(flatten)]\n    pub additional: std::collections::BTreeMap<String, serde_json::Value>,' : ''
  return docs + '#[derive(Clone, PartialEq, Serialize, Deserialize)]\n#[serde(rename_all = "camelCase"' + (schema.additionalProperties === false ? ', deny_unknown_fields' : '') + ')]\npub struct ' + name + ' {\n' + rustFields(schema) + extra + '\n}\n'
}

// Runtime-only fields are deliberately a small explicit template: AbortSignal and
// functions cannot be described as JSON wire values. All data fields come from schema.
const runtimeTypes = `
/** Host-owned cancellation and callbacks are never serialized or refreshed by the SDK. */
export interface MicroProps extends MicroPropsWire {
  signal: AbortSignal
  onNavigate: (path: string) => void
  onNotify: (message: string, variant?: MicroNotifyVariant) => void
}
export interface MicroLifecycle {
  name: string
  mount: (container: HTMLElement, props: MicroProps) => void | Promise<void>
  unmount: () => void | Promise<void>
  update?: (props: MicroProps) => void | Promise<void>
}
export interface MicroEsmModule {
  __MICRO_APP__?: MicroLifecycle
  mount?: MicroLifecycle["mount"]
  unmount?: MicroLifecycle["unmount"]
  update?: MicroLifecycle["update"]
}
`

// Explicit semantic helpers complement generated DTOs. Missing/null/list must not
// collapse into Option<Vec<_>>: only an authoritative server may emit null.
const rustHelpers = `
// A required nullable field must reject absence instead of silently accepting None.
fn required_nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

/// Three wire states for an optional nullable field. Missing is the safe legacy fallback.
#[derive(Clone, Default, PartialEq)]
pub enum OptionalNullable<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}
impl<T> OptionalNullable<T> {
    /// Used only by serde to omit a field that was absent on the wire.
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}
impl<T: Serialize> Serialize for OptionalNullable<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Value(value) => value.serialize(serializer),
            Self::Null | Self::Missing => serializer.serialize_none(),
        }
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for OptionalNullable<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(deserializer).map(|value| value.map_or(Self::Null, Self::Value))
    }
}

/// Fixed, credential-free public protocol failure. It contains no rejected payload or source error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolError;
impl std::fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PUBLIC_CONTRACT_INVALID")
    }
}
impl std::error::Error for ProtocolError {}

fn organization_ceiling<'a>(
    scope: &'a OptionalNullable<Vec<String>>,
    current: &'a str,
) -> Result<Option<Vec<&'a str>>, ProtocolError> {
    match scope {
        OptionalNullable::Missing => Ok(Some(vec![current])),
        OptionalNullable::Null => Ok(None),
        OptionalNullable::Value(ids)
            if ids.len() <= ORGANIZATION_LIMIT
                && ids.iter().all(|id| !id.is_empty())
                && ids.iter().any(|id| id == current) =>
        {
            Ok(Some(ids.iter().map(String::as_str).collect()))
        }
        OptionalNullable::Value(_) => Err(ProtocolError),
    }
}
impl AppAuthorizationContext {
    /// Return the hard organization ceiling; absence restricts an older 1.x context to its current organization.
    /// Explicit null is trusted only after the consumer has verified the platform response and identity binding.
    pub fn organization_ceiling(&self) -> Result<Option<Vec<&str>>, ProtocolError> {
        organization_ceiling(&self.organization_scope_ids, &self.organization.id)
    }
    /// Validate required identifiers and the hard ceiling. The caller must still check expiry, app audience,
    /// introspection binding, permissions and record ownership; this function never authorizes a request.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.app_code.is_empty()
            || self.context_version.is_empty()
            || self.application.id.is_empty()
            || self.user.id.is_empty()
            || self.tenant.id.is_empty()
            || self.organization.id.is_empty()
        {
            return Err(ProtocolError);
        }
        self.organization_ceiling()?;
        if self
            .data_grants
            .iter()
            .any(|grant| grant.organization_ids.len() > ORGANIZATION_LIMIT)
        {
            return Err(ProtocolError);
        }
        Ok(())
    }
    /// Read a bounded public context without logging or retaining parser errors.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > RESPONSE_LIMIT_BYTES {
            return Err(ProtocolError);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ProtocolError)?;
        value.validate()?;
        Ok(value)
    }
}
impl MicroPropsWire {
    /// Validate wire-only bounds; browsers additionally require an unexpired host-owned signal and token lifetime.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !(1..=9007199254740991).contains(&self.context_epoch)
            || self.context_version.is_empty()
            || self.token.is_empty()
            || self.app_code.is_empty()
            || self.base_path != format!("/micro/{}", self.app_code)
        {
            return Err(ProtocolError);
        }
        if let Some(ids) = &self.organization_scope_ids {
            if ids.len() > ORGANIZATION_LIMIT
                || !ids.iter().any(|id| id == &self.organization.id)
                || ids.iter().any(String::is_empty)
            {
                return Err(ProtocolError);
            }
        }
        Ok(())
    }
    /// Parse only a bounded serialized props DTO. This is not an SDK token refresh or an authorization decision.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > RESPONSE_LIMIT_BYTES {
            return Err(ProtocolError);
        }
        let raw: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| ProtocolError)?;
        if raw.get("organizationScopeIds").is_none() {
            return Err(ProtocolError);
        }
        let value: Self = serde_json::from_value(raw).map_err(|_| ProtocolError)?;
        value.validate()?;
        Ok(value)
    }
}

fn notification_text(title: &str, content: &str, link: &str) -> bool {
    !title.trim().is_empty()
        && title.chars().count() <= 200
        && content.chars().count() <= 8192
        && link.chars().count() <= 300
}
fn notification_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128
}
fn publication_pair(application: &Option<String>, publication: &Option<String>) -> bool {
    match (application, publication) {
        (None, None) => true,
        (Some(application), Some(publication)) => {
            notification_id(application) && notification_id(publication)
        }
        _ => false,
    }
}
impl NotificationPublishBody {
    /// Check public input bounds only. The platform separately validates authorization and the link's application owner.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let key = &self.idempotency_key;
        if key.is_empty()
            || key.len() > 128
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
            || !notification_text(
                &self.title,
                self.content.as_deref().unwrap_or(""),
                self.link.as_deref().unwrap_or(""),
            )
        {
            return Err(ProtocolError);
        }
        let target = match &self.target {
            NotificationTarget::User {
                user_id,
                organization_id,
            } => notification_id(user_id) && notification_id(organization_id),
            NotificationTarget::Organization { organization_id } => {
                notification_id(organization_id)
            }
            NotificationTarget::Tenant {} => true,
        };
        if !target {
            return Err(ProtocolError);
        }
        Ok(())
    }
    /// Parse a bounded publish DTO without exposing parser input in its failure.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > RESPONSE_LIMIT_BYTES {
            return Err(ProtocolError);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ProtocolError)?;
        value.validate()?;
        Ok(value)
    }
}
impl RealtimeNotification {
    /// Validate recipient and publication shape; the receiving session still supplies the authoritative identity.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if ![
            &self.id,
            &self.user_id,
            &self.tenant_id,
            &self.organization_id,
        ]
        .into_iter()
        .all(|id| notification_id(id))
            || !notification_text(&self.title, &self.content, &self.link)
            || !publication_pair(&self.application_id, &self.publication_id)
        {
            return Err(ProtocolError);
        }
        Ok(())
    }
}
impl NotificationRecord {
    /// Keep APP publication identity coherent and SYSTEM rows unbound; legacy rows have no current inbox DTO.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let owner = match self.source_kind {
            NotificationSourceKind::App => {
                self.application_id.is_some() && self.publication_id.is_some()
            }
            NotificationSourceKind::System => {
                self.application_id.is_none() && self.publication_id.is_none()
            }
        };
        if !owner
            || ![
                &self.id,
                &self.user_id,
                &self.tenant_id,
                &self.organization_id,
            ]
            .into_iter()
            .all(|id| notification_id(id))
            || !notification_text(&self.title, &self.content, &self.link)
            || !publication_pair(&self.application_id, &self.publication_id)
        {
            return Err(ProtocolError);
        }
        Ok(())
    }
}
`

/** Return deterministic repo-relative generated artifacts without mutating the workspace. */
export function generateContractArtifacts({ root = ownRoot, sdkRoot = ownSdkRoot, release } = {}) {
  const registered = release ?? JSON.parse(readFileSync(path.join(sdkRoot, 'release.json'), 'utf8'))
  const source = JSON.parse(readFileSync(path.join(sdkRoot, 'contracts/schema.json'), 'utf8'))
  const schema = resolveRelease(source, registered)
  const header = '// Generated by examples/micro-app-sdk/scripts/generate-contracts.mjs. Do not edit.\n'
  const types = header + '// Wire data comes from contracts/schema.json and release.json; runtime fields use the explicit generator template.\n' +
    Object.entries(schema.$defs).map(([name, value]) => 'export type ' + name + ' = ' + tsType(value)).join('\n\n') + '\n' + runtimeTypes
  const rust = '//! Generated public wire DTOs. Regenerate with the SDK fixed entry; do not edit.\n//! No platform/database dependency, authentication, token refresh, or shared storage access.\n#![allow(dead_code)]\n\nuse serde::{Deserialize, Serialize};\n' +
    '/// The only registered SDK release; wire contract majors remain independently versioned.\npub const SDK_VERSION: &str = ' + JSON.stringify(registered.sdkVersion) + ';\n' +
    '/// Context wire contract supported by this release.\npub const CONTEXT_VERSION: &str = ' + JSON.stringify(registered.contracts.context) + ';\n' +
    '/// Props wire contract supported by this release.\npub const MICRO_PROPS_VERSION: &str = ' + JSON.stringify(registered.contracts.microProps) + ';\n' +
    '/// Iframe channel contract supported by this release.\npub const IFRAME_VERSION: &str = ' + JSON.stringify(registered.contracts.iframe) + ';\n' +
    '/// Notification wire contract supported by this release.\npub const NOTIFICATION_VERSION: &str = ' + JSON.stringify(registered.contracts.notification) + ';\n' +
    '/// Maximum bounded public JSON response bytes.\npub const RESPONSE_LIMIT_BYTES: usize = ' + registered.limits.maxJsonResponseBytes + ';\n' +
    '/// Maximum explicitly listed organizations; never silently truncated.\npub const ORGANIZATION_LIMIT: usize = ' + registered.limits.maxOrganizationScopeIds + ';\n\n' +
    Object.entries(schema.$defs).map(([name, value]) => rustDefinition(name, value, source.$defs[name])).join('\n') + rustHelpers
  const relative = slash(path.relative(root, sdkRoot))
  if (relative.startsWith('../') || path.isAbsolute(relative)) throw new Error('SDK must be inside repository root')
  return new Map([
    [relative + '/generated/schema.json', JSON.stringify(schema, null, 2) + '\n'],
    [relative + '/generated/types.d.ts', types],
    [relative + '/generated/protocol.rs', rust],
    ['backend/src/public_contract.rs', rust],
  ])
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2)
  if (args.length !== 1 || !['--check', '--write'].includes(args[0])) throw new Error('Usage: node examples/micro-app-sdk/scripts/generate-contracts.mjs --check|--write')
  let drift = false
  for (const [relative, content] of generateContractArtifacts()) {
    const file = path.join(ownRoot, relative)
    if (args[0] === '--write') { mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, content) }
    else { try { if (readFileSync(file, 'utf8') !== content) { console.error('Contract artifact drift: ' + relative); drift = true } } catch { console.error('Missing contract artifact: ' + relative); drift = true } }
  }
  if (drift) process.exitCode = 1
}
