# Mark

**Mark. Sign anything.**

A native desktop application for placing visual signatures and stamps onto
PDFs and images. The original file is never touched — exports go to a new
`-signed` copy. Native Rust, no Electron, no network, fully local.

This build bundles the PDFium runtime next to the executable.

## Run

- **Linux**: extract, then run `mark/mark` (or install:
  `install -Dm755 mark/mark ~/.local/bin/mark` and
  `install -Dm644 mark/mark.svg ~/.local/share/icons/hicolor/scalable/apps/mark.svg`
  and `install -Dm644 mark/mark.desktop ~/.local/share/applications/mark.desktop`).
- **macOS**: unzip, then right-click `Mark.app` → Open (the build is
  unsigned; the first launch needs the right-click path past Gatekeeper).
- **Windows**: extract, run `mark\mark.exe`.

Verify the bundled PDFium runtime from a terminal:

```bash
./mark --check-pdfium        # or mark.exe / Contents/MacOS/mark
```

Keyboard: Ctrl/Cmd-O open · Ctrl/Cmd-S export · Ctrl/Cmd-Z undo ·
Ctrl/Cmd-D duplicate · Delete removes · Esc deselects.

Project: https://github.com/javaded/mark (MIT).
