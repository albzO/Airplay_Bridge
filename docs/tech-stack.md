# Technical Route and Choices

**English** · [简体中文](tech-stack.zh-CN.md)

This page records why the desktop UI changed technology three times, what 2.0.0 is built on, what the project implements itself and the known trade-offs. Module ownership is in [Architecture](architecture.md) (Chinese); usage is in [UI and settings](ui.md) (Chinese).

## Route

| Version | UI technology | Notes |
|---|---|---|
| 1.0.x | Vue 3 + Vite on a Tauri 2 WebView | `airplay-frontend/src/`, `src-tauri/`. The UI reaches the Rust backend over IPC and needs WebView2. |
| 1.1.0 (release candidate, never published) | GPUI `0.2.2` + gpui-component `0.5.1` | `airplay-frontend/src-gpui/`. No WebView; the UI calls the desktop services in-process. Feature-complete, but its look and interaction fell short of release quality. |
| 2.0.0 (release candidate) | GPUI `gpui-pre 0.3.11` + Ely GPUI Components | `airplay-frontend/src-ely/`. A new UI on top of the services split out for 1.1.0. |

All three share the Rust desktop services (`airplay-desktop`), the audio core (`airplay-core`) and the native backend (`airplay-backend`). Changing the UI layer does not change the audio pipeline, the protocol, the settings format or the data directory.

## Why GPUI

- **No WebView2**: GPUI draws with Win32 windows, DirectX and DirectWrite, so a clean Windows install needs no extra runtime.
- **Direct, typed calls**: the UI owns the `Engine` and runs `DesktopJob`s on worker threads, with no JSON IPC or event-name contracts; the compiler checks the types.
- **All Rust**: the toolchain is Rust MSVC plus MSYS2 for the native backend; Node, pnpm and a front-end build are no longer needed.

## Why move from gpui-component to Ely

1.1.0 implemented every feature with gpui-component, but its layout, hierarchy and motion did not meet the bar for release. 2.0.0 uses [Ely GPUI Components](https://elygpui.com):

- **A fuller design base**: theme tokens (colour, radius, type size, shadow, density), light and dark modes, accessibility roles and a reduced-motion switch come from the library.
- **The components needed**: switches, sliders, segmented controls, number inputs, password fields, tabs, collapsibles, skeletons, spinners, tooltips, banners, window buttons and a title-bar drag region.
- **Motion parts**: animated collapsible height and fade / rise transitions, all timed by the theme.
- **Licences**: Ely is MIT; its bundled fonts are SIL OFL 1.1 and its icons ISC, see [THIRD_PARTY.md](../THIRD_PARTY.md).

## The key constraint: a separate crate

Ely depends on `gpui-pre 0.3.11` (Zed's pre-release GPUI), whose types are incompatible with the crates.io `gpui 0.2.2` used by 1.1.0, so components cannot be mixed. The new UI is therefore its own crate, `airplay-frontend/src-ely`, rather than a control-by-control replacement inside `src-gpui`.

Modules that do not depend on gpui are shared as one source: `src-ely` includes `session.rs`, `inbox.rs`, `diagnostics.rs`, `volume.rs` and `desktop_job.rs` from `src-gpui/src/` with `#[path]`. Move them into `src-ely` before deleting `src-gpui`.

The executable keeps the name `airplay-bridge-gpui.exe`; the desktop services treat that name as the preview build, with a startup entry and single-instance signal separate from the installed app.

## Pinned versions

| Dependency | How it is pinned | Why |
|---|---|---|
| Ely GPUI Components | Git dependency at commit `69a62b1` | Marked early upstream; no stable crate yet and the API still moves |
| gpui-pre / gpui-pre-platform | `=0.3.11` | The version Ely's pinned commit uses |
| Rust | 1.98 or later | Ely's minimum; verified locally with 1.99 |

Only `ely-core` is used (theme, primitives, forms, layout, motion, window shell). Ely's editor, terminal, chart and other vertical crates are left out to keep compile time and binary size down.

## What the project implements itself

Where Ely does not fit or cannot be customised, `src-ely` has its own code:

| Module | Why |
|---|---|
| `panel.rs` grouped cards | Ely's Card uses an 8 px radius and 20 px inset, which does not fit the 8 px grid with titles outside the card and rows centred vertically |
| `elastic.rs` smooth and elastic scrolling | gpui jumps a whole wheel notch at once and neither Ely nor gpui bounces at the edges; a critically damped spring now follows the target and stretches against resistance at a limit |
| `source_picker.rs` audio source picker | Ely Select scrolls its list internally, so it cannot use smooth scrolling; the picker still reuses Ely's field box, popup surface and option rows |
| `visibility.rs` window visibility | Pauses idle capture when the window is in the tray, minimised or the display is off; the display state comes from a Windows power-setting notification |
| `lang.rs` Chinese and English | Strings are written in pairs where they are used (`t("中文", "English")`) instead of Ely's message catalogues, so neither language is forgotten when wording changes; Ely's own component text follows the same language |
| Title bar | The system title bar is hidden; Ely's drag region and Windows caption buttons are drawn inside the window |

## Service changes that came with 2.0.0

- **Keep awake only while streaming**: the established phase of a session is the test; stopping, failing, quitting or a cleanup timeout releases the request. See [display-off capture](display-off-capture.md) (Chinese).
- **Discovery is admitted on its own**: it excludes only starting a stream and can run while settings are saved or capture is switched, so the window no longer locks as a whole.

Both live in `airplay-desktop`, so the older GPUI and WebView UIs get them too.

## Known trade-offs and to-dos

- The custom picker has no arrow-key selection, Enter to confirm or Escape to close.
- With scrolling handled by the view, moving focus with Tab to an off-screen control does not scroll it into view.
- Motion runs at about 6 ms per frame in debug and release builds, verified on one display only.
- The packaging scripts build `src-ely`; `build-gpui.ps1 -Legacy` still builds `src-gpui`. Ely's MIT license and the Lucide, Inter, JetBrains Mono and IBM Plex Sans licenses ship in `licenses/ely/`. User-facing versions drop a zero patch (2.0.0 is shown as 2.0); Cargo and the numeric Windows resource version keep all parts.
- Streaming, passwords, volume and stereo swapping against real receivers are verified on hardware by the maintainer.
