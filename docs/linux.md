# Linux source builds (experimental)

Linux uses the existing GTK 3 and system WebKitGTK 4.1 backend. Source builds
are allowed for development, but Linux is not a supported release target yet.
The macOS and Windows feature sets are still ahead of this backend.

## Building

Use the Node version in `.node-version`, the pnpm version in `package.json`, and
the Rust toolchain in `rust-toolchain.toml`. Install the native development
dependencies for your distribution: GTK 3, WebKitGTK 4.1, AppIndicator, OpenSSL,
SQLite and the usual compiler/build tools. GStreamer plugins are needed for
media playback.

From the repository root:

```sh
pnpm install --frozen-lockfile
pnpm dev
```

WebKitGTK must meet the runtime security floor enforced by Zephium. Keep its
sandbox enabled. A successful build does not qualify the browser's native
features or desktop integration.

## Current gaps

These still need Linux implementations or qualification:

- Native download transport.
- Extension hosting. WebKitGTK has no native WebExtension runtime.
- Ask's native page reader.
- Work's native browser adapters.
- Persistent application credentials through Secret Service.
- A qualified sandbox for Work command execution.

Window behavior, global shortcuts and other desktop integration also need
testing across X11 and Wayland compositors. Linux onboarding stays resizable
so Wayland can fit it to displays smaller than the preferred window size.

## Targeted checks

```sh
node --test scripts/require-supported-platform.test.mjs
cargo test -p zephium-engine --features agentic-browser --lib --locked
```

The platform guard tests cover macOS, Windows, Linux and rejection of an
unsupported host. The optional engine tests exercise shared logic; passing
them does not enable the missing native Ask or Work adapters.
