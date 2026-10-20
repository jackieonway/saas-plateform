use crate::SafeError;
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};
use std::{fmt, fs::File, io::Read, path::Path};

pub(crate) const MAX_JSON_BYTES: usize = 1024 * 1024;
pub(crate) const PERMISSIONS: [&str; 6] = [
    "ticket:read",
    "ticket:create",
    "ticket:update",
    "ticket:delete",
    "ticket:export",
    "notification:publish",
];
const STORES_PERMISSIONS: [&str; 9] = [
    "store:read",
    "store:create",
    "store:update",
    "store-staff:read",
    "store-staff:assign",
    "store-staff:unassign",
    "store-daily-report:read",
    "store-daily-report:create",
    "store-daily-report:update",
];

// This is an internal reviewed application set, not a runtime plugin registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub(crate) enum KnownApp {
    #[serde(rename = "work-orders")]
    WorkOrders,
    #[serde(rename = "stores")]
    Stores,
}

pub(crate) struct MenuContract {
    pub(crate) key: &'static str,
    pub(crate) kind: &'static str,
    pub(crate) path: &'static str,
    pub(crate) permission: &'static str,
    pub(crate) visible: bool,
}
impl MenuContract {
    pub(crate) fn target(&self) -> &'static str {
        if self.kind == "MENU" {
            "MICRO"
        } else {
            "BLANK"
        }
    }
}
macro_rules! menu {
    ($key:literal, $path:literal, $perm:literal, $visible:literal) => {
        MenuContract {
            key: $key,
            kind: "MENU",
            path: $path,
            permission: $perm,
            visible: $visible,
        }
    };
    ($perm:literal) => {
        MenuContract {
            key: $perm,
            kind: "BUTTON",
            path: "",
            permission: $perm,
            visible: false,
        }
    };
}
const WORK_ORDERS_MENUS: [MenuContract; 7] = [
    menu!("root", "/micro/work-orders", "", true),
    menu!(
        "ticket:read",
        "/micro/work-orders/tickets/:id",
        "ticket:read",
        false
    ),
    menu!(
        "ticket:create",
        "/micro/work-orders/ticket-create",
        "ticket:create",
        true
    ),
    menu!("ticket:update"),
    menu!("ticket:delete"),
    menu!("ticket:export"),
    menu!("notification:publish"),
];
const STORES_MENUS: [MenuContract; 11] = [
    menu!("root", "/micro/stores", "", true),
    menu!(
        "store:read",
        "/micro/stores/stores/:id",
        "store:read",
        false
    ),
    menu!(
        "store:create",
        "/micro/stores/store-create",
        "store:create",
        true
    ),
    menu!("store:update"),
    menu!(
        "store-staff:read",
        "/micro/stores/staff",
        "store-staff:read",
        true
    ),
    menu!(
        "store-staff:assign",
        "/micro/stores/staff-assign",
        "store-staff:assign",
        true
    ),
    menu!("store-staff:unassign"),
    menu!(
        "store-daily-report:read",
        "/micro/stores/reports",
        "store-daily-report:read",
        true
    ),
    menu!(
        "store-daily-report:detail",
        "/micro/stores/daily-reports/:id",
        "store-daily-report:read",
        false
    ),
    menu!(
        "store-daily-report:create",
        "/micro/stores/daily-report-create",
        "store-daily-report:create",
        true
    ),
    menu!("store-daily-report:update"),
];
impl KnownApp {
    pub(crate) fn parse(code: &str) -> Option<Self> {
        match code {
            "work-orders" => Some(Self::WorkOrders),
            "stores" => Some(Self::Stores),
            _ => None,
        }
    }
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::WorkOrders => "work-orders",
            Self::Stores => "stores",
        }
    }
    pub(crate) fn base_path(self) -> &'static str {
        match self {
            Self::WorkOrders => "/micro/work-orders",
            Self::Stores => "/micro/stores",
        }
    }
    pub(crate) fn client_name(self) -> &'static str {
        match self {
            Self::WorkOrders => "work-orders-client",
            Self::Stores => "stores-client",
        }
    }
    pub(crate) fn entry_url(self) -> &'static str {
        match self {
            Self::WorkOrders => "/sub-apps/work-orders/unreleased/index.html",
            Self::Stores => "/sub-apps/stores/unreleased/index.html",
        }
    }
    pub(crate) fn permissions(self) -> &'static [&'static str] {
        match self {
            Self::WorkOrders => &PERMISSIONS,
            Self::Stores => &STORES_PERMISSIONS,
        }
    }
    pub(crate) fn menus(self) -> &'static [MenuContract] {
        match self {
            Self::WorkOrders => &WORK_ORDERS_MENUS,
            Self::Stores => &STORES_MENUS,
        }
    }
    pub(crate) fn bootstrap(self) -> &'static str {
        match self {
            Self::WorkOrders => include_str!("../manifests/bootstrap.manifest.json"),
            Self::Stores => include_str!("../manifests/stores-bootstrap.manifest.json"),
        }
    }
}
pub(crate) fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}
pub(crate) fn is_environment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || index > 0 && matches!(byte, b'_' | b'-')
        })
}
pub(crate) fn is_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.contains("..")
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || index > 0 && matches!(byte, b'.' | b'_' | b'-')
        })
}
pub(crate) fn read_json(path: &Path, category: &'static str) -> Result<Value, SafeError> {
    let file = File::open(path).map_err(|_| SafeError::new(category))?;
    if !file
        .metadata()
        .map_err(|_| SafeError::new(category))?
        .is_file()
    {
        return Err(SafeError::new(category));
    }
    let mut bytes = Vec::new();
    file.take(MAX_JSON_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SafeError::new(category))?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(SafeError::new(category));
    }
    strict_json(&bytes).map_err(|_| SafeError::new(category))
}
// serde_json::Value normally keeps the last duplicate key. The manifest and API
// boundaries must reject ambiguity before typed deserialization can hide it.
struct Strict(Value);
impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = Strict;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("unambiguous JSON")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Strict, E> {
                Ok(Strict(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Strict, E> {
                Ok(Strict(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Strict, E> {
                Ok(Strict(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Strict, E> {
                serde_json::Number::from_f64(value)
                    .map(|number| Strict(Value::Number(number)))
                    .ok_or_else(|| E::custom("number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Strict, E> {
                Ok(Strict(value.into()))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Strict, E> {
                Ok(Strict(value.into()))
            }
            fn visit_none<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Strict, A::Error> {
                let mut rows = Vec::new();
                while let Some(Strict(value)) = access.next_element()? {
                    rows.push(value);
                }
                Ok(Strict(Value::Array(rows)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Strict, A::Error> {
                let mut rows = Map::new();
                while let Some((key, Strict(value))) = access.next_entry::<String, Strict>()? {
                    if rows.insert(key, value).is_some() {
                        return Err(de::Error::custom("duplicate"));
                    }
                }
                Ok(Strict(Value::Object(rows)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}
pub(crate) fn strict_json(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice::<Strict>(bytes).map(|value| value.0)
}
pub(crate) fn uuid_field<'a>(
    row: &'a Value,
    key: &str,
    category: &'static str,
) -> Result<&'a str, SafeError> {
    row[key]
        .as_str()
        .filter(|id| is_uuid(id))
        .ok_or_else(|| SafeError::new(category))
}
pub(crate) fn rows(value: &Value) -> Result<&Vec<Value>, SafeError> {
    value
        .as_array()
        .ok_or_else(|| SafeError::new("INVALID_CONTRACT"))
}
