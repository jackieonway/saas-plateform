//! Private probes and local transition events. No external notification channel
//! is configured, and a transport send is never called user delivery.
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Fixed component vocabulary: resource identities can never become labels.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    Platform,
    WorkOrders,
    Stores,
    Notification,
    BackupAge,
    BackupRun,
    BackupVerify,
}
impl Component {
    fn name(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::WorkOrders => "work_orders",
            Self::Stores => "stores",
            Self::Notification => "notification",
            Self::BackupAge => "backup_age",
            Self::BackupRun => "backup_run",
            Self::BackupVerify => "backup_verify",
        }
    }
}
/// Stable health state; unknown does not imply healthy.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Health {
    Unknown,
    Up,
    Down,
}
/// Event data has no user, raw URL, remote message or arbitrary label fields.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct Transition {
    /// Closed component identity.
    pub component: Component,
    /// DOWN or RECOVERED after consecutive samples.
    pub event: &'static str,
}
/// A bounded consecutive-sample state machine. First healthy baseline is quiet.
pub struct Machine {
    health: Health,
    candidate: Option<bool>,
    count: u8,
    failures: u8,
    recoveries: u8,
}
impl Machine {
    /// Require 1–20 consecutive failures/successes. Invalid policy is rejected.
    pub fn new(failures: u8, recoveries: u8) -> Result<Self, &'static str> {
        if !(1..=20).contains(&failures) || !(1..=20).contains(&recoveries) {
            return Err("INVALID_POLICY");
        }
        Ok(Self {
            health: Health::Unknown,
            candidate: None,
            count: 0,
            failures,
            recoveries,
        })
    }
    /// Apply one ordered observation. Repeated stable states emit no events.
    pub fn observe(&mut self, component: Component, healthy: bool) -> Option<Transition> {
        if self.candidate == Some(healthy) {
            self.count = self.count.saturating_add(1)
        } else {
            self.candidate = Some(healthy);
            self.count = 1
        }
        let required = if healthy {
            self.recoveries
        } else {
            self.failures
        };
        if self.count < required {
            return None;
        }
        let next = if healthy { Health::Up } else { Health::Down };
        let before = self.health;
        self.health = next;
        if before == next || before == Health::Unknown && healthy {
            return None;
        }
        Some(Transition {
            component,
            event: if healthy { "RECOVERED" } else { "DOWN" },
        })
    }
    /// Current stable state, independent of transient candidate samples.
    pub fn health(&self) -> Health {
        self.health
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    component: Component,
    url: String,
}
/// Explicit private configuration. Debug/Display are deliberately absent.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    bind: SocketAddr,
    interval_ms: u64,
    timeout_ms: u64,
    failures: u8,
    recoveries: u8,
    metrics_token: String,
    event_token: String,
    targets: Vec<Probe>,
}
impl Config {
    /// Parse a bounded closed config, validating all targets without logging values.
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > 16384 {
            return Err("CONFIG_TOO_LARGE");
        }
        let config: Self = serde_json::from_slice(bytes).map_err(|_| "INVALID_CONFIG")?;
        if !(100..=60000).contains(&config.interval_ms)
            || !(50..=10000).contains(&config.timeout_ms)
            || config.targets.is_empty()
            || config.targets.len() > 4
        {
            return Err("INVALID_BUDGET");
        }
        Machine::new(config.failures, config.recoveries)?;
        for token in [&config.metrics_token, &config.event_token] {
            if token.len() < 16
                || token.len() > 1024
                || !token.bytes().all(|byte| byte.is_ascii_graphic())
            {
                return Err("INVALID_CREDENTIAL");
            }
        }
        if config.metrics_token == config.event_token {
            return Err("CREDENTIALS_MUST_DIFFER");
        }
        let mut seen = std::collections::BTreeSet::new();
        for target in &config.targets {
            if !seen.insert(target.component) {
                return Err("DUPLICATE_TARGET");
            }
            let expected = match target.component {
                Component::Platform => "/api/health",
                Component::WorkOrders => "/work-orders-api/health/ready",
                Component::Stores => "/stores-api/health/ready",
                Component::Notification => "/health/ready",
                _ => return Err("INVALID_TARGET"),
            };
            let url = reqwest::Url::parse(&target.url).map_err(|_| "INVALID_TARGET")?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != expected
            {
                return Err("INVALID_TARGET");
            }
        }
        Ok(config)
    }
}
struct Slot {
    machine: Machine,
    sequence: u64,
    down: u64,
    recovered: u64,
}
struct Shared {
    config: Config,
    slots: Mutex<BTreeMap<Component, Slot>>,
    client: reqwest::Client,
}
impl Shared {
    fn observe(
        &self,
        component: Component,
        healthy: bool,
        sequence: Option<u64>,
    ) -> Result<Option<Transition>, StatusCode> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let slot = slots.get_mut(&component).ok_or(StatusCode::BAD_REQUEST)?;
        if let Some(sequence) = sequence {
            if sequence <= slot.sequence {
                return Err(StatusCode::CONFLICT);
            }
            slot.sequence = sequence;
        }
        let transition = slot.machine.observe(component, healthy);
        if let Some(event) = transition {
            if event.event == "DOWN" {
                slot.down = slot.down.saturating_add(1)
            } else {
                slot.recovered = slot.recovered.saturating_add(1)
            }
            println!(
                "{}",
                serde_json::to_string(&event).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            );
        }
        Ok(transition)
    }
    async fn poll(&self) {
        for target in &self.config.targets {
            let healthy = probe(&self.client, &target.url).await;
            let _ = self.observe(target.component, healthy, None);
        }
    }
}
async fn probe(client: &reqwest::Client, url: &str) -> bool {
    let Ok(mut response) = client.get(url).send().await else {
        return false;
    };
    if response.status() != StatusCode::OK {
        return false;
    }
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len() + chunk.len() > 65536 {
                    return false;
                }
                bytes.extend_from_slice(&chunk)
            }
            Ok(None) => break,
            Err(_) => return false,
        }
    }
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .is_ok_and(|body| body["status"] == "UP" || body["status"] == "ok")
}
fn authenticated(headers: &HeaderMap, expected: &str) -> bool {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let candidate = values
        .next()
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    values.next().is_none()
        && candidate.is_some_and(|value| {
            value.len() == expected.len()
                && value
                    .bytes()
                    .zip(expected.bytes())
                    .fold(0u8, |d, (a, b)| d | (a ^ b))
                    == 0
        })
}
async fn metrics(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authenticated(&headers, &shared.config.metrics_token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(slots) = shared.slots.lock() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let mut text=String::from("# TYPE saas_monitor_component_state gauge\n# TYPE saas_monitor_transitions_total counter\n");
    for (component, slot) in slots.iter() {
        for (name, state) in [
            ("unknown", Health::Unknown),
            ("up", Health::Up),
            ("down", Health::Down),
        ] {
            text.push_str(&format!(
                "saas_monitor_component_state{{component=\"{}\",state=\"{name}\"}} {}\n",
                component.name(),
                u8::from(slot.machine.health() == state)
            ));
        }
        for (event, count) in [("DOWN", slot.down), ("RECOVERED", slot.recovered)] {
            text.push_str(&format!(
                "saas_monitor_transitions_total{{component=\"{}\",event=\"{event}\"}} {count}\n",
                component.name()
            ));
        }
    }
    (
        [
            (header::CONTENT_TYPE, "text/plain; version=0.0.4"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        text,
    )
        .into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    component: Component,
    healthy: bool,
    sequence: u64,
}
async fn event(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(event): Json<Event>,
) -> Response {
    if !authenticated(&headers, &shared.config.event_token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !matches!(
        event.component,
        Component::BackupAge | Component::BackupRun | Component::BackupVerify
    ) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match shared.observe(event.component, event.healthy, Some(event.sequence)) {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(status) => status,
    }
    .into_response()
}
fn shared(config: Config) -> Result<Arc<Shared>, &'static str> {
    let slots = [
        Component::Platform,
        Component::WorkOrders,
        Component::Stores,
        Component::Notification,
        Component::BackupAge,
        Component::BackupRun,
        Component::BackupVerify,
    ]
    .into_iter()
    .map(|component| {
        Ok((
            component,
            Slot {
                machine: Machine::new(config.failures, config.recoveries)?,
                sequence: 0,
                down: 0,
                recovered: 0,
            },
        ))
    })
    .collect::<Result<_, &str>>()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(config.timeout_ms))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| "CLIENT_FAILED")?;
    Ok(Arc::new(Shared {
        config,
        slots: Mutex::new(slots),
        client,
    }))
}
/// Run private HTTP and periodic probes until shutdown. External delivery is absent.
pub async fn run(config: Config) -> Result<(), &'static str> {
    let state = shared(config)?;
    let listener = tokio::net::TcpListener::bind(state.config.bind)
        .await
        .map_err(|_| "BIND_FAILED")?;
    let worker = state.clone();
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(worker.config.interval_ms));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            worker.poll().await;
        }
    });
    let result = axum::serve(
        listener,
        Router::new()
            .route("/internal/metrics", get(metrics))
            .route("/internal/events", post(event))
            .layer(axum::extract::DefaultBodyLimit::max(1024))
            .with_state(state),
    )
    .with_graceful_shutdown(async {
        #[cfg(unix)]
        {
            if let Ok(mut signal) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            {
                tokio::select! {_=tokio::signal::ctrl_c()=>{},_=signal.recv()=>{}}
            } else {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    })
    .await;
    task.abort();
    let _ = task.await;
    result.map_err(|_| "SERVER_FAILED")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transitions_are_stable_and_initial_up_is_quiet() {
        let mut m = Machine::new(2, 2).unwrap();
        assert!(m.observe(Component::Platform, true).is_none());
        assert!(m.observe(Component::Platform, true).is_none());
        assert_eq!(m.health(), Health::Up);
        assert!(m.observe(Component::Platform, false).is_none());
        assert_eq!(m.observe(Component::Platform, false).unwrap().event, "DOWN");
        assert!(m.observe(Component::Platform, false).is_none());
        assert!(m.observe(Component::Platform, true).is_none());
        assert_eq!(
            m.observe(Component::Platform, true).unwrap().event,
            "RECOVERED"
        );
        assert!(m.observe(Component::Platform, true).is_none());
    }
    #[test]
    fn flapping_does_not_emit_and_unknown_is_not_up() {
        let mut m = Machine::new(2, 2).unwrap();
        for i in 0..20 {
            assert!(m.observe(Component::Stores, i % 2 == 0).is_none());
        }
        assert_eq!(m.health(), Health::Unknown);
        assert!(Machine::new(0, 2).is_err());
    }
    #[test]
    fn config_rejects_credentials_in_targets() {
        let bytes=br#"{"bind":"127.0.0.1:0","interval_ms":100,"timeout_ms":50,"failures":2,"recoveries":2,"metrics_token":"private-metrics-canary","event_token":"private-events-canary","targets":[{"component":"platform","url":"http://CANARY:SECRET@localhost/api/health"}]}"#;
        assert_eq!(Config::parse(bytes).err(), Some("INVALID_TARGET"));
    }
    #[test]
    fn credentials_must_be_usable_in_authorization_headers() {
        let example: serde_json::Value =
            serde_json::from_slice(include_bytes!("../config.example.json")).unwrap();
        for field in ["metrics_token", "event_token"] {
            for invalid in ["密", "\0", "\x01", "\x7f", " ", "\t", "\r", "\n"] {
                let mut value = example.clone();
                value[field] = format!("private-token-prefix-{invalid}").into();
                assert_eq!(
                    Config::parse(value.to_string().as_bytes()).err(),
                    Some("INVALID_CREDENTIAL"),
                    "{field} must reject non-visible ASCII credentials"
                );
            }
            let mut value = example.clone();
            let token = "private-token-._~+/=";
            value[field] = token.into();
            assert!(Config::parse(value.to_string().as_bytes()).is_ok());
            let mut headers = HeaderMap::new();
            headers.insert(
                header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
            assert!(authenticated(&headers, token));
        }
    }
    #[test]
    fn example_is_valid_and_credentials_and_target_vocabulary_are_closed() {
        let example = include_bytes!("../config.example.json");
        assert!(Config::parse(example).is_ok());
        let mut value: serde_json::Value = serde_json::from_slice(example).unwrap();
        value["event_token"] = value["metrics_token"].clone();
        assert_eq!(
            Config::parse(value.to_string().as_bytes()).err(),
            Some("CREDENTIALS_MUST_DIFFER")
        );
        value["event_token"] = "different-private-event-token".into();
        value["targets"][0]["component"] = "arbitrary-user-or-url".into();
        assert_eq!(
            Config::parse(value.to_string().as_bytes()).err(),
            Some("INVALID_CONFIG")
        );
        assert_eq!(
            Config::parse(&vec![b' '; 16385]).err(),
            Some("CONFIG_TOO_LARGE")
        );
    }
    #[tokio::test]
    async fn real_http_failures_recover_and_events_reject_stale() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let up = Arc::new(AtomicBool::new(true));
        let current = up.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/api/health",
                    get(move || {
                        let up = current.load(Ordering::Relaxed);
                        async move {
                            (
                                if up {
                                    StatusCode::OK
                                } else {
                                    StatusCode::SERVICE_UNAVAILABLE
                                },
                                Json(serde_json::json!({"status":if up{"UP"}else{"DOWN"}})),
                            )
                        }
                    }),
                ),
            )
            .await
            .unwrap()
        });
        let config=Config::parse(serde_json::json!({"bind":"127.0.0.1:0","interval_ms":100,"timeout_ms":100,"failures":2,"recoveries":2,"metrics_token":"private-metrics-canary","event_token":"private-events-canary","targets":[{"component":"platform","url":origin+"/api/health"}]}).to_string().as_bytes()).unwrap();
        let shared = shared(config).unwrap();
        shared.poll().await;
        shared.poll().await;
        up.store(false, Ordering::Relaxed);
        shared.poll().await;
        shared.poll().await;
        shared.poll().await;
        up.store(true, Ordering::Relaxed);
        shared.poll().await;
        shared.poll().await;
        {
            let slots = shared.slots.lock().unwrap();
            assert_eq!(slots[&Component::Platform].down, 1);
            assert_eq!(slots[&Component::Platform].recovered, 1);
        }
        assert!(shared
            .observe(Component::BackupRun, false, Some(1))
            .unwrap()
            .is_none());
        assert!(shared
            .observe(Component::BackupRun, false, Some(2))
            .unwrap()
            .is_some());
        assert_eq!(
            shared.observe(Component::BackupRun, true, Some(1)).err(),
            Some(StatusCode::CONFLICT)
        );
        task.abort();
        let _ = task.await;
    }
}
