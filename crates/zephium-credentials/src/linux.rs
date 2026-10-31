//! Secret Service API for Linux using FreeDesktop org.freedesktop.secrets over D-Bus.

use std::collections::HashMap;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zeroize::Zeroizing;

use crate::VaultError;

const APP_NAME: &str = "zephium";
const MAX_SECRET_BYTES: usize = 4096;

fn validate(target: &str) -> Result<(), VaultError> {
    if !target.starts_with("app.zephium.") || target.contains('\0') || target.contains('*') {
        return Err(VaultError::Invalid);
    }
    if target.len() > 1024 {
        return Err(VaultError::Invalid);
    }
    Ok(())
}

fn connect() -> Result<Connection, VaultError> {
    Connection::session().map_err(|_| VaultError::Inaccessible)
}

fn open_session(conn: &Connection) -> Result<OwnedObjectPath, VaultError> {
    let reply = conn
        .call_method(
            Some("org.freedesktop.secrets"),
            "/org/freedesktop/secrets",
            Some("org.freedesktop.Secret.Service"),
            "OpenSession",
            &("plain", Value::from("")),
        )
        .map_err(|_| VaultError::Inaccessible)?;
    let (_output, session_path): (OwnedValue, OwnedObjectPath) = reply
        .body()
        .deserialize()
        .map_err(|_| VaultError::Inaccessible)?;
    Ok(session_path)
}

fn search_service(
    conn: &Connection,
    target: &str,
) -> Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>), VaultError> {
    let mut attributes = HashMap::new();
    attributes.insert("application", APP_NAME);
    attributes.insert("service", target);
    let reply = conn
        .call_method(
            Some("org.freedesktop.secrets"),
            "/org/freedesktop/secrets",
            Some("org.freedesktop.Secret.Service"),
            "SearchItems",
            &(attributes,),
        )
        .map_err(|_| VaultError::Inaccessible)?;
    let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = reply
        .body()
        .deserialize()
        .map_err(|_| VaultError::Inaccessible)?;
    Ok((unlocked, locked))
}

/// Copies one bounded secret into zeroizing storage.
pub fn read(target: &str) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    validate(target)?;
    let conn = connect()?;
    let session = open_session(&conn)?;
    let (unlocked, locked) = search_service(&conn, target)?;
    if unlocked.is_empty() {
        if !locked.is_empty() {
            return Err(VaultError::AwaitingApproval);
        }
        return Err(VaultError::Missing);
    }
    let item_path = &unlocked[0];
    let reply = conn
        .call_method(
            Some("org.freedesktop.secrets"),
            item_path,
            Some("org.freedesktop.Secret.Item"),
            "GetSecret",
            &(session,),
        )
        .map_err(|_| VaultError::Inaccessible)?;
    let (_session, _parameters, value, _content_type): (OwnedObjectPath, Vec<u8>, Vec<u8>, String) =
        reply
            .body()
            .deserialize()
            .map_err(|_| VaultError::Inaccessible)?;

    if value.len() > MAX_SECRET_BYTES {
        return Err(VaultError::Invalid);
    }
    Ok(Zeroizing::new(value))
}

/// Tests exact presence without copying secret bytes.
pub fn present(target: &str) -> Result<bool, VaultError> {
    validate(target)?;
    let conn = connect()?;
    let (unlocked, locked) = search_service(&conn, target)?;
    Ok(!unlocked.is_empty() || !locked.is_empty())
}

/// Atomically stores or replaces a secret.
pub fn write(target: &str, bytes: &[u8]) -> Result<(), VaultError> {
    validate(target)?;
    if bytes.is_empty() || bytes.len() > MAX_SECRET_BYTES {
        return Err(VaultError::Invalid);
    }
    let conn = connect()?;
    let session = open_session(&conn)?;

    let mut attributes = HashMap::new();
    attributes.insert("application", Value::from(APP_NAME));
    attributes.insert("service", Value::from(target));

    let mut properties = HashMap::new();
    properties.insert(
        "org.freedesktop.Secret.Item.Label",
        Value::from(format!("Zephium: {target}")),
    );
    properties.insert(
        "org.freedesktop.Secret.Item.Attributes",
        Value::from(attributes),
    );

    let secret = (session, Vec::<u8>::new(), bytes, "text/plain");

    // Try default alias collection first, then login collection.
    let collections = [
        "/org/freedesktop/secrets/aliases/default",
        "/org/freedesktop/secrets/collection/login",
    ];

    let mut written = false;
    for col in collections {
        if let Ok(reply) = conn.call_method(
            Some("org.freedesktop.secrets"),
            col,
            Some("org.freedesktop.Secret.Collection"),
            "CreateItem",
            &(properties.clone(), &secret, true),
        ) {
            let (_item, _prompt): (OwnedObjectPath, OwnedObjectPath) =
                match reply.body().deserialize() {
                    Ok(res) => res,
                    Err(_) => continue,
                };
            written = true;
            break;
        }
    }

    if !written {
        return Err(VaultError::Inaccessible);
    }
    Ok(())
}

/// Deletes the exact secret; deleting a missing item succeeds.
pub fn delete(target: &str) -> Result<(), VaultError> {
    validate(target)?;
    let conn = connect()?;
    let (unlocked, _locked) = search_service(&conn, target)?;
    for item in unlocked {
        let _ = conn.call_method(
            Some("org.freedesktop.secrets"),
            &item,
            Some("org.freedesktop.Secret.Item"),
            "Delete",
            &(),
        );
    }
    Ok(())
}

/// Enumerates Zephium credentials matching prefix.
pub fn list(prefix: &str) -> Result<Vec<String>, VaultError> {
    validate(prefix)?;
    let conn = connect()?;
    let mut attributes = HashMap::new();
    attributes.insert("application", APP_NAME);
    let reply = conn
        .call_method(
            Some("org.freedesktop.secrets"),
            "/org/freedesktop/secrets",
            Some("org.freedesktop.Secret.Service"),
            "SearchItems",
            &(attributes,),
        )
        .map_err(|_| VaultError::Inaccessible)?;
    let (unlocked, _locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = reply
        .body()
        .deserialize()
        .map_err(|_| VaultError::Inaccessible)?;

    let mut results = Vec::new();
    for item in unlocked {
        let Ok(reply) = conn.call_method(
            Some("org.freedesktop.secrets"),
            &item,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.Secret.Item", "Attributes"),
        ) else {
            continue;
        };
        let Ok((val,)): Result<(OwnedValue,), _> = reply.body().deserialize() else {
            continue;
        };
        let Ok(dict): Result<HashMap<String, String>, _> = val.try_into() else {
            continue;
        };
        if let Some(service) = dict.get("service") {
            if service.starts_with(prefix) {
                results.push(service.clone());
            }
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_validation_prevents_wildcards_nuls_and_external_namespaces() {
        for name in [
            "other.application",
            "app.zephium.a*",
            "app.zephium.a\0b",
            "",
        ] {
            assert_eq!(validate(name), Err(VaultError::Invalid));
        }
        assert_eq!(
            validate("app.zephium.account|https://api.zephium.app"),
            Ok(())
        );
        assert_eq!(validate("app.zephium.test.123"), Ok(()));
    }

    #[test]
    fn secret_service_credential_roundtrip_uses_unique_fixture() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
        let generated_id = format!(
            "{:x}.{:x}.{:x}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("fixture clock")
                .as_nanos(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        );
        let target = format!("app.zephium.test.{generated_id}");
        let _turn = crate::turn();
        match present(&target) {
            Ok(found) => {
                assert!(!found, "never overwrite an existing item");
            }
            Err(VaultError::Inaccessible | VaultError::AwaitingApproval) => {
                // Secret Service daemon unavailable or locked in test harness.
                return;
            }
            Err(err) => panic!("unexpected error checking fixture: {err:?}"),
        }

        struct FixtureCleanup(String);
        impl Drop for FixtureCleanup {
            fn drop(&mut self) {
                let _ = delete(&self.0);
            }
        }
        let cleanup = FixtureCleanup(target);
        let fixture = b"nonsecret Linux Secret Service credential fixture";
        if let Err(VaultError::Inaccessible | VaultError::AwaitingApproval) =
            write(&cleanup.0, fixture)
        {
            return;
        }
        assert_eq!(present(&cleanup.0), Ok(true));
        assert_eq!(read(&cleanup.0).expect("fixture read").as_slice(), fixture);
        let listed = list("app.zephium.test.").expect("list");
        assert!(listed.iter().any(|s| s == &cleanup.0));
        delete(&cleanup.0).expect("fixture deletion");
        assert_eq!(present(&cleanup.0), Ok(false));
        assert!(matches!(read(&cleanup.0), Err(VaultError::Missing)));
    }
}
