use crate::Config;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Flow {
    AuthorizationCode,
    Device,
    ClientCredentials,
    Password,
}
impl Flow {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::AuthorizationCode => 0,
            Self::Device => 1,
            Self::ClientCredentials => 2,
            Self::Password => 3,
        }
    }
}

pub(crate) struct Auth {
    pub(crate) state: String,
    pub(crate) verifier: String,
    pub(crate) expires: Instant,
}
pub(crate) struct Device {
    pub(crate) code: String,
    pub(crate) expires: Instant,
    pub(crate) next_poll: Instant,
    pub(crate) interval: Duration,
}
#[derive(Clone)]
pub(crate) struct Token {
    pub(crate) access: String,
    pub(crate) refresh: Option<String>,
    pub(crate) token_type: String,
    pub(crate) scope: String,
    pub(crate) expires: Instant,
    pub(crate) retain_until: Instant,
    pub(crate) expires_in: u64,
}
impl Token {
    pub(crate) fn summary(&self, flow: Flow) -> serde_json::Value {
        serde_json::json!({"granted":true,"tokenType":self.token_type,"expiresIn":self.expires_in,
            "scope":self.scope,"hasRefreshToken":self.refresh.is_some(),"flow":flow})
    }
}
pub(crate) struct Session {
    pub(crate) expires: Instant,
    pub(crate) auth: Option<Auth>,
    pub(crate) device: Option<Device>,
    pub(crate) tokens: HashMap<Flow, Token>,
    // Ownership prevents one canceled request from clearing another operation.
    pub(crate) busy: HashMap<Flow, String>,
}
#[derive(Default)]
pub(crate) struct Sessions {
    pub(crate) entries: HashMap<String, Session>,
}
impl Sessions {
    pub(crate) fn cleanup(&mut self, now: Instant) {
        self.entries.retain(|_, session| {
            if session.expires <= now {
                return false;
            }
            if session.auth.as_ref().is_some_and(|a| a.expires <= now) {
                session.auth = None;
            }
            if session.device.as_ref().is_some_and(|d| d.expires <= now) {
                session.device = None;
            }
            session.tokens.retain(|_, token| token.retain_until > now);
            true
        });
    }
    pub(crate) fn create(&mut self, config: &Config, now: Instant) -> Option<String> {
        self.cleanup(now);
        if self.entries.len() >= config.max_sessions {
            return None;
        }
        let id = random();
        self.entries.insert(
            id.clone(),
            Session {
                expires: now + config.session_ttl,
                auth: None,
                device: None,
                tokens: HashMap::new(),
                busy: HashMap::new(),
            },
        );
        Some(id)
    }
    pub(crate) fn start_auth(
        &mut self,
        id: &str,
        config: &Config,
        now: Instant,
    ) -> Option<(String, String)> {
        self.cleanup(now);
        let capacity = self.entries.values().filter(|s| s.auth.is_some()).count();
        let session = self.entries.get_mut(id)?;
        if capacity >= config.max_states && session.auth.is_none() {
            return None;
        }
        let state = random();
        let verifier = random_verifier();
        session.auth = Some(Auth {
            state: state.clone(),
            verifier: verifier.clone(),
            expires: now + config.state_ttl,
        });
        Some((state, verifier))
    }
    pub(crate) fn consume_auth(&mut self, id: &str, expected: &str, now: Instant) -> Option<Auth> {
        self.cleanup(now);
        let session = self.entries.get_mut(id)?;
        if !session
            .auth
            .as_ref()
            .is_some_and(|auth| auth.state == expected)
        {
            return None;
        }
        session.auth.take()
    }
}
pub(crate) fn random() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
fn random_verifier() -> String {
    let mut bytes = [0u8; 48];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config::from_values(
            [
                ("OAUTH_PUBLIC_ORIGIN", "https://playground.test"),
                ("OAUTH_PLATFORM_ORIGIN", "https://platform.test"),
                ("OAUTH_MAX_SESSIONS", "2"),
                ("OAUTH_MAX_STATES", "1"),
            ]
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
        )
        .unwrap()
    }
    #[test]
    fn bounded_sessions_and_states_reject_capacity_and_release_after_expiry() {
        let config = config();
        let now = Instant::now();
        let mut store = Sessions::default();
        let a = store.create(&config, now).unwrap();
        let b = store.create(&config, now).unwrap();
        assert!(store.create(&config, now).is_none());
        let (state, _) = store.start_auth(&a, &config, now).unwrap();
        assert!(store.start_auth(&b, &config, now).is_none());
        assert!(store.consume_auth(&b, &state, now).is_none());
        assert!(store.consume_auth(&a, "wrong", now).is_none());
        assert!(store.consume_auth(&a, &state, now).is_some());
        assert!(store.consume_auth(&a, &state, now).is_none());
        assert!(store.start_auth(&b, &config, now).is_some());
        assert!(store.create(&config, now + config.session_ttl).is_some());
        assert_eq!(store.entries.len(), 1);
    }
    #[test]
    fn expired_auth_cannot_be_consumed_and_verifier_meets_pkce_bounds() {
        let config = config();
        let now = Instant::now();
        let mut store = Sessions::default();
        let id = store.create(&config, now).unwrap();
        let (state, verifier) = store.start_auth(&id, &config, now).unwrap();
        assert_eq!(verifier.len(), 64);
        assert_ne!(state, verifier);
        assert!(verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert!(store
            .consume_auth(&id, &state, now + config.state_ttl)
            .is_none());
    }
}
