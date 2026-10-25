use crate::{
    SafeError,
    contract::{self, KnownApp},
};
use std::{collections::BTreeMap, path::Path};

pub(crate) const USAGE: &str = "saas-platform-admin [plan|inspect|bootstrap|access-policy-read|access-policy-save|register|releases|release-reconcile|artifact-bind|artifact-verify|retention-preview|preflight|activate|deployments|rollback|grants-plan|grants-apply] [options]\nDefault: offline plan. Writes require --apply.\n--app-code work-orders|stores (default work-orders; grant manifest selects its app), --environment NAME (production), --app-id UUID, --timeout-ms 10..35000\naccess-policy-read --policy-target application|client --policy-id UUID (remote GET only)\naccess-policy-save --manifest FILE (offline preview; --apply saves the explicit expected accessPolicyVersion)\nregister/release-reconcile --release-file FILE [--artifact-file ABS --inventory-dir ABS]; legacy register without the pair does not verify digest.\nartifact-bind/artifact-verify --release-file ABS --artifact-file ABS --inventory-dir ABS (offline; bind writes require --apply)\nretention-preview [--minimum-age-days 30..3650] [--keep-latest 5..1000] (remote GET only; apply unavailable)\npreflight/activate --release-id UUID\ngrants-plan/grants-apply --manifest FILE; explicit --app-code must match applicationCode\nDevelopment HTTP: --environment development --allow-loopback-http\nRemote environment: PLATFORM_API_ORIGIN, PLATFORM_ADMIN_BEARER; inspect/bootstrap also PLATFORM_CLIENT_ID.\nNo client secret, client creation, SQL execution or ZIP installation. Artifact bindings are local declarations, not platform digest metadata or Docker readiness.";

pub(crate) struct Options {
    pub(crate) command: String,
    pub(crate) environment: String,
    pub(crate) apply: bool,
    pub(crate) allow_loopback_http: bool,
    pub(crate) app_id: Option<String>,
    pub(crate) app_code: Option<KnownApp>,
    pub(crate) release_id: Option<String>,
    pub(crate) release_file: Option<String>,
    pub(crate) artifact_file: Option<String>,
    pub(crate) inventory_dir: Option<String>,
    pub(crate) manifest: Option<String>,
    pub(crate) policy_target: Option<String>,
    pub(crate) policy_id: Option<String>,
    pub(crate) timeout_ms: u64,
    pub(crate) minimum_age_days: u32,
    pub(crate) keep_latest: usize,
}
impl Options {
    pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, SafeError> {
        let args: Vec<_> = args.into_iter().collect();
        if args == ["--help"] {
            return Ok(Self {
                command: "help".into(),
                environment: "production".into(),
                apply: false,
                allow_loopback_http: false,
                app_id: None,
                app_code: None,
                release_id: None,
                release_file: None,
                artifact_file: None,
                inventory_dir: None,
                manifest: None,
                policy_target: None,
                policy_id: None,
                timeout_ms: 35000,
                minimum_age_days: 30,
                keep_latest: 5,
            });
        }
        let explicit = args.first().is_some_and(|arg| !arg.starts_with("--"));
        let command = if explicit { args[0].as_str() } else { "plan" };
        if !matches!(
            command,
            "plan"
                | "inspect"
                | "bootstrap"
                | "access-policy-read"
                | "access-policy-save"
                | "register"
                | "artifact-bind"
                | "artifact-verify"
                | "retention-preview"
                | "release-reconcile"
                | "releases"
                | "preflight"
                | "activate"
                | "deployments"
                | "rollback"
                | "grants-plan"
                | "grants-apply"
        ) {
            return Err(SafeError::new("INVALID_ARGUMENT"));
        }
        let mut values = BTreeMap::new();
        let mut index = usize::from(explicit);
        while index < args.len() {
            let key = args[index].as_str();
            if !matches!(
                key,
                "--environment"
                    | "--app-code"
                    | "--app-id"
                    | "--release-id"
                    | "--release-file"
                    | "--artifact-file"
                    | "--inventory-dir"
                    | "--manifest"
                    | "--policy-target"
                    | "--policy-id"
                    | "--apply"
                    | "--allow-loopback-http"
                    | "--timeout-ms"
                    | "--minimum-age-days"
                    | "--keep-latest"
            ) || values.contains_key(key)
            {
                return Err(SafeError::new("INVALID_ARGUMENT"));
            }
            let value = if matches!(key, "--apply" | "--allow-loopback-http") {
                "true".to_owned()
            } else {
                index += 1;
                args.get(index)
                    .filter(|value| !value.is_empty() && !value.starts_with("--"))
                    .cloned()
                    .ok_or_else(|| SafeError::new("INVALID_ARGUMENT"))?
            };
            values.insert(key.to_owned(), value);
            index += 1;
        }
        let result = Self {
            command: command.into(),
            environment: values
                .remove("--environment")
                .unwrap_or_else(|| "production".into()),
            apply: values.remove("--apply").is_some(),
            allow_loopback_http: values.remove("--allow-loopback-http").is_some(),
            app_id: values.remove("--app-id"),
            app_code: values
                .remove("--app-code")
                .map(|code| {
                    KnownApp::parse(&code).ok_or_else(|| SafeError::new("INVALID_APPLICATION_CODE"))
                })
                .transpose()?,
            release_id: values.remove("--release-id"),
            release_file: values.remove("--release-file"),
            artifact_file: values.remove("--artifact-file"),
            inventory_dir: values.remove("--inventory-dir"),
            manifest: values.remove("--manifest"),
            policy_target: values.remove("--policy-target"),
            policy_id: values.remove("--policy-id"),
            minimum_age_days: values
                .remove("--minimum-age-days")
                .map(|v| v.parse().map_err(|_| SafeError::new("INVALID_ARGUMENT")))
                .transpose()?
                .unwrap_or(30),
            keep_latest: values
                .remove("--keep-latest")
                .map(|v| v.parse().map_err(|_| SafeError::new("INVALID_ARGUMENT")))
                .transpose()?
                .unwrap_or(5),
            timeout_ms: values
                .remove("--timeout-ms")
                .map(|value| {
                    value
                        .parse::<u64>()
                        .map_err(|_| SafeError::new("INVALID_ARGUMENT"))
                })
                .transpose()?
                .unwrap_or(35000),
        };
        if !(30..=3650).contains(&result.minimum_age_days)
            || !(5..=1000).contains(&result.keep_latest)
            || command != "retention-preview"
                && args
                    .iter()
                    .any(|v| v == "--minimum-age-days" || v == "--keep-latest")
        {
            return Err(SafeError::new("INVALID_ARGUMENT"));
        }
        if !contract::is_environment(&result.environment) {
            return Err(SafeError::new("INVALID_ENVIRONMENT"));
        }
        if result
            .app_id
            .as_ref()
            .is_some_and(|id| !contract::is_uuid(id))
            || result
                .release_id
                .as_ref()
                .is_some_and(|id| !contract::is_uuid(id))
            || result
                .policy_id
                .as_ref()
                .is_some_and(|id| !contract::is_uuid(id))
        {
            return Err(SafeError::new("INVALID_ID"));
        }
        if !(10..=35000).contains(&result.timeout_ms)
            || result.apply
                && matches!(
                    command,
                    "plan"
                        | "inspect"
                        | "access-policy-read"
                        | "retention-preview"
                        | "releases"
                        | "deployments"
                        | "grants-plan"
                        | "artifact-verify"
                        | "release-reconcile"
                )
            || result.release_id.is_some() != matches!(command, "preflight" | "activate")
            || result.release_file.is_some()
                != matches!(
                    command,
                    "register" | "release-reconcile" | "artifact-bind" | "artifact-verify"
                )
            || result.artifact_file.is_some() != result.inventory_dir.is_some()
            || result.artifact_file.is_some()
                && !matches!(
                    command,
                    "register" | "release-reconcile" | "artifact-bind" | "artifact-verify"
                )
            || matches!(command, "artifact-bind" | "artifact-verify")
                && (result.artifact_file.is_none() || result.app_id.is_some())
            || result.manifest.is_some()
                != matches!(
                    command,
                    "grants-plan" | "grants-apply" | "access-policy-save"
                )
            || result.policy_id.is_some() != (command == "access-policy-read")
            || result.policy_target.is_some() != (command == "access-policy-read")
            || result
                .policy_target
                .as_deref()
                .is_some_and(|target| !matches!(target, "application" | "client"))
            || matches!(command, "access-policy-read" | "access-policy-save")
                && (result.app_id.is_some() || result.app_code.is_some())
            || result.allow_loopback_http && result.environment != "development"
        {
            return Err(SafeError::new("INVALID_ARGUMENT"));
        }
        Ok(result)
    }
    pub(crate) fn is_plan(&self) -> bool {
        self.command == "plan"
            || !self.apply
                && !matches!(
                    self.command.as_str(),
                    "inspect"
                        | "access-policy-read"
                        | "retention-preview"
                        | "releases"
                        | "deployments"
                        | "release-reconcile"
                        | "artifact-verify"
                )
    }
    pub(crate) fn app(&self) -> KnownApp {
        self.app_code.unwrap_or(KnownApp::WorkOrders)
    }
    pub(crate) fn manifest_path(&self) -> Result<&Path, SafeError> {
        self.manifest
            .as_deref()
            .map(Path::new)
            .ok_or_else(|| SafeError::new("INVALID_ARGUMENT"))
    }
}
