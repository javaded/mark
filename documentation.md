# Mark — Development Documentation

**Mark. Sign anything.**

This is the living development log for the Mark project. Every agent (AI or human) working on this repository appends an entry for every meaningful action. It is the audit trail of decisions, changes, and verification.

---

## How to use this file

1. Append entries at the top of the `## Entries` section (newest first).
2. Never delete or edit previous entries. Corrections get a new entry.
3. Entry format:

```markdown
### YYYY-MM-DD — <short title>

**Context:** why this work happened.
**Actions:** what was done (files created/changed, commands run).
**Decisions:** choices made and why (link plan.md sections where relevant).
**Verification:** how correctness was checked (commands + results, manual testing).
**Next:** what the next session should pick up.
```

4. Strategic/product decisions live in `plan.md`. This file records what was actually done, when, and how it was verified.
5. If an action contradicts `plan.md`, flag it explicitly in the entry.

---

## Entries

### 2026-10-03 — Project founding: plan review, naming, planning documents

**Context:**

User wants a native desktop app ("Mark") for visually signing/stamping PDFs (multi-page) and images, with multiple reusable signatures/stamps. Linux/Omarchy first, macOS and Windows later. UI stack: `gpui-kit` + `gpui-omarchy`. An initial 78-section implementation plan from a previous agent was reviewed and refined.

**Actions:**

- Reviewed the draft plan against the current state of all three upstream repositories (fetched 2026-10-03):
  - https://github.com/longbridge/gpui-kit — v0.7.x, pins GPUI and re-exports it; provides AI recipes (`examples/ai_recipes/README.md`) and agent skills (`npx skills add longbridge/gpui-kit`).
  - https://github.com/huacnlee/gpui-omarchy — v0.2.0, ~54 components, requires `gpui-kit = { version = "=0.7.0", default-features = false }`; theme loader reads `~/.local/state/omarchy/current/theme/colors.toml`, falls back to Tokyo Night on non-Omarchy systems.
  - https://github.com/ajrcarey/pdfium-render — v0.9.x; default features `pdfium_latest` (build 7881), `image_latest` (image 0.25), `thread_safe` (mutex-serialized); handles `Send + Sync` since 0.9.0; static linking supported via `PDFIUM_STATIC_LIB_PATH` with per-target-triple overrides; PDFium binaries from https://github.com/bblanchon/pdfium-binaries.
- Created `plan.md`: consolidated, corrected implementation spec (product definition, 4-layer architecture, pinned UI stack, PDFium strategy incl. single worker thread, domain model, coordinates, rendering, assets, interaction, undo-from-first-manipulation, export rules, testing/CI, phases 0–12, definition of done, risks).
- Created `documentation.md` (this file).

**Decisions:**

- **Name:** "Mark" with tagline **"Sign anything."** (semantically perfect — to mark a document is to sign it; user confirmed preference over Signet/Mohr/Imprint alternatives).
- **Exact dependency pins:** `gpui-kit = { version = "=0.7.0", default-features = false }` + `gpui-omarchy = "0.2.0"` — the styled `gpui-component` layer stays disabled; gpui-omarchy is the sole presentation system. Never bump one crate without the other (plan.md §5.1).
- **Undo/redo moved earlier** than in the draft plan: command-based undo is built together with the first manipulation code (Phase 7 merged), not as a later retrofit (plan.md §14, §24).
- **Coordinate fixture tests moved to Phase 4** (with PDF loading) instead of export time — the PDF bottom-left origin + `/Rotate` + cropbox math is the highest bug-risk area (plan.md §8.1).
- **Single dedicated PDFium worker thread** with channel-based communication, since all PDFium access is mutex-serialized anyway (plan.md §6.4).
- **`script/fetch-pdfium.sh` as Phase 0 deliverable** — deterministic PDFium dev setup on Omarchy and the future release-bundling mechanism (plan.md §6.3).
- **MVP has Export only** (always picks destination, default `<stem>-signed.<ext>`); "Save to output path" deferred (plan.md §16).
- **Asset normalization:** imported signatures/stamps are copied into app-managed storage as PNG at import time; library is self-contained (plan.md §9.3, §12).
- **CI matrix from Phase 2** (ubuntu/mac/windows `cargo check`/`test` on domain crates) + `clippy` added to the validation set (plan.md §20).

**Verification:**

- Facts about pinned versions/features were verified against the three repositories' READMEs as of 2026-10-03 (see Context links). No code exists yet — nothing to build or test.
- `/home/javaded/Work/mark` confirmed empty before writing; `plan.md` and `documentation.md` are the first files.

**Next:**

1. `git init` the repository (not yet done — awaiting explicit user request).
2. Phase 0: install gpui-kit agent skills; read `examples/ai_recipes/README.md`, gpui-omarchy guides and AGENTS.md; inventory UI needs vs. gpui-omarchy gallery.
3. Phase 0: create Cargo workspace skeleton + `script/fetch-pdfium.sh`; verify `Pdfium::default()` binds to the fetched library.
4. Write `docs/architecture.md` with the pinned version triple.
5. Follow plan.md §24 phases in order; log every session here.
