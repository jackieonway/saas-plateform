//! UUID generation and validation at system-owned identity boundaries.
use serde::{Deserialize, Deserializer};
use uuid::{Uuid, Variant};

use crate::error::AppError;

pub(crate) fn new_uuid() -> Uuid {
    Uuid::now_v7()
}

pub(crate) fn is_uuid_v7(value: Uuid) -> bool {
    value.get_version_num() == 7 && value.get_variant() == Variant::RFC4122
}

pub(crate) fn validate_uuid(value: Uuid) -> Result<Uuid, AppError> {
    if is_uuid_v7(value) {
        Ok(value)
    } else {
        Err(AppError::bad_request("系统标识必须是 RFC UUID v7"))
    }
}

pub(crate) fn parse_uuid(value: &str) -> Result<Uuid, AppError> {
    validate_uuid(Uuid::parse_str(value).map_err(|_| AppError::bad_request("UUID 格式错误"))?)
}

/// A string adapter keeps existing caller-specific error mapping intact.
pub(crate) trait ParseUuid7 {
    fn parse_uuid7(&self) -> Result<Uuid, AppError>;
}

impl<T: AsRef<str> + ?Sized> ParseUuid7 for T {
    fn parse_uuid7(&self) -> Result<Uuid, AppError> {
        parse_uuid(self.as_ref())
    }
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    validate_uuid(Uuid::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}

pub(crate) fn deserialize_optional<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Uuid>, D::Error> {
    Option::<Uuid>::deserialize(deserializer)?
        .map(validate_uuid)
        .transpose()
        .map_err(serde::de::Error::custom)
}

pub(crate) fn deserialize_vec<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Uuid>, D::Error> {
    Vec::<Uuid>::deserialize(deserializer)?
        .into_iter()
        .map(validate_uuid)
        .collect::<Result<_, _>>()
        .map_err(serde::de::Error::custom)
}

/// Missing query fields stay absent; malformed fields must never erase a filter.
pub(crate) fn optional_query(value: Option<&String>) -> Result<Option<Uuid>, AppError> {
    value.map(|value| parse_uuid(value)).transpose()
}

#[cfg(test)]
pub(crate) fn fixture_uuid(value: u128) -> Uuid {
    let mut bytes = value.to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::{Arc, Barrier};

    #[test]
    fn concurrent_generated_ids_are_distinct_rfc_v7() {
        let start = Arc::new(Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    (0..1_000).map(|_| new_uuid()).collect::<Vec<_>>()
                })
            })
            .collect();
        let ids: HashSet<_> = workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(ids.len(), 8_000);
        assert!(ids.into_iter().all(is_uuid_v7));
    }

    #[test]
    fn inputs_reject_nil_other_versions_and_variants() {
        for value in [
            Uuid::nil(),
            Uuid::new_v4(),
            Uuid::parse_str("019a1163-0000-7000-0000-000000000001").unwrap(),
            Uuid::parse_str("019a1163-0000-7000-c000-000000000001").unwrap(),
        ] {
            assert!(validate_uuid(value).is_err());
        }
        assert!(parse_uuid("not-a-uuid").is_err());
        assert_eq!(
            parse_uuid(&fixture_uuid(1).to_string()).unwrap(),
            fixture_uuid(1)
        );
    }

    #[test]
    fn query_and_serde_inputs_do_not_turn_invalid_filters_into_none() {
        #[derive(Deserialize)]
        struct Input {
            #[serde(deserialize_with = "super::deserialize")]
            id: Uuid,
        }
        let invalid = Uuid::new_v4().to_string();
        assert!(optional_query(Some(&invalid)).is_err());
        assert!(optional_query(None).unwrap().is_none());
        assert!(serde_json::from_value::<Input>(serde_json::json!({"id":invalid})).is_err());
        let id = new_uuid();
        let input: Input = serde_json::from_value(serde_json::json!({"id":id})).unwrap();
        assert_eq!(input.id, id);
    }
}
