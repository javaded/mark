#!/usr/bin/env bash
# Fetch a PDFium runtime binary from bblanchon/pdfium-binaries.
#
# Downloads into vendor/pdfium/<platform>/ (gitignored). Run once per machine
# before `cargo run -p mark-pdf --example check_bind` or any PDF feature work.
#
# Environment overrides:
#   PDFIUM_VERSION  PDFium build number to fetch (default: 7881, the build
#                   pdfium-render 0.9.x binds its default `pdfium_latest`
#                   feature against). Keep in sync with docs/architecture.md.
#   PDFIUM_PLATFORM Forced platform triple, e.g. linux-x64, mac-arm64, win-x64.

set -euo pipefail

PDFIUM_VERSION="${PDFIUM_VERSION:-7881}"
TAG="chromium/${PDFIUM_VERSION}"
REPO_URL_BASE="https://github.com/bblanchon/pdfium-binaries/releases/download"

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
    Linux)
      # musl-based distros need the musl build
      if ldd --version 2>&1 | grep -qi musl; then
        echo "linux-musl-${arch}"
      else
        echo "linux-${arch}"
      fi
      ;;
    Darwin) echo "mac-${arch}" ;;
    MINGW*|MSYS*|CYGWIN*|Windows*) echo "win-${arch}" ;;
    *) echo "error: unsupported OS: $os" >&2; exit 1 ;;
  esac
}

PLATFORM="${PDFIUM_PLATFORM:-$(detect_platform)}"
ASSET="pdfium-${PLATFORM}.tgz"
URL="${REPO_URL_BASE}/${TAG}/${ASSET}"
OUT_DIR="$(cd "$(dirname "$0")/.." && pwd)/vendor/pdfium/${PLATFORM}"
TMP_ARCHIVE="$(mktemp -t pdfium-XXXXXX.tgz)"

echo "Fetching PDFium ${TAG} for ${PLATFORM}..."
echo "  ${URL}"
# --retry does not cover connect/SSL failures (curl exit 35/7), which
# CI runners hit occasionally — so the whole download retries too.
downloaded=0
for attempt in 1 2 3 4 5; do
  if curl -fSL --retry 3 --retry-connrefused -o "$TMP_ARCHIVE" "$URL"; then
    downloaded=1
    break
  fi
  echo "download attempt ${attempt} failed; retrying in 5s…" >&2
  sleep 5
done
[ "$downloaded" = 1 ] || { echo "error: could not download ${URL}" >&2; exit 1; }

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
tar -xzf "$TMP_ARCHIVE" -C "$OUT_DIR"
rm -f "$TMP_ARCHIVE"

LIB_PATHS="$(find "$OUT_DIR" \( -name 'libpdfium.so*' -o -name 'libpdfium.dylib' -o -name 'pdfium.dll' \) -print)"

if [ -z "$LIB_PATHS" ]; then
  echo "error: no PDFium library found after extraction" >&2
  exit 1
fi

echo "Extracted to ${OUT_DIR}"
echo "Library:"
echo "$LIB_PATHS"
