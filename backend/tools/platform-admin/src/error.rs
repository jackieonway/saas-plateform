use serde_json::{Value, json};
use std::{error::Error, fmt};

/// Credential-safe failure. It never retains supplied values, URLs, bodies or
/// underlying transport/parser error strings.
#[derive(Debug)]
pub struct SafeError {
    category: &'static str,
    status: Option<u16>,
    outcome_unknown: bool,
    progress: Option<Value>,
}
impl SafeError {
    pub(crate) fn new(category: &'static str) -> Self {
        Self {
            category,
            status: None,
            outcome_unknown: false,
            progress: None,
        }
    }
    pub(crate) fn http(category: &'static str, status: Option<u16>, writing: bool) -> Self {
        Self {
            category,
            status,
            outcome_unknown: writing,
            progress: None,
        }
    }
    pub(crate) fn with_completed(mut self, details: Value) -> Self {
        self.progress = Some(details);
        self
    }
    pub(crate) fn uncertain(mut self) -> Self {
        self.outcome_unknown = true;
        self
    }
    /// Fixed category suitable for automation; no external text is included.
    pub fn category(&self) -> &'static str {
        self.category
    }
    /// HTTP status, when response headers were received.
    pub fn status(&self) -> Option<u16> {
        self.status
    }
    /// Whether the operation may have committed and requires reconciliation.
    pub fn outcome_unknown(&self) -> bool {
        self.outcome_unknown
    }
    /// Redacted machine-readable diagnostic, including mandatory write recovery.
    pub fn to_json(&self) -> Value {
        let mut result = json!({"ok":false,"category":self.category});
        if let Some(status) = self.status {
            result["status"] = json!(status);
        }
        if self.outcome_unknown {
            result["outcomeUnknown"] = json!(true);
            result["reconciliationRequired"] = json!(true);
            result["nextAction"] = json!("INSPECT_AND_RECONCILE_BEFORE_RETRY");
        }
        if let Some(details) = &self.progress {
            result["progress"] = details.clone();
        }
        result
    }
}
impl fmt::Display for SafeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.category)
    }
}
impl Error for SafeError {}
