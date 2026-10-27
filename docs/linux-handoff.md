# Linux/Arch port handoff

## Draft #221 linked build qualification, 26 October 2026

The user authorized resuming the #221 qualification. The production Tauri build
completed, producing `/home/rayyan/Zephium/target/release/zephium-desktop`
(39 MiB). The linked binary then passed both isolated startup probes using
disposable profiles:

- X11 under Xvfb initialized its main window and stayed alive for five seconds.
- Wayland under headless Weston at 1024×640 initialized its main window and
  stayed alive for five seconds.

The probe script was `scripts/ci/probe_linux_startup.py` from the local
`test/linux-startup-probes` branch. The probe output reported success and exit
status 0 for each backend. These results qualify basic linked startup and the
narrow Wayland startup case; they do not verify physical compositor behavior,
input, application workflows, or Windows parity.

Draft PR #221 at
https://github.com/zephium-browser/Zephium/pull/221 remains a draft. Its body
now records the production build and both passing probes, with the linked
build/startup checklist item checked. Do not mark the PR ready or open another
PR without the user's request.

The six-minute optimized Rust desktop compile emitted no new errors. Earlier
dead-code/deprecation warnings, including a gated Work credential helper, are
non-fatal and were already recorded on the PR. There was about 25 GiB free on
`/home` during compilation. No package was installed and no user profile data
was changed.

## Paused follow-up, 26 October 2026, 22:37 UTC

The user asked to update this handoff and stop. **Stop here. Do not resume
builds, tests, packaging, application restarts or PR work without a new request.**
CPU limits remain removed for any future authorized build. No new PR was opened
and draft #221 was not updated during this follow-up.

The user's recording at
`/home/rayyan/Videos/Recordings/recording_2026-10-26_19.09.34.mp4`
exposed clipping during live window resizing around 00:50 and misleading CLI
connection status around 03:20. The earlier startup checks did not resize a
window, so they missed the viewport regression. **The release-2 package and
`target/release/zephium-desktop` described below predate these new fixes.**
There has been no new production build or physical desktop verification of them.

### Applied changes

- `desktop/src/platform/linux.rs`: the composition size handler now calls
  `size_allocate` on the privileged WebView after updating its size request.
  The regression reproduced a 600-pixel window retaining a 1200-pixel WebView.
  The native test exercises 1200×800 → 600×480 → 1400×900 → 720×520 and asserts
  the actual child allocation. Its final helper is
  `assert_chrome_viewport_follows_window_shrink_and_growth`, called from the
  existing `linux_composition_transaction_preserves_hidden_state_and_rolls_back_exactly`
  test so all GTK work has one native thread owner.
- `crates/zephium-app/src/work_connections/cli.rs`: account-status discovery
  returns `CliAuth::Unavailable` immediately when the command broker is closed.
  It no longer reports a blocked probe as an authentication failure. The
  existing no-shell/no-gh regression now expects that state and uses portable
  `tempfile::Builder::tempdir()` instead of macOS-only `/private/tmp`.
- `crates/zephium-ipc/src/work_connections.rs`: added the explicit
  `WorkCliStatusV1::Unavailable` wire state.
- `desktop/src/work_connections.rs`: maps the unavailable state before missing
  executable detection. Added
  `closed_cli_broker_is_reported_as_unavailable_not_an_auth_failure`, which checks
  the real listing's serialized status for all four tools. Its test module was
  moved to the end of the file after the passing focused run.
- `frame/src/features/settings/components/connections/CliRow.svelte`,
  `frame/src/features/settings/components/sections/ConnectionsPage.svelte`, and
  `frame/messages/en.json`: show “Unavailable in this build” and explain the
  command-line restriction while keeping the separate MCP server controls.
  Added `frame/src/features/settings/tests/CliRow.component.test.ts`; a genuine
  unknown authentication result retains its own message.
- `frame/src/shared/ipc/bindings.ts`: regenerated through
  `cargo test -p zephium-desktop --lib --locked export_typescript_bindings`.
  Do not hand-edit this file.
- `packaging/arch/PKGBUILD`: local `pkgrel` is now **3**, ready for the next
  authorized build. No release-3 package has been built.
- `docs/linux.md`: added the actual Windows/Linux Connections behavior.

### What Windows actually does

The user explicitly requested Windows parity for Connections. Source inspection
of both the base revision and current implementation shows:

| Area | Windows behavior and Linux target |
| --- | --- |
| GitHub CLI, Git, Codex and Claude Code probes | `cli::ensure_available()` rejects ambient execution unconditionally on every platform, including Windows. The old Windows path also produced `unknown`; this correction exposes the real restriction on all platforms. |
| Typed GitHub CLI broker | `crates/zephium-app/src/work_commands/runner/github.rs::available()` returns false outside qualified macOS. Windows implementation source exists, but Windows runtime qualification is still outstanding. |
| Configured MCP servers | A separate supported service capability. Settings and `work_connections/mcp.rs` use the shared `zephium-mcp` client, HTTP/OAuth and stdio transports. Windows has its process/Job adapter; Linux uses the existing Unix process-group adapter. |
| Server credentials | Shared application namespaces with the Windows vault adapter on Windows and the new Secret Service adapter on Linux. No plaintext fallback. |

This is a source comparison, not a physical Windows runtime test. Do not turn
on generic command execution to create apparent parity. The restriction and
separate MCP service capability are documented in `docs/security-model.md`
under “Work Computer capability gate”. Native Linux Work browsing remains a
separate documented gap.

### Checks and problems encountered

Evidence is retained in `target/linux-handoff/`.

| Check | Result / limitation | Evidence |
| --- | --- | --- |
| Resize before fix | Failed: window 600, WebView 1200 | `native-resize-red.log` |
| Standalone resize regression after fix, X11 | Passed | `native-resize-green.log` |
| Standalone resize regression after fix, Wayland | Passed with an undecorated window matching the app | `native-resize-wayland.log`, `resize-weston.log` |
| CLI listing before/after fix | `unknown` versus `unavailable` failure, then pass | `connections-red.log`, `connections-green.log` |
| Generated bindings | Export test passed | `bindings-connections.log` |
| CLI row browser tests | Initially 1 failed/1 passed, then both passed | `cli-row-red.log`, `cli-row-green.log` |
| CLI guard and recorded status parser | 2 passed, 1 deliberately ignored legacy execution test | `connections-cli-final.log` |
| MCP keychain/HTTP/stdio suites | Passed; nonempty suite counts: 15, 4, 1 | `connections-mcp-parity.log` |
| Frontend typecheck, ESLint, Stylelint, Prettier and Knip | Passed | `resize-connections-frame-check.log` |
| Full frontend unit run | 967 passed, 3 failed: 5-second timeouts in hostile markdown and architecture tests; a UTC date expectation returned Aug 18 in Halifax instead of Aug 19 | `resize-connections-frame-check.log` |
| Frame style boundary | Passed against the existing emitted `frame/dist` output; rerun after a fresh build | `resize-connections-frame-styles.log` |
| Combined native desktop suite before fixture consolidation | 4 passed, 1 failed: GTK initialization on two different libtest threads | `desktop-native-after-lint.log` |

Important verification limits:

- The standalone resize tests passed before their assertions were folded into
  the existing native composition test. **The consolidated final fixture has
  not been rebuilt or rerun.** Even `--test-threads=1` gives separate libtest
  tests different threads, which is why adding a second GTK test broke the
  combined suite. Preserve the single GTK owner when resuming.
- The first standalone Wayland run counted GTK client decorations and failed
  at 1252 versus 1200 pixels. The fixture now disables decorations to match
  `WebviewWindowBuilder::decorations(false)`; that standalone run passed.
- An initial `zephium-app` test command with only `work-runtime` failed to
  compile because its tests also require `work-execution`. The corrected
  feature combination compiled, then exposed `/private/tmp` in the existing
  CLI fixture. After the portable tempdir fix, both active CLI tests passed.
- The frontend timeouts occurred while a Rust test crate was compiling. A
  follow-up run with the CI timezone and no competing compiler is still
  needed; no timeout was raised and those failures have not been dismissed
  as passing. Full workspace Clippy still has the earlier Work/dependency
  warnings; this follow-up has no fresh strict Clippy result.
- Rust formatting ran before the last fixture consolidation/test-module move.
  Run the formatter and verification again when authorized to resume.

### Next steps, only after the user asks to resume

1. Check for live processes and preserve the original dirty checkout. Format
   the final Rust edits, rebuild the desktop test binary from this full-port
   checkout, and run its unit suite and combined native tests. Then rerun the
   consolidated composition/resize test on Wayland. The retained helpers are
   `run-final-desktop-native.py`, `run-resize-test.py` and
   `run-resize-wayland.py` under `target/linux-handoff/`; they resolve the binary
   through Cargo's artifact report. Current expected desktop counts are
   **221 passed, 4 ignored**, with all four native tests run separately by the
   native command. Do not reuse the earlier 4-pass log as a current result.
2. Rerun the complete frontend gate with `TZ=UTC` and without a competing Rust
   compiler. If test contention persists, serialize test files rather than
   weakening assertions/timeouts. Keep the actual pass/fail scope explicit.
3. Run the affected strict Clippy and boundary checks. Keep existing unrelated
   warnings separate; do not suppress them globally or claim a clean workspace.
4. Build release 3 with pinned Rust 1.95.0, Node 24.18.0 and pnpm 11.17.0,
   normal priority/affinity and no build-job caps. Recheck disk space first;
   the last observation had about 4 GiB free on `/home`.
5. Record the new artifact identity and repeat isolated package checks. The
   retained `validate-package.py` now accepts `--release 3` (default is 2) plus
   the explicit expected SHA-256 and optional `--current-session`. It still
   uses release 1 as the upgrade input and never modifies the host pacman DB.
6. Verify the actual rebuilt application under live narrow/wide resizing and
   inspect Connections. The user's current profile must not be reset again
   unless requested. The earlier reset backup is
   `/home/rayyan/.local/share/zephium-reset-backup-20261026-190901`.
7. Update this section and `docs/linux.md` with current results. Keep only draft
   PR #221 submitted until the user explicitly asks for another PR.

## Upstream Linux assessment and contribution value, 26 October 2026

This section records the requested GitHub analysis. It does not resume the
paused implementation work or authorize more submissions. Findings describe
public upstream `main` at commit
[`2c4503c97bd58923ab990d952bd951d93e970ce7`](https://github.com/zephium-browser/Zephium/commit/2c4503c97bd58923ab990d952bd951d93e970ce7).
The official team may have additional work in progress that is not public.

### Maintainer direction and community feedback

The maintainer says official Linux support is actively being developed:
[announcement on issue #137](https://github.com/zephium-browser/Zephium/issues/137#issuecomment-6426215939).
On the closed Linux PR #145, they explicitly said that “smaller pieces on top
of #221 are very welcome.” They closed that version because some native
operations reported success without doing the work. This is an invitation to
contribute focused patches, not approval of our implementation or a completed
review of #221.
[Maintainer comment](https://github.com/zephium-browser/Zephium/pull/145#issuecomment-6426250443).

Another contributor reports using #221 as a starting point for a source build
on Linux Mint 22.3/X11. They identified default-feature compilation failures,
a chrome WebView stuck at 1×1, an older system WebKitGTK version, and NVIDIA
rendering trouble. Their proposed direct GTK allocation fix addresses the same
mechanism as our reproduced resize clipping. Acknowledge this independent work
when submitting the fix; do not present the discovery as exclusive to this
checkout. These are community-reported results, not tests we performed on their
machine.
[Mint testing report](https://github.com/zephium-browser/Zephium/issues/137#issuecomment-6420962751).

The public [README](https://github.com/zephium-browser/Zephium/blob/2c4503c97bd58923ab990d952bd951d93e970ce7/README.md)
still lists Linux as upcoming. The latest published release checked was
[v1.0.3](https://github.com/zephium-browser/Zephium/releases/tag/v1.0.3), with
macOS and Windows application artifacts and no Linux package. Earlier Linux
PRs #144, #145 and #146 were closed without merging. #144 was superseded;
#146's author deferred to the official release effort. Do not describe those
closures as a general refusal of community Linux contributions.

### Existing upstream foundation and our additions

Upstream already has a substantial GTK/WebKitGTK backend, Linux stage/find/
content-filter code, X11 and Wayland shortcut helpers, and Tauri Linux
configuration. Our contribution is completing and qualifying integration around
that foundation. Do not claim to have created the entire Linux backend.

| Area | Missing or incomplete in the inspected public main | What this checkout adds |
| --- | --- | --- |
| Build and startup | `scripts/require-supported-platform.mjs` explicitly rejects Linux; default desktop feature paths still need platform guards. | Experimental source-build gates, shared engine compile/lifetime fixes and an honest Linux guide. Draft #221 contains only this smaller foundation, not all local features. |
| GTK window sizing | `desktop/src/platform/linux.rs` updates the chrome size request without immediately reallocating the child. | Reproduced clipping during shrink, explicit child allocation, and native shrink/grow assertions. The final consolidated fixture remains unverified, as recorded above. |
| Credential storage | `crates/zephium-credentials/src/lib.rs` exposes Windows and macOS backends, with no Linux Secret Service adapter. | Secret Service integration for account tokens, provider keys and MCP credentials; persistent-collection checks, no plaintext fallback and disposable-keyring regressions. |
| Downloads | `crates/zephium-engine/src/host/downloads/` has native macOS and Windows adapters, with no Linux transport. | WebKitGTK transport and shared coordinator integration, publication safeguards, and a fix for repeated cancellation causing a native crash. Resume and initial blank-popup handoff remain gaps. |
| Launcher | Linux shortcut helpers already exist, but `desktop/src/launcher_trigger.rs` rejects editing on Linux and lacks our completed status/recording integration. | Actual registration readback, recording/rebinding, function-key support, and handling for portal denial and stale workers. Real compositor approval still needs qualification. |
| Open at Login | `desktop/src/login_item.rs` handles macOS and Windows. | XDG Autostart with system/user precedence, readback and a persistent per-user Hidden override. |
| Packaging and regression coverage | No Arch recipe in the inspected tree; `.github/workflows/ci.yml` explicitly avoids compiling Linux application code. | Local Arch packaging, isolated installation/upgrade/removal and resource checks, Linux CI, startup probes, and setup/limitation documentation. |

Relevant upstream source references:

- [Build refusal](https://github.com/zephium-browser/Zephium/blob/2c4503c97bd58923ab990d952bd951d93e970ce7/scripts/require-supported-platform.mjs).
- [GTK composition](https://github.com/zephium-browser/Zephium/blob/2c4503c97bd58923ab990d952bd951d93e970ce7/desktop/src/platform/linux.rs#L204).
- [Credential backends](https://github.com/zephium-browser/Zephium/blob/2c4503c97bd58923ab990d952bd951d93e970ce7/crates/zephium-credentials/src/lib.rs).
- [Download adapters](https://github.com/zephium-browser/Zephium/tree/2c4503c97bd58923ab990d952bd951d93e970ce7/crates/zephium-engine/src/host/downloads).
- [Launcher integration](https://github.com/zephium-browser/Zephium/blob/2c4503c97bd58923ab990d952bd951d93e970ce7/desktop/src/launcher_trigger.rs).
- [Login-item implementation](https://github.com/zephium-browser/Zephium/blob/2c4503c97bd58923ab990d952bd951d93e970ce7/desktop/src/login_item.rs).
- [CI policy-only Linux scope](https://github.com/zephium-browser/Zephium/blob/2c4503c97bd58923ab990d952bd951d93e970ce7/.github/workflows/ci.yml#L126).

### Limits and contribution strategy

The strongest contribution is focused Linux integration, debugging and retained
regression evidence. It is not a completed Windows-parity claim. Native
extensions, Ask/Work browser adapters, a qualified command guard, broader
distribution/driver qualification and release maintenance remain substantial
work. The latest resize/Connections edits are not in the built release binary;
the final test consolidation and broader checks still need completion.

Keep the submitted draft separate from the full local port. #221's recorded
engine result is 359 passing tests; the larger local port's earlier 377-test
result does not qualify that smaller branch. #221 still lacks an isolated
verified desktop build/startup and complete lint qualification. The Mint
report supplies concrete cases to address before considering it ready.

Recommended priorities when the user resumes contribution work:

1. Finish qualification of the existing draft #221 itself, using its isolated
   worktree and the reported default-feature compilation failures. Preserve
   the architecture and security floor; do not advertise missing adapters.
2. Offer the GTK sizing fix with the consolidated regression and attribution
   to the independent Mint report. It addresses a visible problem observed
   by more than one contributor.
3. Keep autostart, credentials, packaging and downloads as separate concerns.
   Preserve the existing requested sequence in `linux-contribution-plan.md`,
   accounting for dependencies. Discuss credentials and downloads before
   asking maintainers to review those larger native changes. These priorities
   do not authorize opening additional PRs.

Suggested community wording:

> I’m helping test and complete the existing Linux backend on Arch and Wayland.
> I have focused fixes for startup, window sizing, desktop integration,
> credentials and downloads, and I’d like to contribute them gradually with
> clear tests and limitations.

## Contribution preparation resumed, 26 October 2026

The user asked to continue implementation, preserve the full work in a repo,
and prepare small upstream PRs in order while asking before submitting each.
This supersedes the earlier pause for future contribution work. It does not
authorize opening the next PR yet.

### Repository and branches

- The personal fork already exists at
  [Sipper1236/Zephium](https://github.com/Sipper1236/Zephium).
- The complete local experimental port is now saved there as branch
  `linux-experimental-workbench`, commit `243c3a2`. This is a preservation
  branch, not a PR. It contains 81 changed files and is based on the older
  local commit `0eaad6d79703ca53c1f77201f7a396dad47552ef`; upstream `main` is now
  `2c4503c97bd58923ab990d952bd951d93e970ce7`. Do not submit this large branch
  wholesale or assume it applies cleanly to current main.
- Existing draft PR #221 is refreshed at
  `https://github.com/zephium-browser/Zephium/pull/221` on
  `feat/linux-platform-gating`, head `5c8361fc`. It remains a draft; it was not
  marked ready or submitted for review.
- The branch was rebased on current upstream `main` at
  `2c4503c97bd58923ab990d952bd951d93e970ce7`. A local backup of its prior head
  remains at `backup/pr221-pre-refresh` (`503dda32`).
- The refreshed PR now changes 13 files, with 325 additions and 48 removals.
  It also gates Linux launcher shortcut editing as unsupported and keeps the
  native Ask page reader unavailable, rather than enabling unsupported paths.
- Prepared local PR branch `fix/linux-webview-resize`, commit `cf73e3b`, based
  directly on refreshed upstream `main`. It changes only
  `desktop/src/platform/linux.rs`: immediately allocate the WebKit child to
  match the fixed container, and extend the existing single-owner GTK native
  test to check 1200×800 → 600×480 → 1400×900 → 720×520.
- Prepared local PR branch `test/linux-startup-probes`, commit `6bf4402`, also
  based on refreshed upstream `main`. It adds only
  `scripts/ci/probe_linux_startup.py` and
  `scripts/ci/test_probe_linux_startup.py`. The probe preserves an explicit
  X11 authority path while using disposable profile directories.
- These two preparation branches are local only. No PR was opened and no
  branch was pushed for them. Ask before submitting each one.

### Verification and next order

Checks on refreshed #221:

- Platform guard tests: 4 passed.
- Linux engine suite with `agentic-browser`: 387 passed, 0 failed, 4 ignored.
- `cargo fmt --all -- --check` and `git diff --check` passed.
- `cargo check -p zephium-desktop --lib --locked` passed with 106 non-fatal
  warnings, primarily deprecations and dead code in feature-gated paths.
- The default desktop check first exposed that Work composition and the Ask
  native reader were not Linux-ready, and that the Tauri shortcut plugin is
  absent on Linux. The PR now gates those paths and reports unsupported
  shortcut editing / unavailable Ask commands honestly.
- A linked desktop binary and live X11/Wayland startup were not qualified.
  Only 1.4 GiB remained on `/home` after the shared-target checks, so the larger
  link and startup probe were deferred rather than risking a full filesystem.

The two focused follow-up branches remain unverified during this preparation.
The consolidated native GTK resize regression still needs qualification.
Existing logs from the larger local port do not automatically qualify either
focused branch, especially because they are based on newer upstream main.

Proposed order is: existing draft #221, WebView resize fix, isolated startup
probes, Arch recipe, then XDG Open at Login. The recipe depends on the platform
gates and probe files because its check phase invokes them. Secret Service and
downloads remain later, separate discussions. Before asking about submission,
review the exact branch diff and run only the checks the user authorizes.

## Current state, 26 October 2026, 21:57 UTC

This section supersedes the historical notes below. Keep Linux experimental
and retain the WebKitGTK architecture. The user wants small contributions and
has authorized only draft PR #221 so far; do not open additional PRs automatically.
The local [contribution plan](linux-contribution-plan.md) records the requested
order and the actual dependencies for later PRs.

### Completed final build and package validation

The final rebuild completed with exit status 0 after the desktop lint cleanup.
It used Rust 1.95.0, Node 24.18.0 and pnpm 11.17.0. The user removed CPU limits;
this build ran at normal priority with all eight CPUs available and no
application-specific Cargo, Rayon, CMake or LLVM job caps. Keep that preference
for future commands.

- Package: `packaging/arch/zephium-1.0.3-2-x86_64.pkg.tar.zst` (17,219,666 bytes).
- SHA-256: `4ed3a160d3dabbb22d69ca58431fdbf18991c2c2847812d69ef47deaae30dc36`.
- Packaged and built executable ELF build ID:
  `08f481e96a7f5f71d2a717ebc6b8c5a5282bfaaa`.
- Release executable: 41,012,632 bytes; Rust release build took 11 minutes
  25 seconds. The complete package command took about 12 minutes 32 seconds.
- Command and terminal result: `target/linux-handoff/desktop-cleanup-build-state.json`.
- Build log: `target/linux-handoff/arch-desktop-cleanup-rebuild.log`.
- Checksum receipt: `target/linux-handoff/package-final.sha256`.

The new artifact passed release-1 installation, upgrade to release 2, a 27-file
presence check, all eight resource byte comparisons, executable `.text` and
build-ID comparison, desktop-entry validation, and removal in a disposable
pacman root. Its initialized window remained alive for 10 seconds on each of
X11, a 1024×640 fullscreen Weston compositor, and the current physical Wayland
desktop. The host package database was untouched. Dependency resolution was
skipped and startup used host Arch libraries; this does not qualify a clean
Arch installation, physical input flows or compositor portal approval.

Evidence: `target/linux-handoff/package-final-validation.log` and
`package-final-validation-driver.log`, validation exit status 0. To repeat it:

```sh
python target/linux-handoff/validate-package.py \
  4ed3a160d3dabbb22d69ca58431fdbf18991c2c2847812d69ef47deaae30dc36 \
  --current-session
```

The build reported existing dependency warnings, no debug index for the
stripped executable, and `ryoku: missing required signature` while generating
package metadata. It still exited 0; `.BUILDINFO` contains 1,409 installed-package
records. No host repository or signature policy was changed. This package is
an unsigned local experimental artifact, not an official release.

### Previous package snapshots

The pre-desktop-cleanup package is retained as
`target/linux-handoff/zephium-1.0.3-2-before-desktop-lint.pkg.tar.zst` with SHA-256
`12c8c9b01a88e3531f68bf4599d18007ef8950fe5c917fbf9cf3c062b5baa471`.
Its earlier validation logs are archived with the suffix
`-before-desktop-lint.log`. The still older release-2 package is retained as
`zephium-1.0.3-2-before-final-cleanup.pkg.tar.zst` in that evidence directory.
Release 1 remains only as the isolated upgrade-test input. These historical
artifacts do not identify the final package above.

### Existing draft PR

[PR #221](https://github.com/zephium-browser/Zephium/pull/221) remains a draft.
CodeRabbit explicitly skipped review because the PR is a draft; there are no
human reviews. Its successful status is not a completed code review.
Historical status at 21:57 UTC: for head `503dda32`, both upstream workflow runs reported `action_required`,
not a passing CI result: [CI](https://github.com/zephium-browser/Zephium/actions/runs/40396579119)
and [Components](https://github.com/zephium-browser/Zephium/actions/runs/40396578795).
Its branch is `feat/linux-platform-gating` in `Sipper1236/Zephium`, with local
worktree `/home/rayyan/Zephium-prs/platform-gating`.

- This historical snapshot records the old head `503dda32c1d31fe8833392c53b1c865768cbd0f0`.
- The refreshed PR head is `5c8361fc`; its current results and remaining
  qualifications are recorded in “Contribution preparation resumed” above.
- Packaging, credentials, downloads, autostart and the later full-port lint
  cleanup remain outside this PR. Full-port checks do not qualify the PR branch.

### Latest verified results for the full local port

Evidence lives in `target/linux-handoff/` under the original checkout.

| Check | Result | Evidence |
| --- | --- | --- |
| Engine, `agentic-browser`, Rust 1.95.0 | 377 passed, 6 ignored | `engine-final-tests.log` |
| Final native sandbox/IPC fixture | 1 passed | `native-ipc-final.log` |
| Exact bundled blocker, native compile and cache reload | 1 passed in 43.83 seconds | `native-blocker-final.log` |
| Provider transport suite | 1125 passed | `provider-transport-tests.log` |
| MCP keychain and HTTP integration | 15 library tests and 4 HTTP tests passed | `mcp-keychain-final.log` |
| Credential retry boundary and mutation test | Passed; exact single 40 ms retry retained | `agentic-boundary-final.log`, `vault-guard-tests-final.log` |
| Fork provenance and lock gate | Passed after fetching vendored lockfile dependencies | `fork-locks-retry.log`, `fork-fetch-results.json` |
| Strict Clippy, engine/credentials/account/MCP/agentic, all targets and relevant features | Passed with `-D warnings` | `clippy-native-final-pass.log` |
| Workflow action pins | All 6 workflows passed | `action-pins-final.log` |
| Desktop primary crate, strict Clippy with `--no-deps`, all targets | Passed | `desktop-primary-clippy-fixed.log` |
| Desktop regression suite after lint cleanup | 220 passed, 4 ignored | `desktop-tests-after-lint.log` |
| Native desktop tests after lint cleanup | All 4 passed | `desktop-native-after-lint.log` |
| Final packaged executable: X11, small Weston and physical Hyprland Wayland | Each initialized window stayed alive for 10 seconds | `package-final-validation.log` |

Earlier retained checks also passed: desktop 220, native desktop 4, account 50,
default MCP 5, default agentic 822, credentials 3 and all three isolated native
credential fixtures. Download transport/coordinator, stage and principal-world
native tests passed separately. Cargo-deny 0.20.2, actionlint, formatting,
platform-guard tests (4), startup-probe tests (7), desktop entry validation,
PKGBUILD syntax and the remaining recorded architecture boundaries passed.
See the corresponding logs for the scope of each check.

The resumed whole-graph desktop lint command stopped on **32 warnings in
unchanged `zephium-app` Work code** (`desktop-clippy-resume.log`): native command,
recovery and screenshot helpers lack Linux consumers. No changes were made to
that crate. The separate desktop primary-crate check passed after fixing its
28 warnings (`desktop-primary-clippy-fixed.log`). This is not a clean workspace
Clippy claim. Dependency warnings from Wry, Store and Voice also remain.

The Ruby checker ran with a local extracted Ruby 3.4.10 package under
`target/linux-handoff/ruby-tool/`, verified against the host Arch public
keyring. No host packages were installed. This supersedes the earlier note
that Ruby validation could not run.

The exact bundled blocker seed was regenerated offline into
`target/linux-handoff/blocker-webkit-final.json`. Seed `202610241529` verified
all 3,750,209 source bytes and both Runtime/WebKit goldens. Its native WebKitGTK
compile and cache reload passed in an isolated Xvfb process with disposable
HOME/XDG directories and a private D-Bus. The retained runner is
`target/linux-handoff/run-final-blocker.py`; it resolves the test executable
from a fresh full-port Cargo artifact report. This replaces the missing
historical `/tmp` artifact and native-filter evidence.

### What changed during final verification

- The provider credential loader was renamed to `load_native_credential`; the
  exact source boundary guard now uses that signature. Its single retry and
  mutation tests passed without relaxing the retry policy.
- Native page-reader and recipe helpers now compile in production only where
  their adapters exist. Shared Linux unit tests remain enabled. The existing
  test-only dead-code pattern is scoped to Linux tests for native APIs whose
  consumers are unavailable there; production warnings were not globally
  suppressed.
- The final IPC fixture observes the actual renderer executable, serves an
  HTTP document, checks native-handler invisibility, rejects oversized events,
  and retains seccomp/namespace/NoNewPrivs assertions. It passed after the
  final request-read check was added.
- Download test setup and helper placement were cleaned up for Clippy. The
  full engine suite still contains 377 passing tests.

### Finishing checklist

The local finishing work for the agreed experimental scope is complete. This
maps the earlier numbered resume steps to the retained evidence:

| Resume step | Final result |
| --- | --- |
| 1. Keep one draft PR and resolve its engine failures | Only draft #221 is open; its pinned-toolchain engine suite passes 359 tests. Head and draft state were rechecked. |
| 2. Rebuild and rerun the complete IPC fixture | `run-final-ipc.py` resolved the full-port binary; `native-ipc-final.log` passes with handler isolation, bounded messages and sandbox assertions retained. |
| 3. Rerun affected suites | Engine 377, desktop 220 plus 4 native, account 50, provider transport 1125, MCP keychain 15 plus 4 integration, credential unit 3 plus all 3 isolated fixtures passed. The other 5 native engine fixtures also have retained passing logs. |
| 4. Recheck formatting, lint and security boundaries | Final format, whitespace and binding-drift checks pass (`final-static-results.json`). Policy, fork, retry, action and dependency checks pass as listed above. Changed primary crates pass strict Clippy; whole-graph Clippy is still not clean because of unchanged Work/dependency warnings. |
| 5. Produce the final pinned-toolchain package | Rebuilt after all production source changes; exit 0, final checksum recorded above, no CPU caps. |
| 6. Qualify this package in isolation | Install, upgrade, resources, executable identity, X11/Wayland/physical startup and removal passed for that exact checksum. |
| 7. Refresh the Linux guide and handoff | `docs/linux.md` and this current section record the final evidence and explicit unsupported surfaces. `docs/linux-contribution-plan.md` preserves the small-PR sequence. |

The final evidence assertions are recorded in
`target/linux-handoff/completion-audit.json`; all assertions passed.
The local evidence directory is ignored by Git. Keep it when handing off this
checkout; the documentation alone cannot recreate the observed test results.

### Follow-up qualification and contribution work

The agreed target remains an experimental WebKitGTK port. Native extensions,
Ask/Work browser adapters, download resume/blank-popup handoff and a qualified
Work command guard are still explicit implementation gaps. Input flows,
real-compositor portal approval, KWallet, aarch64 and native Windows/macOS
regressions remain unqualified. These are future work, not capabilities enabled
by this finishing pass. Whole-graph Clippy also remains limited by the existing
Work/dependency warnings described above; do not claim a clean workspace lint.

Leave draft PR #221 as the only submission until the user asks for another.
Its desktop build/startup and lint qualification are separate from the full
local port; this pass does not make that smaller branch ready for review.

Preserve the original dirty worktree on `main` at
`0eaad6d79703ca53c1f77201f7a396dad47552ef`. The PR uses a newer upstream base;
do not copy whole files between the two without comparing their changes.
All build sessions mentioned in the current section have completed. Recheck
processes before starting a new build rather than relying on historical PIDs.

## Historical notes and paths (superseded)

**Snapshot:** 26 October 2026, 15:43 UTC. The full experimental port remains
uncommitted in `/home/rayyan/Zephium` on `main`. **The release-2 package build
finished successfully.** One small draft PR has been submitted from a separate
worktree; the user asked to leave additional PRs for later. Run
`git status --short` before changing anything and preserve all existing work.
Recheck active jobs before using the shared `target/` directory.
The original checkout is still at `0eaad6d79703ca53c1f77201f7a396dad47552ef`;
the PR worktree uses a newer upstream base. No Cargo, rustc or makepkg process
was running when this note was refreshed. The old `/tmp` logs and build helpers
are now missing; see the recovery notes below.

## Submitted draft PR

[PR #221: feat(linux): allow experimental source builds](https://github.com/zephium-browser/Zephium/pull/221)
is open as a **draft**. This is the only PR submitted.
GitHub status was rechecked for its exact head commit: CodeRabbit reports
success, and no human reviews are recorded. That status does not establish that
native builds or the engine test suite are green.

- Worktree: `/home/rayyan/Zephium-prs/platform-gating`.
- Fork: `Sipper1236/Zephium`; branch: `feat/linux-platform-gating`.
- Commit: `9e49956343d1b76e707880ddbd2acda17eeab7bb`, based on upstream
  `aad9ca826a47b3ca3a90830f2e8e603b83713227`.
- Scope: 8 files, 108 additions and 18 deletions. Platform guards, Linux
  onboarding sizing, optional engine compile guards, platform-guard tests and a
  short Linux guide. Packaging, credentials, downloads and autostart remain in
  the full local port for later submissions.
- The existing build script and Tauri configs were already upstream, so they
  were not included. The engine `lib.rs` download changes require the download
  adapter and were also left out.
- Verified on the PR branch: 4 platform-guard tests passed with Node 24.18.0;
  workspace formatting applied with Rust 1.95.0; changed Rust files passed
  `rustfmt --check`; patch whitespace passed.
- Its isolated engine suite with `agentic-browser`, run on Arch with Rust
  1.99.1: **353 passed, 5 failed, 4 ignored**. The failures concern two lifetime
  admission tests, two Linux content-filter source checks, and the review-age
  fixture. The PR description lists the exact names and leaves testing/Clippy
  checklist items open. **Do not attribute the full port's 377-pass result to
  this smaller branch.** Desktop build/startup and native Windows/macOS checks
  have not been completed for the PR branch.

The exact remaining failures on the PR branch are:

```text
agent_context_port::tests::lifetime_factory_is_exclusive_bounded_and_never_reopens_an_old_port
agent_context_port::tests::retired_lifetime_integrity_failure_blocks_its_successor_lineage
platform::linux::content_filter::tests::cache_maintenance_owns_gio_callbacks_cancellation_and_strv
platform::linux::content_filter::tests::persistent_lookup_precedes_save_and_both_results_are_validated
platform::linux::tests::runtime_preconditions_reject_clock_rollback_and_sandbox_overrides
```

Reproduce from `/home/rayyan/Zephium-prs/platform-gating` with
`cargo test -p zephium-engine --features agentic-browser --lib --locked -- --test-threads=1`.
Use the pinned Rust 1.95.0 toolchain for the final checks; the recorded PR test
run used the host's Rust 1.99.1. Resolve failures within this PR's scope before
marking it ready. Do not open the proposed CI, packaging or autostart PRs until
the user asks for the next submission.

The original worktree and its `origin` remote were preserved. Tests on the PR
worktree reused `/home/rayyan/Zephium/target`; debug test binaries there may now
belong to the smaller PR branch. Rebuild from the intended worktree before
running a native fixture by binary path.

## Architecture decision

Keep WebKitGTK for Linux. Do not switch to Chromium or build a replacement
extension engine. WebKitGTK has no native WebExtension runtime; Windows uses a
Chromium extension host and macOS uses WKWebExtension. `crates/zephium-webext`
prepares and verifies extensions but does not execute them on Linux. Linux
WebExtensions, Ask page reading, native Work browsing, and a qualified Work
command sandbox remain explicit gaps, not grounds to bypass security.

## Work in this tree

- **Linux startup and Arch identity:** changed the refusal/platform guards in
  `desktop/src/lib.rs` and `scripts/require-supported-platform.mjs`. The build
  script and Linux/development Tauri configs already exist upstream and are
  reused: `desktop/build.rs`, `desktop/tauri.linux.conf.json`, and
  `desktop/tauri.dev.conf.json`.
- **GTK/WebKitGTK engine adapter:** split-stage composition, lifecycle, profile
  storage, content handling, and Linux agent-context work are in
  `desktop/src/platform/linux.rs` and `crates/zephium-engine/src/platform/linux/`,
  with shared host integration in `crates/zephium-engine/src/host/`,
  `crates/zephium-engine/src/lib.rs`, `crates/zephium-engine/src/platform/mod.rs`,
  and `crates/zephium-engine/src/agent_context_port.rs`.
- **X11/Wayland launcher:** X11 grabs, Wayland GlobalShortcuts portal, focused
  fallback, live registration status, shortcut recording/rebinding and key
  translation touch `desktop/src/linux_shortcut.rs`,
  `desktop/src/linux_x11_shortcut.rs`, `desktop/src/linux_shortcut_portal.rs`,
  `desktop/src/linux_global_shortcuts.rs`, `desktop/src/launcher_trigger.rs`,
  `desktop/src/keymap.rs`, and `desktop/src/lib.rs`.
- **Wayland sizing:** onboarding no longer fixes min/max geometry to a size
  larger than a fullscreen/tiled compositor surface (`desktop/src/lib.rs`).
- **Login startup:** XDG Autostart supports readback and a user-level Hidden
  override in `desktop/src/login_item.rs` and `desktop/src/login_item/linux.rs`.
- **Credentials:** Linux Secret Service adapter is in
  `crates/zephium-credentials/src/linux.rs`, wired through
  `crates/zephium-credentials/src/lib.rs`,
  `crates/zephium-account/src/vault.rs`,
  `crates/zephium-agentic/src/provider_transport.rs`,
  `crates/zephium-agentic/src/provider_transport/lead/keys.rs`, and
  `crates/zephium-mcp/src/keychain.rs`. There is no plaintext-file fallback.
  A desktop Secret Service provider (GNOME Keyring or KWallet integration) is
  required; without one, credential operations fail closed.
- **Downloads:** WebKitGTK native transport and shared coordinator integration
  are in `crates/zephium-engine/src/host/downloads/platform_linux.rs`,
  `crates/zephium-engine/src/host/downloads.rs`,
  `crates/zephium-engine/src/host/download_files.rs`, adjacent `host/` lifecycle,
  stage, profile and dispatch files, and `vendor/wry/src/webkitgtk/` plus
  `vendor/wry/src/lib.rs`. Resume and initial blank-popup handoff remain gaps.
- **Arch packaging:** `packaging/arch/PKGBUILD`,
  `packaging/arch/zephium.desktop`, and `packaging/arch/.gitignore`.
- **Build/docs/CI:** `docs/linux.md`, `docs/adblock.md`, `README.md`,
  `CONTRIBUTING.md`, `.github/workflows/ci.yml`,
  `scripts/ci/probe_linux_startup.py`,
  `scripts/ci/test_probe_linux_startup.py`, and
  `scripts/ci/probe_linux_credentials.py`.
- **Ask page assistant:** remains deliberately unavailable on Linux until its
  native page-reader adapter exists (`desktop/src/page_assist.rs`).

## Latest fixes in this phase

- `crates/zephium-engine/src/host/agent_context.rs`: made the pure origin helper
  available to Linux tests and qualified its argument types. The optional
  `agentic-browser` suite had otherwise failed to compile.
- `crates/zephium-engine/src/agent_context_port.rs`: shared mutation-closure
  checking between reservation and dispatch. A retired/sealed/failed lifetime
  now returns `Rejected(Shutdown)` even on a platform whose live native requests
  return `Unsupported`; unsupported requests still are not scheduled.
- `crates/zephium-engine/src/platform/linux/content_filter.rs`: fixed source-test
  boundaries to locate the test module rather than the earlier test-only import;
  updated assertions for retained cancellables and multiline filter validation.
- `crates/zephium-engine/src/platform/linux/mod.rs`: corrected the review-age
  test to use the core policy's existing grace boundary. The runtime security
  floor and sandbox requirements were not weakened.
- `vendor/wry/src/webkitgtk/web_context.rs`: **fixed a real SIGSEGV** for
  context-only downloads. The context's deny callback and each view callback
  could cancel the same transfer repeatedly. The context now stops further
  `download-started` emission before cancelling once; per-view handlers ignore
  nonmatching/missing source views. Keep this central ownership when rebasing.
  An attempted deferred-cancellation helper was removed; it is not needed.
- `crates/zephium-credentials/src/linux.rs`: reject an item outside the
  persistent default collection, reject a default alias pointing to the session
  collection, and retain duplicate rejection across collections. Apply the
  enumeration limit only to the requested prefix, not unrelated application
  credentials. Added prefix-capacity and temporary-collection regressions.
- `scripts/ci/probe_linux_credentials.py`: verify that Secret Service belongs to
  the probe's disposable keyring process before running native fixtures. After
  roundtrip testing, retire that disposable default collection and verify that
  missing-default operations fail closed. **Never run this mutation against a
  user's keyring.** Merely clearing GNOME's default alias was insufficient:
  GNOME may resolve it back to its login collection.
- `scripts/ci/probe_linux_startup.py` and its tests: preserve explicit Xauthority
  or the original home authority-file path for `--current-session --backend x11`;
  the disposable HOME must not break display authentication.
- `.github/workflows/ci.yml`: added the optional shared engine runtime tests to
  the existing Linux job. Full release/package and real startup checks remain
  manual; Ubuntu's system WebKitGTK may be below the application's security floor.
- `packaging/arch/PKGBUILD`: bumped `pkgrel` to **2** for the finishing fixes.
  The pinned-toolchain release-2 build completed successfully; package runtime
  and install/upgrade/removal qualification are still pending.
- `crates/zephium-engine/src/platform/linux/mod.rs`: the previously failing
  sandbox/IPC fixture now serves its document over loopback HTTP and waits for
  `LoadEvent::Finished` before exercising the page bridge. Its bounded IPC and
  oversized-event rejection checks passed in an isolated native run. The timeout
  was caused by the old `data:` fixture's URI, not evidence of a sandbox failure;
  see the investigation below. Temporary production IPC diagnostics were removed.

## Verified results

### Full port: recorded checks before the final fixture cleanup

- Engine tests with `agentic-browser`: **377 passed, 0 failed, 6 ignored**.
  This suite and `cargo fmt --all -- --check` were rerun successfully after
  switching the IPC fixture to loopback HTTP.
- Secret Service crate tests: **3 passed, 3 native fixtures ignored normally**.
  The isolated probe explicitly passed all three fixtures: no service;
  roundtrip/replacement/exact deletion including session-only and duplicate
  rejection; and missing persistent default collection.
- Credential Clippy passed with **pinned Rust 1.95.0**, all targets, `-D warnings`.
- Startup-probe unit tests: **7 passed**; platform-guard tests: **4 passed**.
- Five of the six ignored native engine tests passed in separate processes,
  each with disposable HOME/XDG data, private D-Bus and Xvfb:
  1. native download transport: async destination, compressed payload,
     cancellation, callback panic, unowned refusal and same-name preservation;
  2. coordinator: GTK save consent, publication/readback and profile drain;
  3. exact bundled EasyList/EasyPrivacy native compile and cache reload;
  4. stage parking and hidden-child visibility;
  5. isolated principal-world handlers.
- The sixth native engine test, sandbox/context/IPC, has now also passed once:
  **1 passed, 0 failed**, with private D-Bus, Xvfb, disposable HOME/XDG directories
  and `setpriv --no-new-privs`. See `/tmp/zephium-native-ipc-http.log`.
  Its final fixture cleanup has since been applied; the cleanup still needs a
  rebuild and native rerun from the full port worktree.
- The blocker artifact was materialized and verified offline:
  `/tmp/opencode/zephium-blocker-webkit.json`, seed `202610241529`, exact Runtime
  and WebKit goldens. That temporary file is now missing. Regenerate it with
  `cargo xtask materialize-blocker-seed-webkit --output target/zephium-blocker-webkit.json`
  from the full port root, then set `ZEPHIUM_BLOCKER_WEBKIT_ARTIFACT` to the
  regenerated file's absolute path for the native filter test.

### Release-2 package

- `packaging/arch/zephium-1.0.3-2-x86_64.pkg.tar.zst` now exists (about 16.4 MiB).
  `makepkg` exited successfully. The pinned Rust 1.95.0 optimized build took
  77 minutes 11 seconds; Node 24.18.0 and pnpm 11.17.0 were used.
- The package is still present. SHA-256 checked at this snapshot:
  `3a8e75a40c5fbd3d3d7bf12d490115a5758600703506be2dbf136392579a5d42`.
- The package's check phase completed successfully. The debug-index utility
  reported that the stripped executable had no debugging symbols; this did not
  fail packaging.
- This artifact has **not** yet had the isolated install, upgrade, removal,
  resource/hash verification or X11/Wayland startup probes repeated. Release-1
  evidence below does not qualify release 2. Do not call the port finished.

### Release-1 package, before the latest fixes

- `packaging/arch/zephium-1.0.3-1-x86_64.pkg.tar.zst` was built successfully
  (about 17 MB). **It is now stale: it predates the credential and download-crash
  fixes above. Do not distribute it as the finished artifact.**
- It installed in a disposable pacman root using `fakeroot`, with 27 package
  files and none missing. Desktop entry, icons and license/resource hashes were
  checked. Its ELF build ID and `.text` matched the build output; whole-file
  hashes differ because `makepkg` strips/adds debug metadata.
- The packaged release executable passed isolated X11 and 1024×640 fullscreen
  Wayland startup probes, remaining initialized/alive for 10 seconds each.
- Pacman removal succeeded in that isolated root. The host package database was
  never modified. This was not a clean-distribution dependency-install test:
  executable startup used the host Arch runtime libraries.

Earlier desktop (215 passed), account (50 passed), MCP (15 passed), and native
desktop Xvfb checks are useful historical results, **not final regression results
for the latest source**. A previous full provider suite had 1123 passes and two
timing-sensitive failures; serial reruns were reported passing, but the full
current provider suite still needs a final run.

## IPC investigation and remaining fixture cleanup

`platform::linux::tests::wry_contexts_enable_sandbox_and_cross_site_process_swap`
previously failed with:

```text
bounded isolated-world IPC did not arrive: receiving on an empty channel
```

The root cause is confirmed in the native callback in
`vendor/wry/src/webkitgtk/mod.rs`: it constructs an `http::Request` with the
WebView's current URI. The `http` crate rejects the fixture's opaque `data:` URI
with `InvalidUri(InvalidFormat)`, so Wry discards the message before calling the
embedder's IPC closure. Base64-encoding the HTML does not fix that URI mismatch.
A standalone check against the checkout's compiled `http` dependency reproduced
the rejection and accepted the loopback HTTP URI. No production IPC or sandbox
policy was loosened to make the fixture pass.

The replacement fixture binds `127.0.0.1` on an ephemeral port, serves its HTML,
waits for the document load, then evaluates the page-world IPC and over-limit
event checks. The native run passed the sandbox, `NoNewPrivs`, seccomp filter
count, namespace separation and bounded-message assertions.

The following test-only cleanup is applied in the full port but **has not been
rebuilt or rerun yet**:

1. Restored the original native-handler visibility assertion. The evaluated
   script reports `'page-native-handler-visible'` if
   `window.webkit?.messageHandlers?.wryIpc` is exposed in the page world; the
   Rust assertion still requires `'bounded-ipc-ok'`.
2. Reused `web_process_descendant` to select the executable in argv[0], instead
   of matching `WebKitWebProcess` anywhere in a command line. Bubblewrap's
   arguments contain that path before confinement is installed, explaining the
   transient `Seccomp: 0` seen during diagnosis. Seccomp and `NoNewPrivs` remain
   independent assertions after selecting the actual renderer.
3. Bounded the loopback server's accept/read/write waits and added a title
   assertion proving the probe runs on the served document.

Rebuild from `/home/rayyan/Zephium` and rerun this test alone; GTK/native
tests use their own processes to avoid cross-test main-thread ownership issues.

```sh
dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 \
  setpriv --no-new-privs \
  target/debug/deps/zephium_engine-266aee4b065219b9 \
  platform::linux::tests::wry_contexts_enable_sandbox_and_cross_site_process_swap \
  --ignored --exact --test-threads=1
```

Also supply disposable HOME and XDG directories, including a mode-0700
`XDG_RUNTIME_DIR`, as in the previous runs. The binary hash above is a snapshot;
locate the current executable with Cargo's `--no-run --message-format=json` if it
changes. Do not run all six GTK tests together on separate libtest threads.

## Logs and process state

The makepkg session `12738` completed with exit status 0, and the isolated PR
engine test session `97406` completed with exit status 101 (the five failures
listed above). These handles are completed, not active jobs. Recheck processes
before starting new work rather than relying on stale process IDs.
No Cargo, rustc or makepkg jobs were running at the latest check.

The frontend build emitted a Svelte `state_referenced_locally` warning in
`frame/src/session/motion.svelte.ts:4` and an unresolved-at-build-time
`/zephium-wordmark.svg` reference. These did not stop the frontend build; review
their runtime effect separately before claiming complete UI qualification.

**The `/tmp` files listed below no longer exist.** These paths identify the
historical runs, not logs available for inspection in a new session. This
includes the blocker artifact, credential Clippy cache, PR description file
and temporary pinned pnpm installation. Recover the PR description from GitHub
and regenerate test logs/artifacts when rerunning checks. Store new evidence in
an ignored workspace directory, such as `target/linux-handoff/`, if it needs to
survive temporary-directory cleanup.

Historical log paths:

- `/tmp/opencode/zephium-final-engine-tests.log` — 377 unit tests passed
- `/tmp/opencode/zephium-final-native-engine-0.log` — native download transport
- `/tmp/opencode/zephium-final-native-engine-1.log` — native coordinator
- `/tmp/opencode/zephium-final-native-engine-2.log` — native blocker compile/reload
- `/tmp/opencode/zephium-final-native-engine-3.log` — native stage
- `/tmp/opencode/zephium-final-native-engine-4.log` — native principal isolation
- `/tmp/opencode/zephium-final-native-engine-5.log` — old IPC fixture failure
- `/tmp/zephium-native-ipc-http.log` — passing loopback HTTP native IPC fixture
- `/tmp/zephium-pr-a-engine-tests.log` — smaller PR branch: 353 passed, 5 failed,
  4 ignored; **not** a regression run of the full local port
- `/tmp/zephium-pr-a-description.md` — exact body used to create draft PR #221
- `/tmp/zephium-native-ipc-rerun.log`, `/tmp/zephium-native-ipc-debug.log`,
  `/tmp/zephium-native-ipc-pageworld.log`, `/tmp/zephium-native-ipc-handler.log`
  — intermediate diagnostics; superseded by the passing HTTP fixture
- `/tmp/opencode/zephium-final-credentials-native.log` — all three native fixtures
- `/tmp/opencode/zephium-final-pinned-credentials-clippy.log` — pinned-toolchain pass
- `/tmp/opencode/zephium-final-blocker-artifact.log` — exact seed verification
- `/tmp/opencode/zephium-download-trace.log` — GDB evidence of the old repeated
  cancellation crash; fixed code subsequently passed its regression
- `/tmp/opencode/zephium-final-format.log` — check output before applying the
  latest formatting; rerun rather than treating it as a final failure
- `/tmp/opencode/zephium-final-wry-format.log` — standalone formatting differences
  in three unrelated existing macOS files under `vendor/wry/src/wkwebview/`;
  they were deliberately not reformatted
- `/tmp/zephium-arch-final-package.log` — successful old release-1 build
- `/tmp/zephium-account-tests.log`, `/tmp/zephium-mcp-vault-tests.log`,
  `/tmp/zephium-agentic-vault-tests.log` — historical downstream suite results

Hermes/DeepSeek timed out and agy/Gemini exhausted its quota; neither completed
its review. Available OpenCode read-only reviewers confirmed the credential and
Xauthority findings, which were fixed and regression-tested. The previous
structured review plan `rvw_866169e7c5e883762ce4339e8b74dc93df5e70be` is stale and
incomplete, not an approval or a stable-release security audit.

## Resume commands and next steps

1. Respect the submission scope: only draft PR #221 is open; the user wants
   additional PRs later. Resolve its remaining failures before marking it ready,
   keeping its test results separate from the full port's results. Preserve the
   stale release-1 package only for the isolated upgrade test.
2. Rebuild the test-only IPC fixture cleanup above, then rerun the native test
   with its full original isolation assertions and disposable test environment.
   These `#[cfg(test)]` edits do not require another production package rebuild.
3. Rerun current desktop, account, MCP and provider suites serially. Avoid
   concurrent heavy builds: they previously caused test timing failures and
   Cargo cache/target lock contention.
4. Recheck formatting, lint and architecture/security gates. Root formatting
   was applied after the last check; standalone Wry has the unrelated macOS
   differences noted above.
5. Release 2 was built with **Node 24.18, pnpm 11.17.0 and pinned Rust 1.95.0**.
   Versions were checked immediately before launch. Available tools for a
   future rebuild:

   ```sh
   /home/rayyan/.cargo/bin/cargo +1.95.0 --version
   mise exec node@24.18.0 -- node --version
   /tmp/zephium-build-tools/node_modules/.bin/pnpm --version
   ```

   Example local package build (dependencies are already installed on this
   machine; `--nodeps` is not an installation guide):

   ```sh
   cd /home/rayyan/Zephium/packaging/arch
   CARGO_BUILD_JOBS=2 RUSTUP_TOOLCHAIN=1.95.0 \
     mise exec node@24.18.0 -- bash -c '
       export PATH="$HOME/.cargo/bin:/tmp/zephium-build-tools/node_modules/.bin:$PATH"
       makepkg --nodeps --noconfirm --noextract
     '
   ```

   The command above records the successful build; its temporary pnpm path no
   longer exists. Restore pnpm 11.17.0 in the chosen build environment before
   reusing it. The separate `/tmp/opencode/zephium-pinned-check` Clippy cache is
   also gone. Check disk space before restarting; this snapshot has about
   17 GB free on `/home` and 7.7 GB on `/tmp`.
6. Validate release 2: isolated pacman install, release-1-to-2 upgrade and removal;
   packaged legal resources/icons/desktop identity; X11 and Wayland startup of
   the **new** executable. Never install/remove packages in the host database
   merely to test the recipe.
7. Update `docs/linux.md` and this note with final evidence. Keep extensions,
   Ask/native Work, browser password/passkey integration, download resume and
   initial blank-popup handoff explicit. Real compositor portal approval,
   KWallet behavior, aarch64 and Windows/macOS native validation remain unverified.
   Do not claim Windows parity or stable-release readiness.
