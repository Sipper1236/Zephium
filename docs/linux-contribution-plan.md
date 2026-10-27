# Bringing Linux support upstream

Only [draft PR #221](https://github.com/zephium-browser/Zephium/pull/221) is open.
Further submissions need the user's go-ahead. Keep each PR focused on one
change and describe the behavior and evidence on that branch.

The refreshed draft allows experimental Linux source builds and keeps native
features without Linux implementations unavailable. Its refreshed engine suite
passes 387 tests with Rust 1.95.0. The larger local port has additional tests
and features, so its 377-test result must stay separate from the PR's results.
The linked production Tauri build has now completed, and isolated X11/Xvfb and
Wayland/Weston startup probes passed. The draft remains a draft until the user
decides whether it is ready for review.

## Preliminary contribution order

The current public upstream base is `2c4503c97bd58923ab990d952bd951d93e970ce7`.
Keep each contribution on its own branch. Ask before opening or publishing each
PR. The order below reflects the existing draft and the new local preparation:

| Step | Scope | Before submitting |
| --- | --- | --- |
| A: existing draft #221 | Experimental platform gates, Linux feature guards and shared engine fixes. | Refreshed to current `main`, head `5c8361fc`. Engine, platform-guard, format and desktop compile checks pass. Linked production build passed; X11 and 1024×640 Wayland startup probes passed. Ask before marking ready. |
| B: WebView resize | `desktop/src/platform/linux.rs`; direct child allocation plus shrink/grow regression in the existing GTK native test. | Local branch `fix/linux-webview-resize`, commit `cf73e3b`. The consolidated native fixture has not been rerun. |
| C: startup probes | `scripts/ci/probe_linux_startup.py` and its tests. Preserve X11 authority when using a disposable HOME. | Local branch `test/linux-startup-probes`, commit `6bf4402`. These are new files upstream. Describe this as adding isolated startup probes, not fixing an existing probe. |
| D: Arch recipe | `packaging/arch/PKGBUILD`, desktop entry and packaging ignore rules. | Depends on the platform gate and startup probe files because the recipe's check phase invokes them. Set version and release against the chosen upstream base. |
| E: Open at Login | `desktop/src/login_item.rs` and `desktop/src/login_item/linux.rs`. | Explain XDG precedence, status readback and the user disable override. Keep engine and credential changes out of this patch. |

The Secret Service adapter and native downloads can follow as separate
discussions and PRs. Downloads touch the shared coordinator and vendored Wry,
so agree on that scope before asking the maintainer to review the implementation.

The complete experimental port is preserved in the personal fork branch
`Sipper1236/Zephium:linux-experimental-workbench` for reference. It is based on
the earlier local base `0eaad6d79703ca53c1f77201f7a396dad47552ef`, contains many
unqualified changes, and is not a submission branch. Do not open it as a PR or
merge it wholesale. The small PR branches above use the refreshed upstream base.

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
