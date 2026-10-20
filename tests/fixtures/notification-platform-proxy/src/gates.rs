use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::watch;

const MAX_GATES: usize = 8;
const MAX_TOKEN_BYTES: usize = 4096;
pub(crate) const DEADLINE: Duration = Duration::from_secs(15);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GateInput {
    label: String,
    token: String,
    delivery_only: bool,
    delay_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseInput {
    label: String,
}

#[derive(Default)]
pub(crate) struct Gates(Mutex<BTreeMap<String, Arc<Gate>>>);

struct Gate {
    token: String,
    delivery_only: bool,
    delay: Duration,
    released: watch::Sender<bool>,
    inflight: AtomicU64,
    highwater: AtomicU64,
    count: AtomicU64,
}

pub(crate) struct GateRequest(Arc<Gate>);

impl Gates {
    pub(crate) fn insert(&self, input: GateInput) -> Result<(), &'static str> {
        if !valid_label(&input.label)
            || !valid_token(&input.token)
            || input.label == input.token
            || input.delay_ms > DEADLINE.as_millis() as u64
        {
            return Err("invalid gate bounds");
        }
        let mut gates = self.0.lock().map_err(|_| "gate state unavailable")?;
        if gates
            .get(&input.label)
            .is_some_and(|gate| gate.inflight.load(Ordering::Relaxed) > 0)
        {
            return Err("gate is in use");
        }
        if !gates.contains_key(&input.label) && gates.len() >= MAX_GATES {
            return Err("gate capacity exceeded");
        }
        if gates.iter().any(|(label, gate)| {
            label != &input.label && !*gate.released.borrow() && gate.token == input.token
        }) {
            return Err("token already has an active gate");
        }
        let (released, _) = watch::channel(false);
        gates.insert(
            input.label,
            Arc::new(Gate {
                token: input.token,
                delivery_only: input.delivery_only,
                delay: Duration::from_millis(input.delay_ms),
                released,
                inflight: AtomicU64::new(0),
                highwater: AtomicU64::new(0),
                count: AtomicU64::new(0),
            }),
        );
        Ok(())
    }

    pub(crate) fn release(&self, input: ReleaseInput) -> Result<(), &'static str> {
        if !valid_label(&input.label) {
            return Err("invalid label");
        }
        let gates = self.0.lock().map_err(|_| "gate state unavailable")?;
        let gate = gates.get(&input.label).ok_or("gate not found")?;
        gate.released.send_replace(true);
        Ok(())
    }

    pub(crate) fn enter(
        &self,
        token: &str,
        delivery: bool,
    ) -> Result<Option<GateRequest>, &'static str> {
        let gates = self.0.lock().map_err(|_| "gate state unavailable")?;
        let gate = gates.values().find(|gate| {
            !*gate.released.borrow() && gate.token == token && (delivery || !gate.delivery_only)
        });
        Ok(gate.map(|gate| {
            let inflight = gate.inflight.fetch_add(1, Ordering::Relaxed) + 1;
            gate.highwater.fetch_max(inflight, Ordering::Relaxed);
            gate.count.fetch_add(1, Ordering::Relaxed);
            GateRequest(Arc::clone(gate))
        }))
    }

    pub(crate) fn snapshot(&self) -> Result<Value, &'static str> {
        let gates = self.0.lock().map_err(|_| "gate state unavailable")?;
        Ok(json!({"gates": gates.iter().map(|(label, gate)| json!({
            "label": label,
            "inflight": gate.inflight.load(Ordering::Relaxed),
            "highwater": gate.highwater.load(Ordering::Relaxed),
            "count": gate.count.load(Ordering::Relaxed),
        })).collect::<Vec<_>>()}))
    }
}

impl GateRequest {
    pub(crate) async fn delay(&self) {
        let mut release = self.0.released.subscribe();
        if *release.borrow() {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(self.0.delay) => {},
            _ = release.changed() => {},
        }
    }
}

impl Drop for GateRequest {
    fn drop(&mut self) {
        self.0.inflight.fetch_sub(1, Ordering::Relaxed);
    }
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 48
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub(crate) fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= MAX_TOKEN_BYTES
        && token.bytes().all(|byte| byte.is_ascii_graphic())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(label: &str, token: &str) -> GateInput {
        GateInput {
            label: label.into(),
            token: token.into(),
            delivery_only: true,
            delay_ms: 15_000,
        }
    }

    #[tokio::test]
    async fn gate_only_delays_delivery_and_releases_without_losing_counters() {
        let gates = Gates::default();
        gates.insert(input("slow", "secret-bearer")).unwrap();
        assert!(gates.enter("secret-bearer", false).unwrap().is_none());
        assert!(gates.enter("different", true).unwrap().is_none());
        let request = gates.enter("secret-bearer", true).unwrap().unwrap();
        assert!(gates.insert(input("slow", "replacement")).is_err());
        gates
            .release(ReleaseInput {
                label: "slow".into(),
            })
            .unwrap();
        tokio::time::timeout(Duration::from_millis(100), request.delay())
            .await
            .unwrap();
        assert_eq!(gates.snapshot().unwrap()["gates"][0]["inflight"], 1);
        drop(request);
        let snapshot = gates.snapshot().unwrap();
        assert_eq!(snapshot["gates"][0]["inflight"], 0);
        assert_eq!(snapshot["gates"][0]["highwater"], 1);
        assert_eq!(snapshot["gates"][0]["count"], 1);
        assert!(!snapshot.to_string().contains("secret-bearer"));
        assert!(gates.enter("secret-bearer", true).unwrap().is_none());
    }

    #[test]
    fn all_context_gate_tracks_identity_and_delivery_for_only_its_token() {
        let gates = Gates::default();
        let mut all_context = input("auth-race", "real-platform-session");
        all_context.delivery_only = false;
        gates.insert(all_context).unwrap();
        let identity = gates
            .enter("real-platform-session", false)
            .unwrap()
            .unwrap();
        let delivery = gates.enter("real-platform-session", true).unwrap().unwrap();
        assert!(gates
            .enter("other-platform-session", false)
            .unwrap()
            .is_none());
        assert!(gates
            .enter("other-platform-session", true)
            .unwrap()
            .is_none());
        let snapshot = gates.snapshot().unwrap();
        assert_eq!(snapshot["gates"][0]["inflight"], 2);
        assert_eq!(snapshot["gates"][0]["highwater"], 2);
        assert_eq!(snapshot["gates"][0]["count"], 2);
        assert!(!snapshot.to_string().contains("real-platform-session"));
        drop((identity, delivery));
        assert_eq!(gates.snapshot().unwrap()["gates"][0]["inflight"], 0);
    }

    #[test]
    fn gate_memory_and_input_bounds_are_enforced() {
        let gates = Gates::default();
        for number in 0..MAX_GATES {
            gates
                .insert(input(&format!("gate{number}"), &format!("token{number}")))
                .unwrap();
        }
        assert!(gates.insert(input("ninth", "token-nine")).is_err());
        assert!(gates.insert(input("gate0", "token0")).is_ok());
        for label in ["", "bad label", "../x", &"x".repeat(49)] {
            assert!(!valid_label(label));
        }
        assert!(valid_token(&"x".repeat(4096)));
        assert!(!valid_token(&"x".repeat(4097)));
        assert!(!valid_token("bad token"));
        let mut invalid = input("gate0", "other");
        invalid.delay_ms = 15001;
        assert!(gates.insert(invalid).is_err());
    }
}
