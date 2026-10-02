# Mark — Desktop Document Signer

**Mark. Sign anything.**

Implementation specification for an AI coding agent.

Build a native desktop application in Rust using GPUI.

The first target is Linux on an Omarchy installation.

The architecture must support macOS and Windows without rewriting the application.

Primary UI stack:

- `gpui-kit`
- `gpui-omarchy`

Repositories:

- https://github.com/longbridge/gpui-kit
- https://github.com/huacnlee/gpui-omarchy

Do not introduce another GUI framework.

---

## 1. Product definition

Build a focused desktop utility for placing visual signatures and stamps onto documents.

The core workflow is:

1. Open an image or PDF.
2. Display the document.
3. Import one or more signature/stamp images.
4. Place each signature/stamp on one or more pages.
5. Move, resize, and rotate them.
6. Navigate between pages.
7. Export the final document.
8. Keep the original document untouched unless the user explicitly chooses overwrite.

The application is NOT initially:

- a full PDF editor
- a PDF form editor
- an OCR application
- a cloud signing service
- a document management system
- a cryptographic certificate/signature application
- an electronic-signature legal-service platform

The first version performs **visual signing**.

A PNG containing a handwritten signature is treated as an image placed onto the document.

Do not claim that this is a cryptographic PDF digital signature.

The internal architecture should leave room for true PDF digital signatures in a future version.

---

## 2. Product philosophy

The application should feel like a small, serious desktop tool.

Do not build a giant toolbar.

Do not copy Adobe Acrobat.

The core interface should answer three questions immediately:

- What document am I editing?
- What page am I on?
- What object am I currently placing/editing?

The main interaction should be direct manipulation.

A user should be able to understand the application without reading documentation.

Keyboard-first interaction is desirable because the application targets Omarchy.

Use the visual language provided by `gpui-omarchy` rather than inventing a separate design system.

---

## 3. Critical architectural principle

Separate the application into four layers.

### Layer A — UI

GPUI / gpui-kit / gpui-omarchy only.

Responsible for:

- windows
- panels
- buttons
- menus
- document canvas
- page thumbnails
- selection handles
- toolbars
- dialogs
- notifications
- keyboard shortcuts
- focus
- pointer interaction

The UI must NOT contain PDF manipulation logic.

### Layer B — document model

Pure Rust domain model.

Responsible for:

- pages
- placed objects
- object transformations
- selection
- document state
- undo/redo
- dirty state
- serialization of editor state if needed

This layer should have no GPUI dependencies.

### Layer C — document engines

Separate integrations for:

- PDF
- raster images

The document model should not know whether a page originated from PDFium or an image file.

### Layer D — persistence/export

Responsible for:

- loading files
- saving/exporting
- temporary files
- application settings
- recent documents
- signature library
- safe overwrite handling

---

## 4. Repository structure

Start with a workspace immediately.

Suggested structure:

```text
mark/
├── Cargo.toml
├── crates/
│   ├── app/            # Layer A: GPUI UI
│   ├── mark-core/      # Layer B: document model (no GPUI, no PDFium)
│   ├── mark-pdf/       # Layer C: pdfium-render integration
│   ├── mark-image/     # Layer C: raster image integration
│   ├── mark-export/    # Layer D: export engine
│   └── platform/       # file dialogs, app dirs, platform specifics
├── script/
│   └── fetch-pdfium.sh # downloads PDFium artifacts per-platform (see §6)
├── resources/
│   ├── icons/
│   └── test-documents/ # fixtures (see §22)
├── docs/
│   └── architecture.md # pinned versions, decisions
├── tests/
├── plan.md
├── documentation.md
└── README.md
```

Do not over-split into dozens of crates. If a simpler start is needed, a single crate with `src/model/`, `src/pdf/`, `src/export/` modules is acceptable — as long as the layer boundaries remain enforced (no GPUI types in the model, no PDFium types in the UI).

---

## 5. UI stack: exact versions and setup

This is the highest-friction area of the bootstrap. Follow it precisely.

### 5.1 Pinned dependency configuration

`gpui-omarchy 0.2.x` requires an **exact** `gpui-kit` pin with **default features disabled**:

```toml
[dependencies]
gpui-kit = { version = "=0.7.0", default-features = false }
gpui-omarchy = "0.2.0"
```

Consequences:

- The styled `gpui-component` facade is NOT enabled. `gpui-omarchy` supplies all presentation on top of `gpui-base`.
- Do NOT enable `gpui-kit` default features "to get more components". That pulls in a second, conflicting styled layer.
- The `=0.7.0` exact pin means gpui-kit and gpui-omarchy upgrade in lockstep. Never bump one without the other. Record the working version triple (`gpui-kit`, `gpui-omarchy`, effective GPUI release) in `docs/architecture.md`.

### 5.2 Application bootstrap

Use the entry points from the gpui-kit recipes:

```rust
gpui_kit::application()
    .with_assets(gpui_kit::assets::Assets)   // Lucide icon set
    .run(|cx| {
        gpui_omarchy::init(cx);              // Omarchy theme layer
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|_| MarkApp::new(cx))
        })
        .expect("Failed to open window");
    });
```

Read `gpui-kit/examples/ai_recipes/README.md` before writing any bootstrap code. It is the tested source of truth for executable-application setup and AI-assisted development acceptance checks. Do not copy old GPUI examples from blog posts or memory — GPUI APIs change.

### 5.3 Agent skills (install before coding)

Install the maintained skills so the agent uses current API patterns:

```bash
npx skills add longbridge/gpui-kit
```

This installs:

- `gpui-kit` — setup, component catalog, GPUI mechanics, coding guides
- `gpui-kit-design-guides` — layout, spacing, hierarchy, overlays, copy

Additionally copy/reference the `omarchy-style` skill from the `gpui-omarchy` repository (`.agents/skills/omarchy-style`).

### 5.4 Theme behavior per platform

`gpui_omarchy::init(cx)` reads the Omarchy theme from:

```text
~/.local/state/omarchy/current/theme/colors.toml
~/.local/state/omarchy/current/theme.name
```

- On Omarchy: follows the system theme, including live theme changes (filesystem watcher).
- On macOS/Windows/other Linux: falls back to **Tokyo Night** automatically. No configuration needed.
- "Follows the Omarchy theme" is a Linux-only feature. Do not build platform-specific theming elsewhere.

### 5.5 Component inventory (Phase 0 deliverable)

`gpui-omarchy` is young (v0.2.0, ~54 components). Before Phase 3, inventory every UI need against its gallery (run `cargo run --example gallery` from a checkout). Components confirmed available: `button`, `panel`, `input`, `select`, `combobox`, `toggle`, `button_group`, `tab_list`, `tooltip`, `sheet`, `popover`, `collapsible`, `toast`, `alert`, `separator`, `scrollbar`, `virtual_list`, `tree`, `resizable`, `dock_area`.

If a component is missing, build it on `gpui-base` primitives in the app crate — do not fork gpui-omarchy and do not enable gpui-kit default features.

---

## 6. PDF engine and PDFium dependency strategy

### 6.1 Engine choice

Use `pdfium-render` (currently 0.9.x) as the primary PDF engine.

Capabilities used:

```text
PDF load (with optional password)
PDF page dimensions
PDF rendering to bitmap
creating image page objects
transforming page objects
saving modified PDF documents
```

Why not `lopdf` as primary: for version 1, the application should not own all PDF rendering and transformation semantics. `lopdf` may be added later as an optional export/recovery engine only when a concrete preservation problem requires it. Do not add it speculatively.

### 6.2 pdfium-render configuration

```toml
pdfium-render = "0.9"        # default features: pdfium_latest, image_latest, thread_safe
image = "0.25"               # pin to match pdfium-render's image_025 feature — avoids compiling `image` twice
```

Notes validated against current pdfium-render:

- Default features include `thread_safe` (all PDFium access serialized behind a mutex) — keep it.
- Since 0.9.0 all pdfium-render object handles are `Send + Sync`.
- `pdfium_latest` currently targets PDFium build 7881.

### 6.3 PDFium binary distribution

`pdfium-render` does not bundle the PDFium binary. Use pre-built artifacts from https://github.com/bblanchon/pdfium-binaries/releases for all three platforms.

Create `script/fetch-pdfium.sh` as a **Phase 0 deliverable**:

- Downloads the correct artifact (linux/macos/windows × arch) into `vendor/pdfium/<target-triple>/`
- Deterministic dev setup on Omarchy (Arch has no reliable system pdfium package)
- Doubles as the release-bundling mechanism
- Documented in README so contributors get identical setups

Runtime binding order (follows `Pdfium::default()` semantics):

```text
1. PDFium library bundled next to the executable
2. fallback: system library
```

Shipping options per platform:

```text
Linux:   bundle libpdfium.so next to binary (or rpath to vendor dir)
macOS:   bundle inside .app, or static-link via PDFIUM_STATIC_LIB_PATH
Windows: ship pdfium.dll next to .exe, or static-link via PDFIUM_STATIC_LIB_PATH
```

Static linking is supported via `PDFIUM_STATIC_LIB_PATH` (with per-target-triple overrides for universal macOS builds). Evaluate static vs dynamic during packaging (Phase 12); keep the choice behind the `PdfEngine` abstraction.

Never make Linux development depend on an obscure system-wide installation.

### 6.4 Threading model: one dedicated PDFium thread

Do NOT call pdfium-render from UI callbacks or scattered background tasks. All PDFium calls are mutex-serialized anyway, so parallelism buys nothing.

Commit to this architecture:

```text
UI thread (GPUI)
   │  request (page, render size)
   ▼
channel
   ▼
single PDFium worker thread
   - owns the Pdfium instance and all documents
   - processes requests sequentially: render page / export / load
   ▼
channel
   ▼
UI thread receives bitmap/result via GPUI background-task mechanism
```

The UI must remain responsive during load/render/export. Show progress indication where useful.

---

## 7. Core domain model

The most important design decision. Format:

```rust
struct Document {
    source: DocumentSource,
    pages: Vec<Page>,
    dirty: bool,
}

struct Page {
    id: PageId,
    width: f32,          // logical page units (see §8)
    height: f32,
    rotation: PageRotation,
    objects: Vec<DocumentObject>,
}

enum DocumentObject {
    Image(ImageObject),
    // Future: Text(TextObject), Shape(ShapeObject),
}

struct ImageObject {
    asset_id: AssetId,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    rotation: f32,
    opacity: f32,
}
```

Coordinates are in **document/page coordinates**, never screen pixels. Zoom may be 50%–400% but document objects must not change.

Support any number of objects per page (`Vec<DocumentObject>`). No `signature1`/`signature2` special cases.

### 7.1 Application / view state separation

```rust
struct AppState {
    active_document: Option<DocumentSession>,
    selected_page: PageId,
    selected_object: Option<ObjectId>,
    tool: Tool,
    view: ViewState,          // zoom, pan — view state, not document state
}

struct DocumentSession {
    document: Document,
    source_path: Option<PathBuf>,
    undo: UndoManager,
    redo: UndoManager,
    dirty: bool,
}
```

`DocumentSession` is GPUI-independent and is the future boundary for multi-document/tabs support.

---

## 8. Coordinate system

One canonical page coordinate system: logical page units where `1 inch = 72 points`. Images are normalized to the same system on import.

UI pipeline:

```text
document coordinate → zoom → pan → screen coordinate
```

Pointer pipeline:

```text
screen coordinate → inverse pan → inverse zoom → document coordinate
```

Centralize in:

```rust
struct ViewTransform {
    zoom: f32,
    pan_x: f32,
    pan_y: f32,
}

fn document_to_screen(...)
fn screen_to_document(...)
```

Do not scatter conversion formulas around the UI.

### 8.1 PDF coordinate mapping (highest bug risk)

PDF origin is bottom-left; screen origin is top-left. Additionally handle:

- page rotation metadata (`/Rotate`)
- crop box vs media box
- zoom and pan

Create a tested module:

```rust
struct PageGeometry {
    width: f32,
    height: f32,
    rotation: Rotation,
}
// plus
PageCoordinateMapper
```

Fixture tests (portrait, landscape, rotated, US Letter, arbitrary sizes, zoom 50/100/300%, panned views) must exist from **Phase 4 onward** — the day PDF rendering lands, not at export time. Round-trip property: `screen → document → screen` returns the same point within float tolerance.

---

## 9. Opening documents and images

### 9.1 File dialogs

Use `rfd` (native dialogs on all three platforms) behind:

```rust
trait FilePicker { ... }
struct NativeFileDialogs;
```

Filters: `pdf, png, jpg/jpeg, webp`.

Notes:

- `rfd` on Wayland goes through xdg-desktop-portal — fine on Omarchy; keep dialogs out of automated tests.
- Do not build a custom fake file browser.

### 9.2 Image documents

Opening an image treats it as a one-page document:

```text
Document
└── Page
    └── base image (page background)
```

The rest of the application then works identically for PDF and image documents.

### 9.3 Signature/stamp image formats

Supported: PNG, JPEG, WebP.

- Decode via the `image` crate, normalize to RGBA internally.
- Alpha transparency is mandatory for handwritten signatures — never paint a background behind a signature.
- **On import into the library, copy the asset into app-managed storage normalized to PNG.** JPEG/WebP sources cannot carry alpha and PDFium cannot embed WebP, so normalization at import keeps export simple and makes the library self-contained (user may delete the original file).

### 9.4 Unsupported/encrypted files

Show useful errors ("The PDF is encrypted and requires a password", "PDFium could not be loaded — check the bundled PDF runtime"), never raw Rust/PDFium errors. Keep technical detail in logs only. Password entry: supported if the pdfium-render flow allows; never persist the password.

---

## 10. Main layout

```text
┌───────────────────────────────────────────────────────────┐
│ Mark — contract.pdf                     Zoom   Export     │
├───────────────┬───────────────────────────────┬───────────┤
│ Page          │                               │ Assets /  │
│ thumbnails    │        DOCUMENT CANVAS        │ Tools     │
│  1            │                               │           │
│  2            │                               │ Signature │
│  3            │                               │ Stamp     │
├───────────────┴───────────────────────────────┴───────────┤
│ Page 3 / 12                                   100%        │
└───────────────────────────────────────────────────────────┘
```

Panels are collapsible; the document dominates the window.

---

## 11. Rendering strategy

Render according to visible size, not maximum quality:

```text
PDF page → PDFium → bitmap at appropriate resolution → GPUI image → canvas
```

If a page occupies ~1000 screen pixels, do not render it at 10,000.

### 11.1 Page render cache

```text
PageRenderCache keyed by: page_id, render_width/scale, rotation
```

While zooming: reuse the existing bitmap for small zoom changes; request a higher-resolution render when significantly zoomed in.

### 11.2 Thumbnails

- ~160–240 px wide, low resolution, generated lazily.
- For a 200-page PDF: load metadata first, render thumbnails progressively, never all pages at full resolution at startup.
- Current-page render takes priority over thumbnail generation.

### 11.3 Large documents

Design for 200+ pages, 100+ MB, scanned documents:

- lazy page rendering
- bounded memory cache
- cheap metadata load
- export as background operation with progress

---

## 12. Signature/stamp asset library

Reusable assets have their own identity and persist across documents:

```rust
struct Asset {
    id: AssetId,
    name: String,
    kind: AssetKind,       // Signature | Stamp | Initials | ...
    image_path: PathBuf,   // inside app data dir
}
```

Document objects reference assets by `AssetId` — image data is not copied into every object.

Storage:

```text
OS application data directory/
└── assets/
    ├── <asset-id>.png     # normalized PNG (see §9.3)
    └── library.json       # asset metadata
```

Local only. No cloud sync, no accounts, no database.

Placement workflow (optimize for speed of first experience):

```text
"+ Add signature" → file picker → asset appears centered on current page,
selected, ready to drag
```

Initial size: ~20–30% of page width, aspect preserved, immediately manipulable. (Configurable later.)

Background removal ("remove white background") is explicitly NOT in the initial critical path. Accept transparent PNGs first; add cleanup later.

---

## 13. Object interaction

When selected, show a subtle outline plus handles for corner resize; rotation optional in v1 (a small rotate handle or toolbar action — not mandatory).

- Default resize: lock aspect ratio. Shift = free resize (or the inverse if it conflicts with platform conventions — pick one and document it).
- Pointer down inside object selects it.
- Pointer movement updates document-space coordinates.
- Pointer release commits **one** undoable command. Never one undo step per mouse movement.
- Lightweight snapping only: page edges, page center, nearby object edges, with visible snap guides. No design-tool snapping engine.

Contextual toolbar when an object is selected:

```text
Signature   − size +   100%   Rotate   Duplicate   Delete
```

Optionally: Bring Forward / Send Backward / Reset Size. Do not add 30 controls.

### 13.1 Multi-page placement

```text
Signature selected on page 1 → "Duplicate to page..." (pick page)
```

Later: "all pages", "pages 2–5". Uses the same asset reference.

### 13.2 Copy/paste and delete

- Ctrl+C / Ctrl+V: pasted object lands offset near the selection, not exactly on top.
- Ctrl+D: duplicate.
- Delete/Backspace: removes the overlay object only — never underlying PDF page content.

---

## 14. Undo/redo — built WITH the first manipulation code

Undo is not a later phase. The command stack exists from the first drag/resize implementation; retrofitting undo after manipulation exists is how this architecture gets ruined.

Command-based:

```rust
trait Command {
    fn apply(&mut self, doc: &mut Document);
    fn undo(&mut self, doc: &mut Document);
}
```

Commands: `AddObject`, `DeleteObject`, `MoveObject`, `ResizeObject`, `RotateObject`, `DuplicateObject`, `MoveObjectToPage`, `DuplicateToPage`.

One user action = one command. One drag = `Undo: Move signature`.

Shortcuts: Ctrl+Z / Ctrl+Shift+Z (and Ctrl+Y).

---

## 15. Keyboard shortcuts

```text
Ctrl+O       open
Ctrl+S       export (see §16)
Ctrl+Z / Ctrl+Shift+Z   undo / redo
Ctrl+C / Ctrl+V / Ctrl+D  copy / paste / duplicate
PageUp       previous page
PageDown     next page
Home         first page
End          last page
+ / - / 0    zoom in / out / fit   (matches gpui-omarchy gallery zoom conventions)
Delete       delete selected object
Escape       clear selection
```

Use platform-appropriate modifiers.

---

## 16. Saving and export

MVP has **Export only** (no separate "Save to output path" semantics — that adds dirty-state ambiguity and can be added later).

- Export always asks for a destination, defaulting to `<original-stem>-signed.<ext>` next to the source. Never silently overwrite the original.
- Dirty state (`clean | dirty | saving | error`) shown in the window title: `contract.pdf *`.
- Closing a dirty document prompts: Save-as / Discard / Cancel.
- Image documents export to PNG (v1); PDF documents export to PDF.

### 16.1 PDF export rules

```text
Original PDF + DocumentObject overlays → PDFium → exported PDF
```

- The signature image becomes a real PDF image page object at the correct position/scale (PDFium `PdfPageImageObject` + transform).
- Preserve the original PDF: text stays selectable, vectors stay vector, structure survives.
- Never export a screenshot of the canvas. Never include selection UI, backgrounds, or transparency artifacts from the editor.
- Export runs on the PDFium worker thread with progress UI.
- All coordinate conversion goes through `PageCoordinateMapper` (§8.1).

---

## 17. Errors, logging, privacy

Structured logging (`tracing`). Never log document contents, signature pixels, or sensitive data. Safe examples:

```text
Opened document: 8 pages
Rendered page: 4
Export completed: 1.8s
```

Application error enum (`thiserror`):

```rust
enum AppError {
    Io(...), Pdf(...), Image(...), Export(...),
    UnsupportedFormat(...), InvalidDocument(...),
}
```

Map to friendly messages; no user-facing raw errors; no stray `unwrap()` as error handling.

Privacy: no network access, no telemetry, no cloud upload, no document data in crash reports. The workflow is entirely local.

---

## 18. Persistence

- Settings + recent documents: small local file(s) in the platform app-config directory (via `directories`). No database.
- `File → Open Recent` after the main editor works.
- Asset library persistence per §12.
- Auto-save/recovery: not in v1; `DocumentSession` is the boundary where it would attach later (`~/.local/state/mark/recovery/`).

---

## 19. Platform strategy

### 19.1 Linux-first

Optimize first for Omarchy / Wayland / Arch. GPUI builds need system prerequisites (clang toolchain, `libxkbcommon`, etc.) — document exact packages in README for other Linux users. Linux-specific code stays out of the domain crates (`mark-core`, `mark-pdf`, `mark-export` remain platform-independent).

### 19.2 macOS and Windows

- Domain crates must compile on all three platforms from Phase 2 onward (CI enforces it — §22).
- `#[cfg(target_os = ...)]` appears only inside `crates/platform/`.
- UI differences (modifers, menus) handled in the app crate.

### 19.3 Packaging

Progressive: `cargo run` → `cargo build --release` → per-platform packaging last. PDFium bundling is explicitly tested during packaging (static vs dynamic per §6.3). Don't solve packaging prematurely, but don't leave it to the end user either.

---

## 20. Testing and CI

### 20.1 Validation commands (run at every milestone)

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo check --all-targets
cargo test --all-targets
```

Plus a real native launch on Omarchy for visual/interaction verification. Automated headless tests do not replace native review of dragging, zoom, focus, dialogs, and thumbnails.

### 20.2 Unit tests (domain, no GPUI)

```text
object create/delete/move/resize/rotate/duplicate
undo/redo of every command
page duplication
coordinate conversion: portrait, landscape, rotated, Letter, arbitrary,
  zoom 50/100/300%, panned; screen→document→screen round-trip
page rotation conversion
```

### 20.3 Export tests (fixtures in `resources/test-documents/`)

```text
1-page portrait, 3-page mixed, landscape, rotated page,
scanned image page, text-heavy page
```

For each: load → add known object → export → reload with pdfium-render → assert page count, page sizes, valid PDF, and (as soon as feasible) object placement at known coordinates. Never rely on "returned Ok". Verification also happens **outside the app** in a second PDF viewer.

### 20.4 Interaction tests

Use gpui-kit's headless UI integration testing (real components in headless windows, driven by synthetic pointer/keyboard) for:

```text
click page, PageDown, Ctrl+Z, Ctrl+Shift+Z, Escape, Delete, dialogs
```

Assert on the document model, not screenshots.

### 20.5 CI (from Phase 2 onward)

GitHub Actions matrix:

```text
ubuntu-latest   → fmt, clippy, check, test, full suite
macos-latest    → check + test (domain crates at minimum)
windows-latest  → check + test (domain crates at minimum)
```

UI-crate compilation on mac/windows can lag if system deps are fiddly — domain crates must not. Keep file dialogs and PDFium loading out of unit tests.

---

## 21. Dependency philosophy

Small set, each earning its place:

```text
gpui-kit =0.7.0 (no default features)
gpui-omarchy 0.2
pdfium-render 0.9
image 0.25          # pinned to match pdfium-render
rfd                 # native file dialogs
serde               # settings/library serialization
thiserror           # error types
tracing             # structured logging
```

Optionally: `uuid` (ids), `directories` (platform dirs).

Before adding any crate: current version? license? all-3-platform support? MSRV? native deps? compatible with the pinned GPUI ecosystem? Do not accumulate libraries. Do not add `lopdf` speculatively.

---

## 22. Anti-patterns (do NOT)

1. Build the UI around a screenshot of the PDF.
2. Store signature positions in screen pixels.
3. Couple UI state to PDFium object handles.
4. Render all PDF pages at maximum resolution.
5. Create one giant `App` struct containing everything.
6. Call PDFium from random UI callbacks (single worker thread — §6.4).
7. Implement only one signature slot.
8. Overwrite the original PDF automatically.
9. Introduce a backend/server, authentication, a database, or an Electron/WebView layer.
10. Enable gpui-kit default features alongside gpui-omarchy.
11. Bump gpui-kit without bumping gpui-omarchy in lockstep.
12. Add AI/cloud features just because this is an AI-assisted project.

Keep this a native Rust application.

---

## 23. Git workflow

Small commits, each building. Suggested sequence:

```text
01 bootstrap-gpui
02 fetch-pdfium-script
03 document-model
04 coordinate-fixtures
05 image-document
06 pdf-document
07 page-viewer
08 page-thumbnails
09 signature-assets
10 object-selection-and-commands   (undo/redo included here)
11 object-transform
12 multi-page-placement
13 pdf-export
14 persistence
15 polish
16 ci-cross-platform
17 packaging
```

---

## 24. Development phases

Implement in this order unless a concrete technical dependency requires adjustment.

### Phase 0 — bootstrap and reconnaissance

- Install agent skills (§5.3), read recipes, READMEs, AGENTS.md files, guides.
- Inventory UI needs against gpui-omarchy gallery (§5.5).
- Write `script/fetch-pdfium.sh`; verify `Pdfium::default()` binds to the fetched library.
- Record exact pinned versions in `docs/architecture.md`.
- Initialize the workspace skeleton.

Deliverables: workspace that builds; PDFium loads; `docs/architecture.md`.

### Phase 1 — GPUI application shell

Window + header + empty workspace using gpui-omarchy components. Launches on Omarchy, follows the theme, resizes, closes cleanly, keyboard focus works. Full validation commands pass (§20.1).

### Phase 2 — document domain model

`Document`, `Page`, `ImageObject`, `Asset`, `DocumentSession`, command/undo framework skeleton. Unit tests. CI matrix live (ubuntu/mac/windows on domain crates).

### Phase 3 — open image

Native file dialog → image load → one-page document → canvas renderer. Open JPG/PNG and see it inside Mark.

### Phase 4 — PDF loading + coordinate fixtures

PDFium worker thread (§6.4). Load PDF, page count, dimensions, render current page. `PageCoordinateMapper` with the full fixture test suite (§8.1) — these tests exist at the END of this phase.

### Phase 5 — page navigation

Thumbnails (lazy, progressive), current-page indication, next/prev/first/last, keyboard navigation, zoom/pan/fit.

### Phase 6 — signature library

Add asset (normalized PNG copy), asset list panel, place asset centered + selected.

### Phase 7 — object manipulation + undo/redo

Select, drag, resize (aspect-locked), delete — each committing proper commands; Ctrl+Z/Ctrl+Shift+Z working from this phase forward. Rotation optional at the end of the phase.

### Phase 8 — multi-object / multi-page workflow

Multiple objects, multiple assets, duplicate, copy/paste, "Duplicate to page...".

### Phase 9 — PDF export (most important milestone)

Export via PDFium image objects on the worker thread. Exported PDF verified in an external viewer with all fixtures (§20.3). Progress UI.

### Phase 10 — persistence and polish

Dirty state + close confirmation, export dialogs with `-signed` default name, recent files, asset library persistence, error surfaces, toasts.

### Phase 11 — cross-platform hardening

Compile and run on macOS and Windows. Platform code confined to `crates/platform/`. PDFium bundling validated per platform.

### Phase 12 — packaging

Release builds + PDFium distribution per platform (§19.3).

---

## 25. Definition of done for version 1

A user can do this without friction:

```text
Open a 10-page PDF
→ see thumbnails
→ go to page 3
→ insert signature
→ resize it
→ move it to the signature area
→ go to page 8
→ insert company stamp
→ go to page 10
→ insert two different signatures
→ undo one, redo it
→ export signed PDF
→ open exported PDF externally
→ all signatures/stamps present in the correct places
```

And the same loop works for a simple image document.

---

## 26. Explicitly NOT in MVP

Text insertion, checkboxes, radio buttons, date fields, typed/drawn signatures, signature cleanup (background removal), page rotation/reorder/delete/extract, merge/split, watermarks, highlighting, freehand drawing, true cryptographic PDF digital signatures, batch signing.

The quality bar is not "many features". The quality bar is:

```text
Open → place → adjust → repeat → export
```

being extremely reliable. Build that loop first.

---

## 27. Top risks and mitigations

1. **PDF coordinate/rotation math at export** → early fixture suite (Phase 4) + external-viewer verification of every export test.
2. **`gpui-kit =0.7.0` lockstep with gpui-omarchy** → never bump independently; record versions in `docs/architecture.md`.
3. **Agent fighting GPUI API drift** → installed skills + AI recipes are the source of truth; Rule: read current source before using any GPUI API; never "fix" version conflicts by adding duplicate GPUI versions.
