# Bringing Linux support upstream

Only [draft PR #221](https://github.com/zephium-browser/Zephium/pull/221) is open.
Further submissions need the user's go-ahead. Keep each PR focused on one
change and describe the behavior and evidence on that branch.

The first draft allows experimental source builds and fixes the shared engine
checks needed for Linux. Its engine suite passes 359 tests with Rust 1.95.0.
The larger local port has additional tests and features, so its 377-test result
must stay separate from the PR's results. The draft still needs desktop build,
native startup and lint qualification before it can be considered ready.

## Possible next steps

Keep the requested order, adjusting each patch to its actual dependencies:

| Step | Scope | Before submitting |
| --- | --- | --- |
| Startup probes | `scripts/ci/probe_linux_startup.py` and its tests. Preserve X11 authority when using a disposable HOME. | These are new files upstream. Describe this as adding isolated startup probes, rather than fixing an existing upstream probe. Keep credential tests with the credential adapter they require. |
| Arch recipe | `packaging/arch/PKGBUILD`, desktop entry and packaging ignore rules. | Base it on the chosen upstream revision. The current recipe invokes the startup-probe tests, so those files need to land first or the check phase needs an appropriate independent scope. |
| Open at Login | `desktop/src/login_item.rs` and `desktop/src/login_item/linux.rs`. | Explain XDG precedence, status readback and the user disable override. Keep engine and credential changes out of this patch. |

The Secret Service adapter and native downloads can follow as separate
discussions and PRs. Downloads touch the shared coordinator and vendored Wry,
so agree on that scope before asking the maintainer to review the implementation.

## Details to get right in future descriptions

- The current Arch recipe builds a local checkout. It is not an AUR source
  package or an official Linux release. Its local `pkgrel=2` records this
  checkout's iterations; choose the version and release number for the actual
  upstream submission.
- The recipe installs three PNG icon sizes: 32, 128 and 256 pixels. It also
  installs the desktop entry, the main license, and three runtime legal files.
- Autostart uses `$XDG_CONFIG_HOME/autostart/app.zephium.desktop`, falling back
  to `~/.config/autostart/app.zephium.desktop`. Disabling writes `Hidden=true`
  there so a system entry cannot reappear through fallback. It does not use a
  custom disable flag under `~/.config/zephium`, and it does not expand
  `%ASESSION%`.
- Secret Service needs a persistent default collection. Its native regression
  probe must use its own disposable keyring process, never the user's keyring.
- A passing headless startup probe does not qualify physical input, portal
  shortcut approval, paid providers, extensions or native Work browsing.
- CodeRabbit skips draft reviews by default. A successful skip status is not
  review approval.

Use the repository's PR template and ordinary language. Say what the patch
changes, why it helps, and exactly what was tested. Link the existing Linux
issue without closing it for a partial contribution. Avoid review-time promises
or claims of Windows parity.
