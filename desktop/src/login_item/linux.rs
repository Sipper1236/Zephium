//! Per-user XDG Autostart. No shell, service installation, or root access.

use super::LoginItemStatus;
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const DESKTOP_ID: &str = "app.zephium.desktop";
const MAX_ENTRY_BYTES: u64 = 16 * 1024;

struct Autostart {
    executable: String,
    directories: Vec<PathBuf>,
}

impl Autostart {
    fn current() -> Option<Self> {
        let executable = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && path.is_file())
            .or_else(|| std::env::current_exe().ok())?;
        let executable = desktop_exec(&executable)?;
        let user = user_config(
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        )?;
        let mut directories = vec![user];
        let system = std::env::var_os("XDG_CONFIG_DIRS")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/etc/xdg".into());
        directories.extend(std::env::split_paths(&system).filter(|path| path.is_absolute()));
        Some(Self {
            executable,
            directories,
        })
    }

    fn status(&self) -> LoginItemStatus {
        // XDG Autostart: the first existing basename wins. A hidden user entry
        // must not fall through to an enabled system entry.
        let enabled = self
            .directories
            .iter()
            .find_map(|directory| {
                let path = directory.join("autostart").join(DESKTOP_ID);
                match std::fs::symlink_metadata(&path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(_) => Some(false),
                    Ok(metadata) if !metadata.is_file() || metadata.len() > MAX_ENTRY_BYTES => {
                        Some(false)
                    }
                    Ok(_) => Some(
                        read_entry(&path)
                            .is_some_and(|entry| entry_enabled(&entry, &self.executable)),
                    ),
                }
            })
            .unwrap_or(false);
        LoginItemStatus {
            supported: true,
            enabled,
            needs_approval: false,
        }
    }

    fn set(&self, enabled: bool) -> std::io::Result<()> {
        let directory = self.directories[0].join("autostart");
        std::fs::create_dir_all(&directory)?;
        // Write a user override even when disabling: removing our file could
        // reveal a system-wide entry and start the app again at the next login.
        let mut entry = tempfile::NamedTempFile::new_in(&directory)?;
        write!(entry,
            "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Zephium\nExec={}\nIcon=app.zephium\nTerminal=false\nHidden={}\n",
            self.executable, !enabled,
        )?;
        entry.as_file().sync_all()?;
        entry
            .persist(directory.join(DESKTOP_ID))
            .map_err(|error| error.error)?;
        Ok(())
    }
}

fn user_config(config: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    config
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".config"))
        })
}

fn desktop_exec(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let text = path.to_str()?;
    if text.chars().any(char::is_control) {
        return None;
    }
    // Desktop Entry string unescaping happens before Exec argument unquoting.
    // A literal percent must also be doubled to prevent field-code expansion.
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\\\\\"),
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(character);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    Some(quoted)
}

fn read_entry(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_ENTRY_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() as u64 <= MAX_ENTRY_BYTES).then_some(text)
}

fn entry_enabled(text: &str, executable: &str) -> bool {
    let mut desktop = false;
    let mut valid_type = false;
    let mut valid_executable = false;
    let mut hidden = false;
    let mut disabled = false;
    let mut restricted = false;
    let mut seen = std::collections::HashSet::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            desktop = line == "[Desktop Entry]";
            continue;
        }
        if !desktop || line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        let key = key.trim();
        let value = value.trim();
        if !seen.insert(key) {
            return false;
        }
        match key {
            "Type" => valid_type = value == "Application",
            "Exec" => valid_executable = value == executable,
            "Hidden" => hidden = value != "false",
            "X-GNOME-Autostart-enabled" => disabled = value != "true",
            // Do not report an unconditional enabled state for an entry whose
            // desktop-specific conditions we have not evaluated.
            "OnlyShowIn" | "NotShowIn" | "TryExec" => restricted = true,
            _ => {}
        }
    }
    valid_type && valid_executable && !hidden && !disabled && !restricted
}

pub(super) fn status() -> LoginItemStatus {
    Autostart::current()
        .map(|entry| entry.status())
        .unwrap_or_default()
}

pub(super) fn set(enabled: bool) {
    if let Some(entry) = Autostart::current() {
        if let Err(error) = entry.set(enabled) {
            crate::write_diagnostic(format_args!(
                "login item: could not update XDG Autostart: {error}"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_are_read_back_and_user_disable_shadows_system_registration() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user");
        let system = root.path().join("system");
        let mut autostart = Autostart {
            executable: "\"/usr/bin/zephium-desktop\"".into(),
            directories: vec![system.clone()],
        };
        autostart.set(true).unwrap();
        autostart.directories = vec![user.clone(), system];
        assert!(autostart.status().enabled);
        autostart.set(false).unwrap();
        assert!(!autostart.status().enabled);
        autostart.set(true).unwrap();
        assert!(autostart.status().enabled);
        let path = user.join("autostart").join(DESKTOP_ID);
        std::fs::write(
            &path,
            format!(
                "{}\nX-GNOME-Autostart-enabled=false\n",
                std::fs::read_to_string(&path).unwrap()
            ),
        )
        .unwrap();
        assert!(!autostart.status().enabled);
        autostart.set(true).unwrap();
        assert!(autostart.status().enabled);
    }

    #[test]
    fn desktop_exec_never_becomes_shell_or_field_code_syntax() {
        assert_eq!(
            desktop_exec(Path::new("/opt/Zephium Browser/zephium")),
            Some("\"/opt/Zephium Browser/zephium\"".into())
        );
        assert_eq!(
            desktop_exec(Path::new("/opt/%u")),
            Some("\"/opt/%%u\"".into())
        );
        for path in ["relative", "/opt/a\nb", "/opt/a\tb"] {
            assert!(desktop_exec(Path::new(path)).is_none());
        }
        for character in ['"', '$', '`'] {
            let quoted = desktop_exec(Path::new(&format!("/opt/{character}a"))).unwrap();
            assert!(quoted.contains(&format!("\\\\{character}")));
        }
        assert!(desktop_exec(Path::new("/opt/a\\b"))
            .unwrap()
            .contains("\\\\\\\\"));
    }

    #[test]
    fn xdg_paths_must_be_absolute_and_unsafe_entries_are_not_enabled() {
        assert_eq!(
            user_config(
                Some(OsStr::new("relative")),
                Some(OsStr::new("/home/person"))
            ),
            Some(PathBuf::from("/home/person/.config"))
        );
        assert!(user_config(None, Some(OsStr::new("relative"))).is_none());
        let valid = "[Desktop Entry]\nType=Application\nExec=\"/usr/bin/zephium-desktop\"\n";
        let exe = "\"/usr/bin/zephium-desktop\"";
        assert!(entry_enabled(valid, exe));
        for extra in [
            "Hidden=true",
            "Hidden=invalid",
            "OnlyShowIn=GNOME;",
            "NotShowIn=KDE;",
            "Exec=/other",
        ] {
            assert!(!entry_enabled(&format!("{valid}{extra}\n"), exe));
        }
        assert!(!entry_enabled(
            "[Desktop Action x]\nType=Application\nExec=\"/usr/bin/zephium-desktop\"",
            exe
        ));
    }

    #[test]
    fn replacing_an_autostart_symlink_never_writes_its_target() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("autostart");
        std::fs::create_dir(&directory).unwrap();
        let unrelated = root.path().join("unrelated");
        std::fs::write(&unrelated, "do not change").unwrap();
        let path = directory.join(DESKTOP_ID);
        symlink(&unrelated, &path).unwrap();
        let autostart = Autostart {
            executable: "\"/usr/bin/zephium-desktop\"".into(),
            directories: vec![root.path().into()],
        };
        assert!(!autostart.status().enabled);
        autostart.set(true).unwrap();
        assert_eq!(std::fs::read_to_string(unrelated).unwrap(), "do not change");
        assert!(!path.is_symlink());
        assert!(autostart.status().enabled);
        std::fs::write(&path, vec![b'x'; MAX_ENTRY_BYTES as usize + 1]).unwrap();
        assert!(!autostart.status().enabled);
    }

    #[test]
    #[ignore = "requires desktop-file-validate; Linux CI runs this explicitly"]
    fn generated_entries_follow_the_desktop_entry_exec_escaping_rules() {
        let root = tempfile::tempdir().unwrap();
        for executable in ["/opt/Zephium Browser/zephium", "/opt/a\\b\"c$d`e%u"] {
            let autostart = Autostart {
                executable: desktop_exec(Path::new(executable)).unwrap(),
                directories: vec![root.path().into()],
            };
            autostart.set(true).unwrap();
            let result = std::process::Command::new("desktop-file-validate")
                .arg(root.path().join("autostart").join(DESKTOP_ID))
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
        }
    }
}
