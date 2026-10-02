# Mark

**Mark. Sign anything.**

A native desktop application for placing visual signatures and stamps onto
PDFs and images. Open a document, drop in your signature or stamp, position
it, repeat across pages, export. The original file is never touched.

Native Rust, no Electron, no network, fully local.

## Status

Pre-alpha — Phase 0 (bootstrap). See `plan.md` for the full specification
and `documentation.md` for the development log.

## Stack

| Concern    | Choice                                        |
| ---------- | --------------------------------------------- |
| UI         | [GPUI](https://github.com/zed-industries/zed) via [gpui-kit](https://github.com/longbridge/gpui-kit) + [gpui-omarchy](https://github.com/huacnlee/gpui-omarchy) |
| PDF engine | PDFium via [pdfium-render](https://github.com/ajrcarey/pdfium-render) |
| Platforms  | Linux (Omarchy/Wayland) first, macOS and Windows next |

## Development setup (Linux / Omarchy)

Prerequisites: Rust (rustup), `clang`, `pkg-config`, `libxkbcommon`,
Wayland/EGL development packages. On Arch:

```bash
sudo pacman -S rust clang pkg-config libxkbcommon wayland-protocols mesa
```

Fetch the PDFium runtime (one-time, per platform):

```bash
script/fetch-pdfium.sh
```

Build and run:

```bash
cargo run -p mark-app
```

Verify the PDFium binding:

```bash
cargo run -p mark-pdf --example check_bind
```

## License

Apache-2.0
