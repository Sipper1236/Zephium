//! Reads bookmarks and history from other browsers on this device. Pure
//! readers: they find profiles, copy what a running browser holds open, and
//! return neutral values. Nothing here writes to Zephium's own storage.

#![forbid(unsafe_code)]

mod arc;
mod chromium;
mod firefox;
mod icons;
mod mozlz4;
mod safari;
mod snapshot;
mod zen;

use std::path::{Path, PathBuf};

pub use icons::{https_origin, SourceIcon, MAX_ICONS};
pub use zephium_core::bookmarks::ImportNode;
pub use zephium_core::ports::store::ImportedVisit;

/// Imported trees are opened into their parent past this depth. Real
/// browsers nest a handful of levels; a hostile file cannot exhaust the stack.
pub const MAX_TREE_DEPTH: usize = 48;
/// Bookmarks read from one source, folders included.
pub const MAX_NODES: usize = 50_000;
/// How far back history is read.
pub const HISTORY_DAYS: i64 = 180;
/// Essentials read from one source; a row of kept sites, not a library.
pub const MAX_ESSENTIALS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Browser {
    Chrome,
    Arc,
    Safari,
    Firefox,
    Brave,
    Edge,
    Zen,
}

impl Browser {
    pub const ALL: [Browser; 7] = [
        Browser::Chrome,
        Browser::Arc,
        Browser::Zen,
        Browser::Safari,
        Browser::Firefox,
        Browser::Brave,
        Browser::Edge,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Browser::Chrome => "chrome",
            Browser::Arc => "arc",
            Browser::Safari => "safari",
            Browser::Firefox => "firefox",
            Browser::Brave => "brave",
            Browser::Edge => "edge",
            Browser::Zen => "zen",
        }
    }

    pub fn from_id(id: &str) -> Option<Browser> {
        Browser::ALL.into_iter().find(|browser| browser.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Browser::Chrome => "Chrome",
            Browser::Arc => "Arc",
            Browser::Safari => "Safari",
            Browser::Firefox => "Firefox",
            Browser::Brave => "Brave",
            Browser::Edge => "Edge",
            Browser::Zen => "Zen",
        }
    }
}

/// Where this user's browsers keep their data. Built from the environment in
/// production and from a scratch directory in tests.
#[derive(Clone, Debug)]
pub struct Locations {
    pub home: PathBuf,
    /// Windows %LOCALAPPDATA%.
    pub local_app_data: Option<PathBuf>,
    /// Windows %APPDATA%.
    pub app_data: Option<PathBuf>,
}

impl Locations {
    pub fn from_env() -> Option<Locations> {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)?;
        Some(Locations {
            home,
            local_app_data: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            app_data: std::env::var_os("APPDATA").map(PathBuf::from),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Stable within the source: a Chromium profile directory name, or a
    /// Firefox profile path as profiles.ini records it.
    pub id: String,
    pub name: String,
    pub bookmarks: bool,
    pub history: bool,
    /// Sites kept in the sidebar, which Zephium calls Essentials.
    pub essentials: bool,
}

/// A site another browser keeps pinned at the top of its sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Essential {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub browser: Browser,
    pub profiles: Vec<Profile>,
    /// The system must grant access first (Safari's Full Disk Access).
    pub needs_permission: bool,
}

#[derive(Debug)]
pub enum ImportError {
    /// The source or profile is not on this device any more.
    Missing,
    /// The system refused access (Safari without Full Disk Access).
    Permission,
    /// The browser holds the file locked (Windows, while it runs).
    Busy,
    /// The file is not in the shape this browser writes.
    Unreadable,
}

impl From<std::io::Error> for ImportError {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => ImportError::Missing,
            std::io::ErrorKind::PermissionDenied => ImportError::Permission,
            // ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION.
            _ if matches!(error.raw_os_error(), Some(32 | 33)) => ImportError::Busy,
            _ => ImportError::Unreadable,
        }
    }
}

impl From<rusqlite::Error> for ImportError {
    fn from(_: rusqlite::Error) -> Self {
        ImportError::Unreadable
    }
}

/// Browsers found on this device with at least one profile that has
/// something to import.
pub fn discover(locations: &Locations) -> Vec<Source> {
    Browser::ALL
        .into_iter()
        .filter_map(|browser| source(locations, browser))
        .collect()
}

fn source(locations: &Locations, browser: Browser) -> Option<Source> {
    let (profiles, needs_permission) = match browser {
        Browser::Safari => return safari::source(locations),
        Browser::Firefox => (
            firefox::profiles(locations, firefox::Family::Firefox),
            false,
        ),
        Browser::Zen => (firefox::profiles(locations, firefox::Family::Zen), false),
        Browser::Arc => (arc::profiles(locations), false),
        _ => (
            chromium::user_data(locations, browser)
                .map(|root| chromium::profiles(&root))
                .unwrap_or_default(),
            false,
        ),
    };
    (!profiles.is_empty()).then_some(Source {
        browser,
        profiles,
        needs_permission,
    })
}

/// Bookmarks of one profile, in that browser's own order.
pub fn bookmarks(
    locations: &Locations,
    browser: Browser,
    profile: &str,
) -> Result<Vec<ImportNode>, ImportError> {
    let mut budget = MAX_NODES;
    let nodes = match browser {
        Browser::Safari => safari::bookmarks(locations)?,
        Browser::Firefox => firefox::bookmarks(&firefox::profile_dir(
            locations,
            firefox::Family::Firefox,
            profile,
        )?)?,
        Browser::Zen => {
            let dir = firefox::profile_dir(locations, firefox::Family::Zen, profile)?;
            // The library first, then what each space keeps pinned.
            let mut nodes = firefox::bookmarks(&dir)?;
            if zen::has_session(&dir) {
                nodes.extend(zen::pinned(&dir)?);
            }
            nodes
        }
        Browser::Arc => arc::bookmarks(locations, profile)?,
        _ => chromium::bookmarks(&chromium::profile_dir(locations, browser, profile)?)?,
    };
    Ok(bound(nodes, 0, &mut budget))
}

/// Visits of one profile from the last `HISTORY_DAYS`, newest first.
pub fn history(
    locations: &Locations,
    browser: Browser,
    profile: &str,
    now: i64,
) -> Result<Vec<ImportedVisit>, ImportError> {
    let since = now - HISTORY_DAYS * 24 * 3600;
    let limit = zephium_core::ports::store::MAX_IMPORTED_VISITS;
    match browser {
        Browser::Safari => safari::history(locations, since, limit),
        Browser::Firefox => firefox::history(
            &firefox::profile_dir(locations, firefox::Family::Firefox, profile)?,
            since,
            limit,
        ),
        Browser::Zen => firefox::history(
            &firefox::profile_dir(locations, firefox::Family::Zen, profile)?,
            since,
            limit,
        ),
        Browser::Arc => chromium::history(&arc::profile_dir(locations, profile)?, since, limit),
        _ => chromium::history(
            &chromium::profile_dir(locations, browser, profile)?,
            since,
            limit,
        ),
    }
}

/// The icons one profile already holds for `wanted` origins, most wanted
/// first. Safari keeps its icon cache out of reach, so it has none to offer.
pub fn icons(
    locations: &Locations,
    browser: Browser,
    profile: &str,
    wanted: &[String],
) -> Result<Vec<SourceIcon>, ImportError> {
    match browser {
        Browser::Safari => Ok(Vec::new()),
        Browser::Firefox => icons::firefox(
            &firefox::profile_dir(locations, firefox::Family::Firefox, profile)?,
            wanted,
        ),
        Browser::Zen => icons::firefox(
            &firefox::profile_dir(locations, firefox::Family::Zen, profile)?,
            wanted,
        ),
        Browser::Arc => icons::chromium(&arc::profile_dir(locations, profile)?, wanted),
        _ => icons::chromium(&chromium::profile_dir(locations, browser, profile)?, wanted),
    }
}

/// Sites one profile keeps at the top of its sidebar, in its order. Only
/// browsers with such a row have any.
pub fn essentials(
    locations: &Locations,
    browser: Browser,
    profile: &str,
) -> Result<Vec<Essential>, ImportError> {
    match browser {
        Browser::Arc => arc::essentials(locations, profile),
        Browser::Zen => zen::essentials(&firefox::profile_dir(
            locations,
            firefox::Family::Zen,
            profile,
        )?),
        _ => Ok(Vec::new()),
    }
}

/// Caps a tree's size and depth: folders past the depth limit are opened
/// into their parent, and reading stops at the node budget. Only links
/// Zephium would open are kept, so what is counted is what can be added.
fn bound(nodes: Vec<ImportNode>, depth: usize, budget: &mut usize) -> Vec<ImportNode> {
    let mut kept = Vec::new();
    for node in nodes {
        if *budget == 0 {
            break;
        }
        match node {
            ImportNode::Folder { title, children } if depth < MAX_TREE_DEPTH => {
                *budget -= 1;
                let children = bound(children, depth + 1, budget);
                if !children.is_empty() {
                    kept.push(ImportNode::Folder { title, children });
                }
            }
            ImportNode::Folder { children, .. } => kept.extend(bound(children, depth, budget)),
            ImportNode::Link { ref url, .. } if !zephium_core::navigation::is_allowed_str(url) => {}
            link => {
                *budget -= 1;
                kept.push(link);
            }
        }
    }
    kept
}

/// A child path that stays inside `root`: no separators, no parent steps.
fn child(root: &Path, name: &str) -> Option<PathBuf> {
    let plain = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.contains('\0');
    plain.then(|| root.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(url: &str) -> ImportNode {
        ImportNode::Link {
            title: url.into(),
            url: url.into(),
        }
    }

    #[test]
    fn bounding_opens_deep_folders_and_drops_empty_ones() {
        let mut deep = link("https://deep.example/");
        for level in 0..(MAX_TREE_DEPTH + 5) {
            deep = ImportNode::Folder {
                title: format!("{level}"),
                children: vec![deep],
            };
        }
        let empty = ImportNode::Folder {
            title: "Empty".into(),
            children: vec![],
        };
        let mut budget = MAX_NODES;
        let bounded = bound(vec![deep, empty], 0, &mut budget);
        assert_eq!(bounded.len(), 1);
        let mut depth = 0;
        let mut node = &bounded[0];
        while let ImportNode::Folder { children, .. } = node {
            depth += 1;
            node = &children[0];
        }
        assert_eq!(depth, MAX_TREE_DEPTH);
        assert_eq!(node, &link("https://deep.example/"));
    }

    #[test]
    fn bounding_keeps_only_links_that_can_be_opened() {
        let mut budget = MAX_NODES;
        let bounded = bound(
            vec![
                link("https://ok.example/"),
                link("javascript:alert(1)"),
                link("chrome://settings/"),
                link("file:///C:/notes.txt"),
                link("https://user:secret@example.com/"),
                ImportNode::Folder {
                    title: "Internal".into(),
                    children: vec![link("edge://favorites/"), link("about:config")],
                },
                ImportNode::Folder {
                    title: "Mixed".into(),
                    children: vec![link("chrome://flags/"), link("http://also-ok.example/")],
                },
            ],
            0,
            &mut budget,
        );
        assert_eq!(
            bounded,
            vec![
                link("https://ok.example/"),
                ImportNode::Folder {
                    title: "Mixed".into(),
                    children: vec![link("http://also-ok.example/")]
                }
            ]
        );
    }

    #[test]
    fn bounding_stops_at_the_node_budget() {
        let mut budget = 3;
        let bounded = bound(
            (0..10)
                .map(|i| link(&format!("https://{i}.example/")))
                .collect(),
            0,
            &mut budget,
        );
        assert_eq!(bounded.len(), 3);
    }

    #[test]
    fn profile_names_cannot_leave_their_directory() {
        let root = Path::new("/data");
        assert_eq!(child(root, "Default"), Some(PathBuf::from("/data/Default")));
        for name in ["", ".", "..", "a/b", "a\\b", "../x"] {
            assert_eq!(child(root, name), None, "{name}");
        }
    }
}
