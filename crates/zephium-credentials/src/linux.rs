//! Exact, namespaced secrets in the session user's persistent Secret Service.
use crate::VaultError;
use secret_service::{
    blocking::{Collection, Item, SecretService},
    EncryptionType,
};
use std::collections::{HashMap, HashSet};
use zeroize::Zeroizing;

const APPLICATION: &str = "app.zephium";
const MAX_TARGET: usize = 32767;
const MAX_BLOB: usize = 2560;
const MAX_ITEMS: usize = 4096;

fn validate(target: &str) -> Result<(), VaultError> {
    if !target.starts_with("app.zephium.")
        || target.len() > MAX_TARGET
        || target.contains(['\0', '*'])
    {
        return Err(VaultError::Invalid);
    }
    Ok(())
}

fn connect() -> Result<SecretService<'static>, VaultError> {
    // Require an encrypted session. Never fall back to a file or plaintext session.
    SecretService::connect(EncryptionType::Dh).map_err(|_| VaultError::Inaccessible)
}

fn attributes(target: &str) -> HashMap<&str, &str> {
    HashMap::from([("application", APPLICATION), ("target", target)])
}

fn persistent_collection<'a>(service: &'a SecretService<'a>) -> Result<Collection<'a>, VaultError> {
    let collection = service
        .get_default_collection()
        .map_err(|_| VaultError::Inaccessible)?;
    match service.get_collection_by_alias("session") {
        Ok(session) if session.collection_path == collection.collection_path => {
            return Err(VaultError::Invalid)
        }
        Ok(_) | Err(secret_service::Error::NoResult) => {}
        Err(_) => return Err(VaultError::Inaccessible),
    }
    Ok(collection)
}

fn exact<'a>(service: &'a SecretService<'a>, target: &str) -> Result<Option<Item<'a>>, VaultError> {
    let collection = persistent_collection(service)?;
    let found = service
        .search_items(attributes(target))
        .map_err(|_| VaultError::Inaccessible)?;
    // Duplicate identities across collections are ambiguous; do not pick a secret arbitrarily.
    if found.unlocked.len() + found.locked.len() > 1 {
        return Err(VaultError::Invalid);
    }
    let Some(item) = found.unlocked.into_iter().chain(found.locked).next() else {
        return Ok(None);
    };
    if !collection
        .search_items(attributes(target))
        .map_err(|_| VaultError::Inaccessible)?
        .iter()
        .any(|persistent| persistent.item_path == item.item_path)
    {
        return Err(VaultError::Invalid);
    }
    Ok(Some(item))
}

/// Reads one exact bounded secret into zeroizing storage, asking the service to unlock it.
pub fn read(target: &str) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    validate(target)?;
    let service = connect()?;
    let item = exact(&service, target)?.ok_or(VaultError::Missing)?;
    item.unlock().map_err(|_| VaultError::Inaccessible)?;
    let bytes = Zeroizing::new(item.get_secret().map_err(|_| VaultError::Inaccessible)?);
    if bytes.len() > MAX_BLOB {
        return Err(VaultError::Invalid);
    }
    Ok(bytes)
}

/// Checks attributes only, including locked items, without unlocking or reading a secret.
pub fn present(target: &str) -> Result<bool, VaultError> {
    validate(target)?;
    let service = connect()?;
    let present = exact(&service, target)?.is_some();
    Ok(present)
}

/// Replaces one item in the user's persistent default collection, with no session fallback.
pub fn write(target: &str, bytes: &[u8]) -> Result<(), VaultError> {
    validate(target)?;
    if bytes.len() > MAX_BLOB {
        return Err(VaultError::Invalid);
    }
    let service = connect()?;
    if let Some(item) = exact(&service, target)? {
        item.unlock().map_err(|_| VaultError::Inaccessible)?;
        return item
            .set_secret(bytes, "application/octet-stream")
            .map_err(|_| VaultError::Inaccessible);
    }
    let collection = persistent_collection(&service)?;
    collection.unlock().map_err(|_| VaultError::Inaccessible)?;
    collection
        .create_item(
            "Zephium credential",
            attributes(target),
            bytes,
            true,
            "application/octet-stream",
        )
        .map_err(|_| VaultError::Inaccessible)?;
    Ok(())
}

/// Deletes only the exact item; absent items succeed, service failures remain errors.
pub fn delete(target: &str) -> Result<(), VaultError> {
    validate(target)?;
    let service = connect()?;
    if let Some(item) = exact(&service, target)? {
        item.unlock().map_err(|_| VaultError::Inaccessible)?;
        item.delete().map_err(|_| VaultError::Inaccessible)?;
    }
    Ok(())
}

/// Lists only this application's target attributes under an exact prefix, never secret bytes.
pub fn list(prefix: &str) -> Result<Vec<String>, VaultError> {
    validate(prefix)?;
    let service = connect()?;
    let collection = persistent_collection(&service)?;
    let persistent: HashSet<_> = collection
        .search_items(HashMap::from([("application", APPLICATION)]))
        .map_err(|_| VaultError::Inaccessible)?
        .into_iter()
        .map(|item| item.item_path)
        .collect();
    let items = service
        .search_items(HashMap::from([("application", APPLICATION)]))
        .map_err(|_| VaultError::Inaccessible)?;
    let mut targets = Vec::new();
    for item in items.unlocked.into_iter().chain(items.locked) {
        let attrs = item
            .get_attributes()
            .map_err(|_| VaultError::Inaccessible)?;
        if let Some(target) = attrs.get("target") {
            push_matching_target(
                &mut targets,
                prefix,
                target,
                persistent.contains(&item.item_path),
            )?;
        }
    }
    targets.sort();
    if targets.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(VaultError::Invalid);
    }
    Ok(targets)
}

fn push_matching_target(
    targets: &mut Vec<String>,
    prefix: &str,
    target: &str,
    persistent: bool,
) -> Result<(), VaultError> {
    if !target.starts_with(prefix) {
        return Ok(());
    }
    validate(target)?;
    if !persistent {
        return Err(VaultError::Invalid);
    }
    if targets.len() >= MAX_ITEMS {
        return Err(VaultError::Capacity);
    }
    targets.push(target.to_owned());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_targets_and_oversized_blobs_are_rejected_before_connecting() {
        for target in ["other.application", "app.zephium.a*", "app.zephium.a\0b"] {
            assert_eq!(validate(target), Err(VaultError::Invalid));
            assert_eq!(present(target), Err(VaultError::Invalid));
        }
        assert_eq!(
            validate(&format!("app.zephium.{}", "x".repeat(MAX_TARGET))),
            Err(VaultError::Invalid)
        );
        assert_eq!(
            write("app.zephium.test", &vec![0; MAX_BLOB + 1]),
            Err(VaultError::Invalid)
        );
    }

    #[test]
    fn listing_capacity_applies_only_to_the_requested_prefix() {
        let prefix = "app.zephium.test.connection/";
        let mut targets = Vec::new();
        for _ in 0..=MAX_ITEMS {
            push_matching_target(&mut targets, prefix, "app.zephium.unrelated/item", false)
                .unwrap();
        }
        assert!(targets.is_empty());
        for number in 0..MAX_ITEMS {
            push_matching_target(&mut targets, prefix, &format!("{prefix}{number}"), true).unwrap();
        }
        assert_eq!(
            push_matching_target(&mut targets, prefix, &format!("{prefix}overflow"), true),
            Err(VaultError::Capacity)
        );
        assert_eq!(
            push_matching_target(
                &mut Vec::new(),
                prefix,
                &format!("{prefix}temporary"),
                false
            ),
            Err(VaultError::Invalid)
        );
    }

    #[test]
    #[ignore = "requires an intentionally unavailable session bus"]
    fn unavailable_service_fails_closed() {
        let target = "app.zephium.test.unavailable";
        assert_eq!(present(target), Err(VaultError::Inaccessible));
        assert_eq!(read(target), Err(VaultError::Inaccessible));
        assert_eq!(
            write(target, b"nonsecret fixture"),
            Err(VaultError::Inaccessible)
        );
        assert_eq!(delete(target), Err(VaultError::Inaccessible));
        assert_eq!(list(target), Err(VaultError::Inaccessible));
    }

    #[test]
    #[ignore = "requires an isolated Secret Service with an unlocked default collection"]
    fn native_roundtrip_replacement_and_exact_deletion() {
        let _turn = crate::turn().unwrap();
        let target = format!(
            "app.zephium.test.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        assert!(!present(&target).unwrap());
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete(&self.0);
            }
        }
        let cleanup = Cleanup(target);
        assert_eq!(read(&cleanup.0), Err(VaultError::Missing));
        write(&cleanup.0, b"nonsecret fixture").unwrap();
        assert!(present(&cleanup.0).unwrap());
        assert_eq!(read(&cleanup.0).unwrap().as_slice(), b"nonsecret fixture");
        assert_eq!(list(&cleanup.0).unwrap(), vec![cleanup.0.clone()]);
        write(&cleanup.0, b"replacement").unwrap();
        assert_eq!(read(&cleanup.0).unwrap().as_slice(), b"replacement");
        // A second matching item must not turn a read or replacement into an
        // arbitrary choice. Remove the duplicate before asserting so cleanup works.
        let service = connect().unwrap();
        let collection = service.get_default_collection().unwrap();
        let duplicate = collection
            .create_item(
                "duplicate fixture",
                attributes(&cleanup.0),
                b"duplicate",
                false,
                "application/octet-stream",
            )
            .unwrap();
        let duplicate_read = read(&cleanup.0);
        let duplicate_write = write(&cleanup.0, b"must not replace either");
        let duplicate_presence = present(&cleanup.0);
        let duplicate_list = list(&cleanup.0);
        duplicate.delete().unwrap();
        assert_eq!(duplicate_read, Err(VaultError::Invalid));
        assert_eq!(duplicate_write, Err(VaultError::Invalid));
        assert_eq!(duplicate_presence, Err(VaultError::Invalid));
        assert_eq!(duplicate_list, Err(VaultError::Invalid));
        assert_eq!(read(&cleanup.0).unwrap().as_slice(), b"replacement");
        delete(&cleanup.0).unwrap();
        delete(&cleanup.0).unwrap();
        assert!(!present(&cleanup.0).unwrap());

        let session = service.get_collection_by_alias("session").unwrap();
        let temporary = session
            .create_item(
                "temporary fixture",
                attributes(&cleanup.0),
                b"session only",
                false,
                "application/octet-stream",
            )
            .unwrap();
        assert_eq!(read(&cleanup.0), Err(VaultError::Invalid));
        assert_eq!(present(&cleanup.0), Err(VaultError::Invalid));
        assert_eq!(
            write(&cleanup.0, b"must not report persistence"),
            Err(VaultError::Invalid)
        );
        assert_eq!(delete(&cleanup.0), Err(VaultError::Invalid));
        assert_eq!(list(&cleanup.0), Err(VaultError::Invalid));
        assert_eq!(&*temporary.get_secret().unwrap(), b"session only");
        temporary.delete().unwrap();
        write(&cleanup.0, b"persistent").unwrap();
        let duplicate = session
            .create_item(
                "temporary duplicate",
                attributes(&cleanup.0),
                b"duplicate",
                false,
                "application/octet-stream",
            )
            .unwrap();
        assert_eq!(read(&cleanup.0), Err(VaultError::Invalid));
        assert_eq!(
            write(&cleanup.0, b"must not replace either"),
            Err(VaultError::Invalid)
        );
        duplicate.delete().unwrap();
        assert_eq!(read(&cleanup.0).unwrap().as_slice(), b"persistent");
    }

    #[test]
    #[ignore = "requires an isolated Secret Service without a default collection alias"]
    fn missing_default_collection_fails_closed() {
        let target = "app.zephium.test.no-default";
        assert_eq!(read(target), Err(VaultError::Inaccessible));
        assert_eq!(present(target), Err(VaultError::Inaccessible));
        assert_eq!(
            write(target, b"must not use session"),
            Err(VaultError::Inaccessible)
        );
        assert_eq!(delete(target), Err(VaultError::Inaccessible));
        assert_eq!(list(target), Err(VaultError::Inaccessible));
    }
}
