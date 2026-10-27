# WebKitGTK API notes for Linux feature adapters

This note records what the current Linux dependency can support through public WebKitGTK APIs. It is based on the project's pinned `webkit2gtk = 2.0.2` Rust crate with the `v2_40` feature, its generated bindings in Cargo's registry, and the WebKitGTK reference manual. The application currently requires WebKitGTK 2.54 or newer at runtime, so the APIs below that were introduced before 2.40 are within that runtime floor. The reference pages linked here are generated from the upstream WebKitGTK API; the stable site currently labels the reference as WebKit 2.42.5, so use the individual `since` annotations and the local binding source for exact availability.

## Favicon observation

WebKitGTK offers a native favicon path: `WebKitWebView` has a read-only `favicon` property, emits `notify::favicon`, and returns a Cairo surface through `webkit_web_view_get_favicon()`. These APIs are available since WebKitGTK 2.8. The `webkit2gtk` 2.0.2 binding exposes `WebViewExt::favicon()` and `connect_favicon_notify`; it returns an owned Cairo reference that can be copied/rasterized by the host. See the [WebView favicon property](https://webkitgtk.org/reference/webkit2gtk/stable/property.WebView.favicon.html) and [get_favicon documentation](https://webkitgtk.org/reference/webkit2gtk/2.40.2/method.WebView.get_favicon.html).

This is a better Linux adapter seam than observing the DOM for favicon links. The native property reports the favicon WebKit associates with that view and avoids running page-world JavaScript. The adapter still needs to bind the value to the view's current navigation epoch, reject stale notify callbacks after navigation/destruction, bound raster dimensions and output bytes, and avoid persisting private-profile data. The Cairo surface is owned by the view; copy pixels while the native object is valid and impose the bound before allocating the destination raster.

## Media observation

WebKitGTK exposes `WebView:is-playing-audio`, `is_playing_audio()`, and `notify::is-playing-audio` since 2.8. This provides a native signal that a page is producing audio and is suitable for driving a Linux media-observation lifecycle. See [is_playing_audio](https://webkitgtk.org/reference/webkit2gtk/2.40.2/method.WebView.is_playing_audio.html) and the [WebView class reference](https://webkitgtk.org/reference/webkit2gtk/stable/class.WebView.html).

That API does not expose the active media element, title/artist, duration, playback position/rate, seeking, per-element mute state, or picture-in-picture actions. It is therefore not a drop-in implementation of the project's complete media snapshot and control contract. The native property can gate a narrowly scoped adapter that gathers remaining details, but gathering them with JavaScript would still require strict document identity, an isolated content world, lifecycle cancellation, bounded outputs, and explicit tests. The native API alone cannot provide those details.

## Browsing data

`WebKitWebsiteDataManager` provides async `fetch`, `remove`, and `clear` operations, plus a `WebsiteData` result that exposes a name, data-type flags, and size. The manager belongs to a WebKit context and therefore naturally scopes operations to that context/profile. These methods are available since 2.16. The pinned Rust binding exposes `WebsiteDataManagerExt::fetch` and related async operations behind the `v2_16` Cargo feature; because the crate enables `v2_40`, these bindings are compiled. See the [WebsiteDataManager reference](https://webkitgtk.org/reference/webkit2gtk/2.39.2/class.WebsiteDataManager.html), [clear method](https://webkitgtk.org/reference/webkit2gtk/stable/method.WebsiteDataManager.clear.html), and [older stable manual's async/remove details](https://webkitgtk.org/reference/webkit2gtk/2.28.4/WebKitWebsiteDataManager.html).

The native API has two important limits for this product's contract:

* `clear(types, timespan)` can clear selected data types by age, but WebKit documents that nonzero timespans currently do not delete stored cookies. A success callback must not be represented as a complete cookie clear for a date range.
* `WebsiteData` exposes name, types, and size, not per-site modification timestamps. The manager can fetch then remove selected site records, but cannot implement an exact per-site date-range listing/removal from those records alone.

Implement only operations whose semantics can be represented faithfully. A full-profile clear with `timespan = 0` and carefully selected types may be supportable, with async completion and error propagation. Site-specific listing/removal can be explored using `fetch`/`remove` and the data manager belonging to the exact profile context, but host-side history is separate data and needs its own adapter. Keep ephemeral partitions rejected and refuse work while the profile is being erased. Do not report success for unsupported ranges or data classes.

## Popup policy and source identity

WebKitGTK's `WebView::create` signal supplies a `NavigationAction` and expects a newly created view related to the opener; the view should only become visible after `ready-to-show`. `NavigationAction::is_user_gesture()` reports whether the navigation came from a user gesture. `NavigationAction` also reports the target frame name; `NavigationPolicyDecision` has frame-name access. These interfaces are available well before the project's 2.54 runtime floor. See [WebView::create](https://webkitgtk.org/reference/webkit2gtk/stable/signal.WebView.create.html), [NavigationAction](https://webkitgtk.org/reference/webkit2gtk/2.42.4/struct.NavigationAction.html), and the [WebView class reference](https://webkitgtk.org/reference/webkit2gtk/stable/class.WebView.html).

The public UI-process action does not identify the initiating frame's committed URL/origin. A `user_gesture` boolean plus the opener view's main-frame URL is insufficient to prove that an iframe at another origin initiated the popup. The frame name identifies a target, not a source origin. Therefore, an allowlist keyed to a committed page origin cannot safely be implemented for all popup requests from these APIs alone. Do not trust the top-level URL as the initiator when frames can trigger windows. Keep the automatic-popup path denied on Linux unless a supported WebKit extension/process API supplies authenticated initiating-frame identity, or the product policy is redesigned to avoid making that origin-level claim.

Even for verified user-initiated popups, supporting the `create` lifecycle requires integrating the returned related WebView with Zephium's profile, view ownership, native parent/window composition, readiness, and close lifecycle. Returning an untracked child or immediately displaying it would violate the documented lifecycle.

## Practical next steps

1. Implement native favicon observation from `notify::favicon`, with navigation-generation checks and bounded Cairo rasterization.
2. Add Linux audio-state observation using `notify::is-playing-audio`; document which detailed controls remain unavailable unless a separately reviewed adapter can satisfy the existing contract.
3. Implement profile-scoped data operations only after mapping the application's clear/list/forget semantics to WebKit's exact type, age, and per-site limitations. Add tests for cookies with time ranges, profile isolation, async errors, and erasure races.
4. Preserve deny-by-default popup behavior until initiating-frame identity and full related-view lifecycle are available.

## Integration status (27 October 2026)

The Linux port now uses the `zephium-observation` JavaScript world through
WebKitGTK's asynchronous, world-aware evaluation API for favicon discovery and
rasterization. Native X11 and headless Wayland probes confirmed that it can
inspect the shared document while the main page world cannot read its state.
Rust rejects returned strings above the shared 256 KiB limit, and the favicon
script bounds its candidate list and URL fields before serialization.

`crates/zephium-engine/src/platform/linux/browsing_data.rs` uses the exact live
profile manager. A native probe on WebKitGTK 2.54.2 found that data for
`alpha.example.test` and `beta.example.test` appeared in one `example.test`
record. An exact subdomain removal cannot safely remove that entire record or
claim success when no exact record is found. The adapter now refuses a narrow
request when an ancestor record could contain sibling data.

The same runtime can create an origin-private filesystem entry when its
experimental Storage and File System features are enabled. Clearing the old
GTK website-data flags removed a test cookie but left that entry intact.
`OFFLINE_APPLICATION_CACHE` is not the filesystem-data flag. WebKit introduced
the separate [filesystem-data API in 2.56](https://github.com/WebKit/WebKit/commit/bc363a82cf22f21fbe5472135e5bafc9b67b0bb3).
On older runtimes the adapter marks listings partial and refuses site-data
removal before changing data. The 2.56 path passes the new flag through the
older Rust binding, but still needs testing on a 2.56 runtime. Date-limited
site-data clears are refused because WebKitGTK cannot clear cookies by age.
The X11 and Wayland native fixtures confirm the 2.54 refusal and retain the
seeded local storage. A profile with no live manager returns unavailable.

Detailed media controls and origin-authenticated popup policy remain open.
WebKitGTK's audio-state property does not provide per-element actions, and its
popup request does not provide the initiating frame's origin.
