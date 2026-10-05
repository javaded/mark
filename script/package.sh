#!/usr/bin/env bash
# Assemble a release artifact for the current platform (plan.md §19.3).
#
#   1. cargo build --release -p mark-app
#   2. Stage the binary + PDFium runtime + platform integration files
#   3. Produce dist/mark-<version>-<platform>-<arch>.{tar.gz,zip}
#   4. Verify the packaged binary binds the *bundled* runtime: its
#      --check-pdfium runs with the development vendor tree hidden, so a
#      pass proves §6.3's bundled-next-to-executable binding order works.
#
# Layouts (all resolved by absolute path in mark-pdf's bind.rs):
#   Linux    tar.gz  mark/       mark, libpdfium.so, mark.desktop, mark.svg, README.md
#   macOS    zip     Mark.app/   Contents/{MacOS/mark, Frameworks/libpdfium.dylib, Info.plist}
#   Windows  zip     mark/       mark.exe, pdfium.dll, README.md
#
# Run script/fetch-pdfium.sh first. Used by CI (.github/workflows/release.yml)
# and usable locally.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
if [ -z "$VERSION" ]; then
  echo "error: could not read the workspace version from Cargo.toml" >&2
  exit 1
fi

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$arch" in
    x86_64|amd64) arch="x64" ;;
    aarch64|arm64) arch="arm64" ;;
    *) echo "error: unsupported architecture: $arch" >&2; exit 1 ;;
  esac
  case "$os" in
    Linux) echo "linux-${arch}" ;;
    Darwin) echo "mac-${arch}" ;;
    MINGW*|MSYS*|CYGWIN*|Windows*) echo "win-${arch}" ;;
    *) echo "error: unsupported OS: $os" >&2; exit 1 ;;
  esac
}

PLATFORM="$(detect_platform)"
LIB_NAME="$(case "$PLATFORM" in
  win-*) echo "pdfium.dll" ;;
  mac-*) echo "libpdfium.dylib" ;;
  *) echo "libpdfium.so" ;;
esac)"

VENDOR_LIB="$(find "vendor/pdfium/$PLATFORM" -name "$LIB_NAME" -print | head -1)"
if [ -z "$VENDOR_LIB" ]; then
  echo "error: $LIB_NAME not found under vendor/pdfium/$PLATFORM — run script/fetch-pdfium.sh" >&2
  exit 1
fi

echo "Packaging mark $VERSION for $PLATFORM (runtime: $VENDOR_LIB)"
cargo build --release -p mark-app

STAGE="$(mktemp -d)"
DIST="$ROOT/dist"
rm -rf "$DIST"
mkdir -p "$DIST"

case "$PLATFORM" in
  linux-*)
    APP_DIR="$STAGE/mark"
    mkdir -p "$APP_DIR"
    cp target/release/mark "$APP_DIR/mark"
    cp "$VENDOR_LIB" "$APP_DIR/$LIB_NAME"
    cp resources/packaging/mark.desktop resources/packaging/mark.svg \
       resources/packaging/README.md "$APP_DIR/"
    ARTIFACT="$DIST/mark-$VERSION-$PLATFORM.tar.gz"
    tar -czf "$ARTIFACT" -C "$STAGE" mark
    ;;
  mac-*)
    APP_DIR="$STAGE/Mark.app/Contents"
    mkdir -p "$APP_DIR/MacOS" "$APP_DIR/Frameworks" "$APP_DIR/Resources"
    cp target/release/mark "$APP_DIR/MacOS/mark"
    cp "$VENDOR_LIB" "$APP_DIR/Frameworks/$LIB_NAME"
    sed "s/@VERSION@/$VERSION/g" resources/packaging/Info.plist > "$APP_DIR/Info.plist"
    cp resources/packaging/mark.svg "$APP_DIR/Resources/mark.svg"
    codesign --force --deep --sign - "$STAGE/Mark.app" >/dev/null 2>&1 || true
    ARTIFACT="$DIST/mark-$VERSION-$PLATFORM.zip"
    # -y keeps the zip quiet; ditto preserves bundle structure/permissions.
    if command -v ditto >/dev/null 2>&1; then
      (cd "$STAGE" && ditto -c -k --keepParent Mark.app "$ARTIFACT")
    else
      (cd "$STAGE" && zip -qr "$ARTIFACT" Mark.app)
    fi
    ;;
  win-*)
    APP_DIR="$STAGE/mark"
    mkdir -p "$APP_DIR"
    cp target/release/mark.exe "$APP_DIR/mark.exe"
    cp "$VENDOR_LIB" "$APP_DIR/$LIB_NAME"
    cp resources/packaging/README.md "$APP_DIR/"
    ARTIFACT="$DIST/mark-$VERSION-$PLATFORM.zip"
    powershell.exe -NoProfile -Command \
      "Compress-Archive -Path '$(cygpath -w "$APP_DIR" 2>/dev/null || echo "$APP_DIR")\*' -DestinationPath '$(cygpath -w "$ARTIFACT" 2>/dev/null || echo "$ARTIFACT")' -Force"
    ;;
esac

BINARY="$(case "$PLATFORM" in
  mac-*) echo "$STAGE/Mark.app/Contents/MacOS/mark" ;;
  win-*) echo "$STAGE/mark/mark.exe" ;;
  *) echo "$STAGE/mark/mark" ;;
esac)"

# §19.3: PDFium bundling is explicitly tested during packaging. Hide the
# development vendor tree so a successful check proves the bundled library
# (resolved next to the packaged executable) is what loaded. A trap puts
# it back whatever happens.
VENDOR_TREE="$ROOT/vendor/pdfium"
restore_vendor() { [ -d "$ROOT/.vendor-pdfium-hidden" ] && mv "$ROOT/.vendor-pdfium-hidden" "$VENDOR_TREE" || true; }
trap restore_vendor EXIT
mv "$VENDOR_TREE" "$ROOT/.vendor-pdfium-hidden"

echo "Verifying the packaged binary binds its bundled runtime…"
# The check must not see the repo's relative vendor path either.
(cd "$STAGE" && "$BINARY" --check-pdfium)
restore_vendor
trap - EXIT

echo "Artifact: $ARTIFACT"
