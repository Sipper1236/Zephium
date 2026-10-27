//! Profile-scoped WebsiteDataManager operations for the live WebKitGTK
//! context. Operations that WebKitGTK cannot represent exactly fail closed.

use gtk::gio::Cancellable;
use gtk::glib::translate::ToGlibPtr;
use webkit2gtk::{
    WebsiteDataManager, WebsiteDataManagerExt, WebsiteDataManagerExtManual, WebsiteDataTypes,
};
use zephium_core::browsing_data::{
    host_is_on_site, is_site_name, EngineDataCompletion, EngineDataFailure, EngineDataKinds,
    EngineDataReply, MAX_ENGINE_SITES,
};

const MAX_RECORDS: usize = 16_384;
// Added to the GTK 3 / WebKitGTK 4.1 ABI in WebKitGTK 2.56. The pinned Rust
// bindings predate it, so pass it only to runtimes that implement it.
const FILE_SYSTEM_BIT: u32 = 1 << 14;

pub(super) fn filesystem_supported() -> bool {
    // SAFETY: These ABI functions return immutable version constants.
    let version = unsafe {
        (
            webkit2gtk::ffi::webkit_get_major_version(),
            webkit2gtk::ffi::webkit_get_minor_version(),
        )
    };
    filesystem_supported_by(version)
}

fn filesystem_supported_by(version: (u32, u32)) -> bool {
    version >= (2, 56)
}

fn webkit_site_data(filesystem: bool) -> WebsiteDataTypes {
    use crate::data_kinds::WEBKIT_SITE_DATA;

    let types = WEBKIT_SITE_DATA.iter().copied().fold(
        WebsiteDataTypes::OFFLINE_APPLICATION_CACHE,
        |types, kind| {
            types
                | match kind {
                    crate::data_kinds::WebKitDataType::Cookies => WebsiteDataTypes::COOKIES,
                    crate::data_kinds::WebKitDataType::LocalStorage => {
                        WebsiteDataTypes::LOCAL_STORAGE
                    }
                    crate::data_kinds::WebKitDataType::SessionStorage => {
                        WebsiteDataTypes::SESSION_STORAGE
                    }
                    crate::data_kinds::WebKitDataType::IndexedDb => {
                        WebsiteDataTypes::INDEXEDDB_DATABASES
                    }
                    crate::data_kinds::WebKitDataType::WebSql => WebsiteDataTypes::WEBSQL_DATABASES,
                    crate::data_kinds::WebKitDataType::ServiceWorkers => {
                        WebsiteDataTypes::SERVICE_WORKER_REGISTRATIONS
                    }
                    crate::data_kinds::WebKitDataType::FetchCache => WebsiteDataTypes::DOM_CACHE,
                    crate::data_kinds::WebKitDataType::FileSystem => WebsiteDataTypes::empty(),
                    crate::data_kinds::WebKitDataType::DiskCache
                    | crate::data_kinds::WebKitDataType::MemoryCache => WebsiteDataTypes::empty(),
                }
        },
    );
    if filesystem {
        // The loaded 4.1 ABI recognizes this bit starting with 2.56. Preserve
        // it even though the pinned Rust binding predates that value.
        types | WebsiteDataTypes::from_bits_retain(FILE_SYSTEM_BIT)
    } else {
        types
    }
}

fn webkit_cache() -> WebsiteDataTypes {
    WebsiteDataTypes::DISK_CACHE | WebsiteDataTypes::MEMORY_CACHE
}

pub(super) fn all_website_data_types() -> WebsiteDataTypes {
    let types = WebsiteDataTypes::ALL;
    if filesystem_supported() {
        types | WebsiteDataTypes::from_bits_retain(FILE_SYSTEM_BIT)
    } else {
        types
    }
}

pub(crate) fn clear(
    manager: &WebsiteDataManager,
    kinds: EngineDataKinds,
    since: Option<i64>,
    done: EngineDataCompletion,
) {
    if crate::data_kinds::webkit_plan(kinds, since).is_empty() {
        done.finish(EngineDataReply::Cleared);
        return;
    }
    if kinds.site_data && (since.is_some() || !filesystem_supported()) {
        // WebKitGTK cannot clear cookies by age, and before 2.56 its public
        // API cannot address origin-private filesystem data at all.
        done.finish(EngineDataReply::Failed(EngineDataFailure::Unsupported));
        return;
    }
    if kinds.site_data {
        clear_site_records(manager, kinds.cache, done);
        return;
    }
    if !kinds.cache {
        done.finish(EngineDataReply::Cleared);
        return;
    }
    clear_cache(manager, since, done);
}

fn clear_cache(manager: &WebsiteDataManager, since: Option<i64>, done: EngineDataCompletion) {
    let age = match since {
        None => 0,
        Some(since) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs().min(i64::MAX as u64) as i64)
                .unwrap_or(0);
            if since >= now {
                done.finish(EngineDataReply::Cleared);
                return;
            }
            now.saturating_sub(since).min(i64::MAX / 1_000_000)
        }
    };
    manager.clear(
        webkit_cache(),
        gtk::glib::TimeSpan::from_seconds(age),
        None::<&Cancellable>,
        move |result| {
            done.finish(match result {
                Ok(()) => EngineDataReply::Cleared,
                Err(_) => EngineDataReply::Failed(EngineDataFailure::Failed),
            });
        },
    );
}

fn clear_site_records(manager: &WebsiteDataManager, cache: bool, done: EngineDataCompletion) {
    let types = webkit_site_data(true);
    let manager = manager.clone();
    let fetch_manager = manager.clone();
    fetch_manager.fetch(types, None::<&Cancellable>, move |result| {
        let records = match result {
            Ok(records) if records.len() <= MAX_RECORDS => records,
            Ok(_) => {
                done.finish(EngineDataReply::Failed(EngineDataFailure::Unavailable));
                return;
            }
            Err(_) => {
                done.finish(EngineDataReply::Failed(EngineDataFailure::Failed));
                return;
            }
        };
        let selected: Vec<_> = records
            .iter()
            .filter(|record| record.name().and_then(|name| record_host(&name)).is_some())
            .collect();
        if selected.is_empty() {
            if cache {
                clear_cache(&manager, None, done);
            } else {
                done.finish(EngineDataReply::Cleared);
            }
            return;
        }
        let join = std::sync::Arc::new(std::sync::Mutex::new(ClearJoin {
            remaining: if cache { 2 } else { 1 },
            failure: None,
            done: Some(done),
        }));
        let removed = join_clear(join.clone());
        manager.remove(types, &selected, None::<&Cancellable>, move |result| {
            removed.finish(match result {
                Ok(()) => EngineDataReply::Cleared,
                Err(_) => EngineDataReply::Failed(EngineDataFailure::Failed),
            });
        });
        if cache {
            clear_cache(&manager, None, join_clear(join));
        }
    });
}

struct ClearJoin {
    remaining: usize,
    failure: Option<EngineDataFailure>,
    done: Option<EngineDataCompletion>,
}

fn join_clear(join: std::sync::Arc<std::sync::Mutex<ClearJoin>>) -> EngineDataCompletion {
    EngineDataCompletion::new(move |reply| {
        let mut state = join.lock().unwrap_or_else(|error| error.into_inner());
        if reply != EngineDataReply::Cleared {
            state.failure.get_or_insert(match reply {
                EngineDataReply::Failed(failure) => failure,
                _ => EngineDataFailure::Failed,
            });
        }
        state.remaining -= 1;
        if state.remaining == 0 {
            let done = state.done.take();
            let failure = state.failure;
            drop(state);
            if let Some(done) = done {
                done.finish(failure.map_or(EngineDataReply::Cleared, EngineDataReply::Failed));
            }
        }
    })
}

fn record_host(name: &str) -> Option<String> {
    let name = name.trim().trim_start_matches('.').trim_end_matches('.');
    let bare = name
        .strip_prefix('[')
        .and_then(|name| name.strip_suffix(']'));
    let name = if let Ok(address) = bare.unwrap_or(name).parse::<std::net::Ipv6Addr>() {
        format!("[{address}]")
    } else {
        name.to_ascii_lowercase()
    };
    is_site_name(&name).then_some(name)
}

/// WebKit groups subdomains into a single record. An ancestor record may
/// contain unrelated siblings, so it cannot safely answer a narrower request.
fn record_is_broader_than_site(name: &str, site: &str) -> bool {
    record_host(name).is_some_and(|name| name != site && host_is_on_site(site, &name))
}

pub(crate) fn sites(manager: &WebsiteDataManager, done: EngineDataCompletion) {
    let filesystem = filesystem_supported();
    let types = webkit_site_data(filesystem);
    manager.fetch(types, None::<&Cancellable>, move |result| {
        let reply = match result {
            Err(_) => EngineDataReply::Failed(EngineDataFailure::Failed),
            Ok(records) => {
                let partial = !filesystem || records.len() > MAX_ENGINE_SITES;
                let raw = records.iter().take(MAX_RECORDS).filter_map(|record| {
                    let name = record_host(&record.name()?)?;
                    // SAFETY: record owns this native WebKitWebsiteData for
                    // the callback. The older Rust binding truncates new bits,
                    // so read the C flags to recognize 2.56 filesystem data.
                    let kinds = unsafe {
                        webkit2gtk::ffi::webkit_website_data_get_types(record.to_glib_none().0)
                    };
                    let cookie_bit = WebsiteDataTypes::COOKIES.bits();
                    let cookies = kinds & cookie_bit != 0;
                    let storage = kinds & !cookie_bit != 0;
                    Some((name, cookies, storage))
                });
                EngineDataReply::Sites {
                    sites: crate::data_kinds::merge_sites(raw),
                    partial,
                }
            }
        };
        done.finish(reply);
    });
}

/// Removes whole WebKit records only when none also covers an ancestor site.
pub(crate) fn forget_site(manager: &WebsiteDataManager, site: String, done: EngineDataCompletion) {
    if !is_site_name(&site) || !filesystem_supported() {
        done.finish(EngineDataReply::Failed(EngineDataFailure::Unsupported));
        return;
    }
    let Some(site) = record_host(&site) else {
        done.finish(EngineDataReply::Failed(EngineDataFailure::Unsupported));
        return;
    };
    let types = webkit_site_data(true) | webkit_cache();
    let manager = manager.clone();
    let fetch_manager = manager.clone();
    fetch_manager.fetch(types, None::<&Cancellable>, move |result| {
        let records = match result {
            Ok(records) if records.len() <= MAX_RECORDS => records,
            Ok(_) => {
                done.finish(EngineDataReply::Failed(EngineDataFailure::Unavailable));
                return;
            }
            Err(_) => {
                done.finish(EngineDataReply::Failed(EngineDataFailure::Failed));
                return;
            }
        };
        if records.iter().any(|record| {
            record
                .name()
                .is_some_and(|name| record_is_broader_than_site(&name, &site))
        }) {
            done.finish(EngineDataReply::Failed(EngineDataFailure::Unsupported));
            return;
        }
        let selected: Vec<_> = records
            .iter()
            .filter(|record| {
                record
                    .name()
                    .and_then(|name| record_host(&name))
                    .is_some_and(|name| host_is_on_site(&name, &site))
            })
            .collect();
        if selected.is_empty() {
            done.finish(EngineDataReply::Cleared);
            return;
        }
        manager.remove(types, &selected, None::<&Cancellable>, move |result| {
            done.finish(match result {
                Ok(()) => EngineDataReply::Cleared,
                Err(_) => EngineDataReply::Failed(EngineDataFailure::Failed),
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_data_requires_the_native_flag_introduced_in_256() {
        assert!(!filesystem_supported_by((2, 54)));
        assert!(!filesystem_supported_by((2, 55)));
        assert!(filesystem_supported_by((2, 56)));
        assert!(filesystem_supported_by((3, 0)));
        assert_eq!(webkit_site_data(false).bits() & FILE_SYSTEM_BIT, 0);
        assert_eq!(
            webkit_site_data(true).bits() & FILE_SYSTEM_BIT,
            FILE_SYSTEM_BIT
        );
    }

    #[test]
    fn a_grouped_ancestor_record_cannot_answer_a_subdomain_request() {
        assert!(record_is_broader_than_site(
            "example.test",
            "alpha.example.test"
        ));
        assert!(!record_is_broader_than_site("example.test", "example.test"));
        assert!(!record_is_broader_than_site(
            "beta.example.test",
            "alpha.example.test"
        ));
        assert!(!record_is_broader_than_site(
            "Local files",
            "alpha.example.test"
        ));
        assert_eq!(record_host(".EXAMPLE.test"), Some("example.test".into()));
        assert_eq!(record_host("0:0:0:0:0:0:0:1"), Some("[::1]".into()));
        assert_eq!(record_host("[0:0:0:0:0:0:0:1]"), Some("[::1]".into()));
        assert_eq!(record_host("Local files"), None);
    }

    #[test]
    fn a_combined_clear_waits_for_both_native_operations_and_propagates_failure() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let join = std::sync::Arc::new(std::sync::Mutex::new(ClearJoin {
            remaining: 2,
            failure: None,
            done: Some(EngineDataCompletion::new(move |reply| {
                sender.send(reply).unwrap();
            })),
        }));
        let site = join_clear(join.clone());
        let cache = join_clear(join);
        cache.finish(EngineDataReply::Cleared);
        assert!(receiver.try_recv().is_err());
        drop(site);
        assert_eq!(
            receiver.recv().unwrap(),
            EngineDataReply::Failed(EngineDataFailure::Unavailable)
        );
    }
}
