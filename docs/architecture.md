# Mark — Architecture

Authoritative record of pinned versions, structural decisions, and their
sources. Strategic intent lives in `plan.md`; the running log of work lives
in `documentation.md`. When any version below changes, update this file in
the same commit.

## Pinned dependency triple (verified 2026-10-03)

| Crate          | Version | Notes                                                       |
| -------------- | ------- | ----------------------------------------------------------- |
| `gpui-kit`     | `=0.7.0` | exact pin forced by gpui-omarchy; `default-features = false`, `features = ["assets"]` |
| `gpui-omarchy` | `0.2.0`  | presentation layer; upgrades only in lockstep with gpui-kit |
| `gpui-pre` (effective GPUI) | `0.3.7` | resolved transitively via gpui-kit; do not add a direct GPUI dependency |
| `pdfium-render`| `0.9.4`  | default features: `pdfium_latest` (build 7881), `image_025`, `thread_safe` |
| `image`        | `0.25`   | pinned to match pdfium-render's `image_025` — one `image` compile |
| PDFium runtime | build `7881` | from `bblanchon/pdfium-binaries`, tag `chromium/7881`, via `script/fetch-pdfium.sh` into `vendor/pdfium/<platform>/` |

### Lockstep rule

`gpui-omarchy` declares `gpui-kit = { version = "=0.7.0", default-features =
false, features = ["assets"] }`. Never bump `gpui-kit` alone; wait for a
`gpui-omarchy` release that moves to the next exact pin, then bump both in
one commit and update this table.

### Feature-flag policy

- gpui-kit **default features stay off**: the styled `gpui-component`
  facade is never enabled alongside gpui-omarchy. gpui-omarchy supplies all
  presentation on top of `gpui-base`.
- `assets` (Lucide SVG icon set) is required — gpui-omarchy components read
  icons through `gpui_kit::assets::Assets`, registered via
  `.with_assets(...)` at startup.

## Application bootstrap (tested consumer recipe)

Source of truth: `gpui-kit/examples/ai_recipes/src/bootstrap.rs` and
`gpui-omarchy/examples/hello.rs`.

```rust
gpui_kit::application()
    .with_assets(gpui_kit::assets::Assets)
    .run(|cx| {
        gpui_omarchy::init(cx); // base init + omarchy components + system theme
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|_| MarkApp)
        })
        .expect("failed to open window");
        cx.activate(true);
    });
```

Decisions:

- `gpui_omarchy::init(cx)` already calls `gpui_kit::base::init(cx)` plus
  focus/popover/button-group/date-picker init and the Omarchy theme. Do not
  also call `gpui_kit::init`.
- Open windows with `gpui_kit::open_window` (not raw `cx.open_window`) so
  the base `Root` is mounted — required later for sheets, popovers, dialogs,
  and notifications.
- Theme: follows the Omarchy system theme on Linux (live updates via
  filesystem watcher); falls back to Tokyo Night on macOS/Windows. No
  cross-platform theming code of our own.

## Threading model

All PDFium access goes through **one dedicated worker thread** (plan.md
§6.4). pdfium-render's default `thread_safe` feature serializes calls
behind a mutex; a single worker makes that serialization explicit and keeps
the UI thread free. UI → channel → worker → channel → UI. No PDFium types
cross into UI code (they stay inside `mark-pdf`).

## Crate boundaries

| Crate         | Layer | May depend on                    | Never contains            |
| ------------- | ----- | -------------------------------- | ------------------------- |
| `mark-app`    | A — UI | gpui-kit, gpui-omarchy, all internal crates | PDFium calls, business logic |
| `mark-core`   | B — model | (nothing yet)                | GPUI types, PDFium types  |
| `mark-pdf`    | C — engine | pdfium-render, mark-core    | GPUI types                |
| `mark-image`  | C — engine | image, mark-core            | GPUI types, PDFium types  |
| `mark-export` | D — export | mark-core, engines          | GPUI types                |
| `platform`    | integration | rfd (later), etc.          | anything but platform glue; only crate with `#[cfg(target_os = ...)]` |

## PDFium runtime distribution

- Development: `script/fetch-pdfium.sh` downloads
  `pdfium-<platform>.tgz` from `bblanchon/pdfium-binaries` release
  `chromium/<PDFIUM_VERSION>` into `vendor/pdfium/<platform>/`
  (gitignored). `PDFIUM_VERSION` / `PDFIUM_PLATFORM` env vars override.
- Binding: `Pdfium::bind_to_library` against the vendor dir, falling back
  to `bind_to_system_library` (mirrors `Pdfium::default()` semantics).
  Verified by `cargo run -p mark-pdf --example check_bind`.
- Release bundling: dynamic library next to the executable (all platforms)
  or static linking via `PDFIUM_STATIC_LIB_PATH` — decided in Phase 12.

## System prerequisites (Linux/Omarchy)

`clang`, `pkg-config`, `libxkbcommon`, Wayland/EGL dev packages. On Arch:
`sudo pacman -S rust clang pkg-config libxkbcommon wayland-protocols mesa`.

## Agent resources

Installed in this repository (`.agents/skills/`, `.claude/skills/`):

- `gpui-kit` — setup, component catalog, GPUI mechanics (from
  `npx skills add longbridge/gpui-kit`)
- `gpui-kit-design-guides` — layout/spacing/hierarchy guides
- `omarchy-style` — Omarchy visual conventions (copied from the
  gpui-omarchy repository)

Rules for agents: read these skills and the current upstream source before
using any GPUI API; never add a second GPUI version to the graph; prefer
tested recipes over remembered patterns.
