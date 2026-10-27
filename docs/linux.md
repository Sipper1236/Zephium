# Linux (experimental)

Zephium uses GTK 3 and the system WebKitGTK 4.1 engine on Linux. This port is
experimental, not a claim of feature parity with macOS or Windows. In particular,
WebExtensions, Ask Zephium's page assistant, native Work browsing, principal-owned
userscripts, match-scoped injection and `document_idle` scheduling remain
unavailable. The launcher uses Linux's portal/X11 backend;
shortcut recording and rebinding are available in Settings. Existing
runtime security checks still apply; do not disable WebKit's sandbox to launch it.

## Arch Linux

Use a fully updated system (partial upgrades are unsupported):

```sh
sudo pacman -Syu --needed base-devel rustup nodejs-lts-krypton pnpm python cmake desktop-file-utils \
  webkit2gtk-4.1 gtk3 libayatana-appindicator librsvg openssl sqlite xdg-utils \
  gst-plugins-base gst-plugins-good
```

Use the Node 24 version required by `package.json` and the pinned pnpm version
from its `packageManager` field. The repository's `rust-toolchain.toml` selects
the Rust toolchain through rustup.

From the repository root:

```sh
pnpm install --frozen-lockfile
pnpm dev
```

For a native pacman package, run as a normal user:

```sh
cd packaging/arch
makepkg -si
```

This PKGBUILD builds the current local checkout, including local modifications;
it is not an AUR source package. Build commands download pinned dependencies and
need network access. Launch `zephium-desktop` or select Zephium in your launcher.
Package upgrades should be installed through pacman, not the in-app updater.

The desktop entry supports HTTP/HTTPS links. To make it your default browser:

```sh
xdg-settings set default-web-browser app.zephium.desktop
```

On Wayland, install the XDG desktop portal backend appropriate for your desktop
to support portal-mediated features such as global shortcuts. Availability and
shortcut approval depend on the compositor. X11 uses the native shortcut path.

Open at Login uses the standard XDG Autostart directory and works on both X11
and Wayland. Turning it off writes a per-user `Hidden=true` override so a
system-wide autostart entry cannot silently turn it back on. The launcher status
in Settings reflects the actual native registration, including denial or loss
of the portal service. Space, Tab and modified F1–F12 chords have an X11 global
path. Recording releases the native grab/session and suppresses focused-window
shortcuts until it ends. X11 edits check the new grab and restore the previous
choice on a collision; configurable global text keys are refused until layout-safe
resolution exists. Existing text-key overrides keep the focused fallback. Wayland
edits create a new portal session: the chosen shortcut is saved, but registration
stays false while portal approval is pending or denied. Stale portal/X11 workers
cannot publish registration status or activate after replacement.

For other distributions, install the corresponding GTK 3, WebKitGTK 4.1,
AppIndicator, OpenSSL, SQLite and build dependencies before using `pnpm dev`.

Accounts, provider keys and MCP credentials require a running Secret Service
provider, such as GNOME Keyring or KWallet with Secret Service integration.
Zephium stores them in the service's persistent default collection and may ask
the service to unlock it. There is no plaintext or temporary-session fallback.
This is application credential storage; it does not add website password
AutoFill or browser passkey support.

Settings → Connections uses the same MCP server configuration, transport and
OAuth flow as Windows. The system vault stores any server credentials. Legacy
GitHub CLI, Git, Codex and Claude Code connections are disabled by the shared
command broker on Windows and Linux; their account-status probes are also
blocked. Their Settings rows report this as unavailable, rather than suggesting
that signing in again will enable them. Configuring an MCP server does not
enable Linux's missing native Work browser adapters.

## Other experimental limitations

Linux's native website brokers still deny file uploads, HTTP authentication
dialogs, color pickers and JavaScript dialogs. Camera, microphone, screen
sharing, geolocation, notifications and pointer-lock permissions remain denied
without an implemented origin-labelled broker. Automatic picture-in-picture
and fullscreen lifecycle behavior are not qualified on Linux. Do not infer
support for these surfaces from their availability in system WebKitGTK.

Favicon discovery and rasterization use a private WebKitGTK JavaScript world
and the committed view's navigation epoch. Browsing-data operations use the
exact live `WebsiteDataManager` attached to the requested profile. Cache-only
clearing works with or without a date range. On WebKitGTK 2.54, site listings
are partial and site-data clears and removals return unavailable: that runtime
cannot address origin-private filesystem data through its public data API. The
adapter has a path for WebKitGTK 2.56's filesystem-data flag, but that path
still needs native qualification. Date-limited site-data clears remain refused
because WebKitGTK cannot clear cookies by age. WebKit can group subdomains in
one record, so a request for a narrower host is refused when a broader record
could include other sites. Large listings are marked partial; removal refuses
an oversized record set. Profiles with no live native manager return
unavailable.

On WebKitGTK 2.54, deleting a profile whose native data manager has been used
in the current process cannot yet be verified, so deletion stays pending rather
than claiming the data is gone. Zephium retries journaled deletions on restart;
successful recovery depends on the profile's data being removable. An in-flight
browsing-data request also holds deletion until its native callback completes.

Linux does not yet expose the detailed media player controls or qualified
popup policy that Zephium has on Windows and macOS. WebKitGTK reports whether a
view is playing audio, but that signal alone cannot identify or control the
active media element. Its popup API does not identify the initiating frame's
origin, so it cannot safely enforce the existing origin allowlist.

Native download pause and resume are unsupported. The download controls keep
cancellation available and omit Pause and Resume on Linux. Every download
requires a save dialog, including when automatic saving is selected in
preferences, because WebKitGTK
does not expose trustworthy initiation evidence. Destination filesystems must
support exclusive rename and the XDG origin extended attribute; a failure to
preserve the protection metadata rejects publication. Executable content is
not launched through the download panel's Open action.

## Windows parity target

Linux remains an experimental WebKitGTK port. This table tracks the native
adapters and their remaining qualification or implementation work:

| Area | Linux implementation / remaining work |
| --- | --- |
| Window composition, tabs, split stage and find | GTK/WebKitGTK adapters exist; exercise native resizing, focus, drag/drop and page flows on both displays. |
| Launcher | X11 grabs and Wayland portal registration, persisted recording/rebinding and native status readback. Layout-safe global text keys remain unavailable on X11; Wayland approval depends on the compositor. |
| Open at Login | XDG Autostart with system/user precedence and readback. |
| Favicons | Isolated WebKitGTK observation and fixed-size rasterization, bound to the committed document. |
| Browsing data | Profile-scoped cache clearing and partial site listing work on 2.54. Complete site-data removal needs the WebKitGTK 2.56 filesystem-data API and native qualification; dated site-data removal and removal of a host inside a broader grouped record are refused. |
| Media and popups | Audio-state readback exists; detailed player actions and origin-authenticated popup policy remain unsupported. |
| Downloads | Native WebKitGTK transport and GTK consent per transfer use the shared coordinator, Store/recovery, identity-checked staging, exclusive publication and XDG origin metadata. Native pause/resume and initial blank-popup download handoff remain unsupported; controls use native capability readback. |
| Extensions | WebKitGTK has no equivalent to Chromium's native extension runtime or macOS WKWebExtension. Full support needs a new runtime or a different Linux engine, with principal isolation and bounded messages; package verification alone does not execute extensions. |
| Ask / Work browsing | Shared optional runtime compiles, but native reader, agent-context and retained-page adapters under `crates/zephium-engine/src/platform/linux/` remain unimplemented. |
| Provider keys, account tokens and MCP credentials | System Secret Service vault using encrypted sessions, exact application namespaces and persistent collections. Requires an installed/unlocked service such as GNOME Keyring or KWallet; service failures never fall back to plaintext files. |
| Work command execution | Arbitrary commands are disabled on every platform, including Windows. Linux also lacks the separately qualified private guard used for typed Git review on macOS; a sandbox must enforce grants, network denial, process custody and acknowledged cleanup before either capability can be admitted. |
| Updates | Pacman owns Arch package upgrades. Do not replace packaged files with AppImage updater logic. |

These are implementation gaps, not permissions to silently drop security
checks or return successful placeholder results. Shared frontend IPC, Store,
admission, lifecycle and policy code remain the owners of their existing
contracts; OS-specific operations belong in their platform adapters.

The optional engine `agentic-browser` feature compiles on Linux for shared
runtime development. Its native context construction, cookie transfer, page
observation and screenshot requests remain unsupported before native admission;
compiling this feature does not enable Ask or Work browsing. A Linux adapter
must prove isolated document worlds, bounded native results, exact document
identity, separate profile storage, navigation ownership and acknowledged native
teardown before the desktop can advertise these capabilities.

## Verification

On 27 October 2026, the current x86_64 Arch `1.0.3-4` package passed an
isolated release-1 install, release-4 upgrade and removal, packaged-resource
and executable-identity checks, and 10-second startup probes on X11, headless
Wayland and the live Hyprland session using disposable data. These checks used
the host runtime libraries and do not establish full browser functionality or a
clean distribution image. The checksum and detailed evidence are in
[the handoff](linux-handoff.md).

On 27 October 2026, an x86_64 Arch `1.0.3-3` package passed an isolated
release-1 install, release-3 upgrade and removal, resource and executable
identity checks, and 10-second X11 and headless Wayland startup probes. The
freshly built executable also passed a 10-second startup probe on the current
Hyprland v0.56.2 session with a disposable profile. That package was built
before the browsing-data safety fix described above; rebuild it before
distribution. The checks used host runtime libraries, not a clean Arch image.

On 26 October 2026, the local x86_64 Arch `1.0.3-2` package was rebuilt with
Rust 1.95.0, Node 24.18.0 and pnpm 11.17.0. It passed isolated installation,
upgrade from `1.0.3-1`, and removal. Packaged desktop/icon/legal resources and
executable identity were checked against the build inputs. Its initialized
window remained alive for 10 seconds on X11, headless fullscreen Wayland, and
the physical Hyprland v0.56.2 desktop. The final build includes the desktop lint
cleanup; the exact bundled blocker also passed native compilation and cache
reload.
These checks used the host's runtime libraries, not a clean distribution image.
The native sandbox/IPC fixture also passed with its isolation assertions intact.
Real compositor shortcut approval, KWallet, aarch64 and native Windows/macOS
regression checks remain unverified. The current artifact checksum and detailed
evidence are recorded in [the handoff](linux-handoff.md).

```sh
node --test scripts/require-supported-platform.test.mjs
python3 -m unittest discover -s scripts/ci -p 'test_probe_linux_startup.py'
python3 scripts/ci/probe_linux_credentials.py
cargo test -p zephium-engine --features agentic-browser --lib --locked -- --test-threads=1
cargo test -p zephium-desktop --lib --locked -- --test-threads=1
xvfb-run -a env GDK_BACKEND=x11 cargo test -p zephium-desktop --lib --locked \
  -- --ignored --test-threads=1
pnpm --dir desktop exec tauri build --debug --no-bundle -- --locked
python3 scripts/ci/probe_linux_startup.py target/debug/zephium-desktop --backend x11
python3 scripts/ci/probe_linux_startup.py target/debug/zephium-desktop --backend wayland
xvfb-run -a env GDK_BACKEND=x11 cargo test -p zephium-engine --lib --locked \
  host::downloads::platform::tests::native_attachment_download_is_view_scoped_async_exclusive_and_cancellable \
  -- --ignored --exact --test-threads=1
```

The display probes need Xvfb/Xauth or Weston, plus `dbus-run-session`. They use
disposable XDG directories and a private session bus, and require the initialized
window to remain alive, not just appear briefly. The Wayland probe deliberately
uses a 1024×640 fullscreen compositor to catch fixed-size startup disconnects.
They do not prove paid-provider, extension, browsing or portal-approval flows.
The Linux CI job covers compilation, unit tests and native X11/GTK tests;
runtime smoke tests additionally need a WebKitGTK version meeting the app's
current security floor.

To exercise startup on the current physical desktop with disposable profile data:

```sh
python3 scripts/ci/probe_linux_startup.py target/debug/zephium-desktop \
  --backend wayland --current-session --settle 15
```

Use `--backend x11` in an X11 session. This displays a temporary window and
uses a private bus; it does not test approval through the user's portal or
access their profile or keyring. Startup on a real compositor is one check,
not qualification of browsing, extensions, paid providers or input flows.
