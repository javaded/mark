# Mark

**Mark. Sign anything.**

A native desktop application for placing visual signatures and stamps onto
PDFs and images. Open a document, drop in your signature or stamp, position
it, repeat across pages, export. The original file is never touched.

Native Rust, no Electron, no network, fully local.

## Status

Pre-alpha — under active development. See `plan.md` for the full
specification and `documentation.md` for the development log.

## Stack

| Concern    | Choice                                        |
| ---------- | --------------------------------------------- |
| UI         | [GPUI](https://github.com/zed-industries/zed) via [gpui-kit](https://github.com/longbridge/gpui-kit) + [gpui-omarchy](https://github.com/huacnlee/gpui-omarchy) |
| PDF engine | PDFium via [pdfium-render](https://github.com/ajrcarey/pdfium-render) |
| Platforms  | Linux (Omarchy/Wayland) first; macOS and Windows compile+CI-verified |

## Development setup (Linux / Omarchy)

Prerequisites: Rust (rustup), `clang`, `pkg-config`, and the GPUI Linux
backend libraries — `libxkbcommon` (+ X11 variant), `libxcb1`, Wayland
(client + EGL), EGL/GLESv2, and `fontconfig` development packages.

On Arch / Omarchy:

```bash
sudo pacman -S rust clang pkg-config libxkbcommon wayland-protocols mesa fontconfig
```

On Debian / Ubuntu (the exact set CI installs):

```bash
sudo apt install clang pkg-config libxkbcommon-dev libxkbcommon-x11-dev \
  libxcb1-dev libwayland-dev libegl1-mesa-dev libgles2-mesa-dev \
  libfontconfig1-dev
```

## Development setup (macOS / Windows)

Rust (rustup) and — on Windows — the [Git bash](
https://gitforwindows.org/) tools (Git for Windows includes them; used by
`script/fetch-pdfium.sh`). No extra system libraries: GPUI links the
platform frameworks (Metal / D3D via wgpu) out of the box.

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
