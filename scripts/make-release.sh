#!/usr/bin/env bash
#
# Builds the release package: dist-release/wcf-<version>-<os>-<arch>.tar.gz
#
#   bash scripts/make-release.sh [--no-build]
#
# The archive holds one top-level folder `wcf-<version>/` with the server
# binary, the static site (CSS/JS/WASM, scripts, vendored libraries), the
# setup script, example configs and the docs. Database migrations are
# compiled into the binary. Deliberately absent: sources, .env, data/.
#
# Needs: Rust (see rust-toolchain.toml) with the wasm32-unknown-unknown
# target and cargo-leptos (`cargo install cargo-leptos --locked`).
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
BUILD=1
[ "${1:-}" = "--no-build" ] && BUILD=0

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
case "$(uname -s)" in
  Linux*) OS=linux ;;
  Darwin*) OS=macos ;;
  MINGW*|MSYS*|CYGWIN*) OS=windows ;;
  *) OS="$(uname -s | tr '[:upper:]' '[:lower:]')" ;;
esac
ARCH="$(uname -m)"
[ "$ARCH" = "amd64" ] && ARCH=x86_64
NAME="wcf-${VERSION}"
OUT="$ROOT/dist-release"
STAGE="$OUT/$NAME"

if [ "$BUILD" = 1 ]; then
  echo "==> cargo leptos build --release (hashed asset names, gzip/brotli)"
  # Content hashes in /pkg file names let browsers cache them forever;
  # the precompressed .br/.gz copies are served by leptos_axum directly.
  LEPTOS_HASH_FILES=true cargo leptos build --release --precompress
fi

BIN="target/release/server"
[ -f "$BIN.exe" ] && BIN="$BIN.exe"
[ -f "$BIN" ] || { echo "missing $BIN — build first" >&2; exit 1; }
[ -d target/site/pkg ] || { echo "missing target/site — build first" >&2; exit 1; }

echo "==> staging $STAGE"
rm -rf "$STAGE"
mkdir -p "$STAGE/scripts" "$STAGE/deploy"
if [ "$OS" = windows ]; then cp "$BIN" "$STAGE/wcf.exe"; else cp "$BIN" "$STAGE/wcf"; chmod +x "$STAGE/wcf"; fi
cp -r target/site "$STAGE/site"
HASHED=false
if [ -f target/release/hash.txt ]; then
  # Leptos looks for the hash file next to the executable.
  cp target/release/hash.txt "$STAGE/hash.txt"
  HASHED=true
fi
cp scripts/setup.sh "$STAGE/scripts/"
cp deploy/* "$STAGE/deploy/"
cp .env.example README.md ROADMAP.md "$STAGE/"
cp assets/vendor/*.LICENSE "$STAGE/site/vendor/" 2>/dev/null || true
printf '%s\n' "$VERSION" > "$STAGE/VERSION"
printf 'LEPTOS_HASH_FILES=%s\n' "$HASHED" > "$STAGE/.release-env"

cat > "$STAGE/INSTALL.md" <<EOF
# wcf ${VERSION} installieren

1. Paket auf den Server laden und entpacken, z. B.

   \`\`\`sh
   tar xzf ${NAME}-${OS}-${ARCH}.tar.gz -C /opt && cd /opt/${NAME}
   \`\`\`

2. Setup ausführen. Es prüft PostgreSQL und ffmpeg, legt auf Wunsch Datenbank
   und Benutzer an, schreibt die \`.env\`, macht einen Probestart und richtet
   einen systemd-Dienst ein:

   \`\`\`sh
   sudo bash scripts/setup.sh
   \`\`\`

   \`--yes\` übernimmt alle Vorgaben ohne Rückfragen.

3. Den Reverse-Proxy (nginx oder Caddy, Vorlagen in \`deploy/\`) auf den
   ausgegebenen Port zeigen lassen – **WebSockets müssen durchgereicht werden**
   – und TLS aktivieren.

4. Seite öffnen und das erste Konto registrieren: es wird Administrator.

Alles Weitere (LiveKit für Anrufe, Kino-Mediathek, Backups, Updates):
README.md.
EOF

mkdir -p "$OUT"
ARCHIVE="$OUT/${NAME}-${OS}-${ARCH}.tar.gz"
tar -C "$OUT" -czf "$ARCHIVE" "$NAME"
echo "==> $ARCHIVE ($(du -h "$ARCHIVE" | cut -f1))"
