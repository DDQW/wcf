#!/usr/bin/env bash
#
# wcf setup — takes an unpacked release from zero to a running service.
#
#   sudo bash scripts/setup.sh [options]
#
#     -y, --yes          accept every default, no questions asked
#     --port PORT        local port for the server (default 3000)
#     --domain NAME      public domain, e.g. community.example.org
#     --user NAME        system user for the service (default: wcf)
#     --no-service       do not install a systemd service
#     -h, --help         this text
#
# Safe to re-run: an existing .env is kept unless you ask for a new one.
# Run as root for the full setup (database, user, systemd). Without root the
# script writes .env, smoke-tests and prints the remaining steps.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ASSUME_YES=0
PORT=3000
DOMAIN=""
SVC_USER=wcf
SKIP_SERVICE=0

while [ $# -gt 0 ]; do
  case "$1" in
    -y|--yes) ASSUME_YES=1 ;;
    --port) PORT="$2"; shift ;;
    --domain) DOMAIN="$2"; shift ;;
    --user) SVC_USER="$2"; shift ;;
    --no-service) SKIP_SERVICE=1 ;;
    -h|--help) sed -n '3,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
  esac
  shift
done

if [ -t 1 ]; then B=$'\033[1m'; G=$'\033[32m'; Y=$'\033[33m'; R=$'\033[31m'; N=$'\033[0m'
else B=''; G=''; Y=''; R=''; N=''; fi
step() { printf '\n%s==> %s%s\n' "$B" "$1" "$N"; }
ok()   { printf '    %s✓%s %s\n' "$G" "$N" "$1"; }
warn() { printf '    %s!%s %s\n' "$Y" "$N" "$1"; }
die()  { printf '\n%sx%s %s\n' "$R" "$N" "$1" >&2; exit 1; }
ask() {
  local prompt="$1" default="${2:-}" reply=''
  if [ "$ASSUME_YES" = 1 ]; then echo "$default"; return; fi
  printf '    %s [%s]: ' "$prompt" "$default" >&2
  read -r reply </dev/tty || reply=''
  echo "${reply:-$default}"
}
confirm() {
  local prompt="$1" default="${2:-y}" reply=''
  if [ "$ASSUME_YES" = 1 ]; then [ "$default" = y ]; return; fi
  local hint='[J/n]'; [ "$default" = y ] || hint='[j/N]'
  printf '    %s %s ' "$prompt" "$hint" >&2
  read -r reply </dev/tty || reply=''
  reply="${reply:-$default}"
  case "$reply" in [JjYy]*) return 0 ;; *) return 1 ;; esac
}
randpw() { head -c 24 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32; }

IS_ROOT=0; [ "$(id -u)" = 0 ] && IS_ROOT=1
cd "$ROOT"
BIN="$ROOT/wcf"
[ -x "$BIN" ] || die "Programmdatei $BIN fehlt – bitte aus dem entpackten Release-Ordner starten."
VERSION="$(cat VERSION 2>/dev/null || echo '?')"
printf '%swcf %s – Einrichtung in %s%s\n' "$B" "$VERSION" "$ROOT" "$N"

# ── 1. Werkzeuge ─────────────────────────────────────────────────────────────
step "Voraussetzungen prüfen"
command -v curl >/dev/null || die "curl wird benötigt (apt install curl)."
ok "curl"
if command -v ffmpeg >/dev/null && command -v ffprobe >/dev/null; then
  ok "ffmpeg $(ffmpeg -version | head -n1 | awk '{print $3}') (Video-Vorschauen, Kino-Mediathek)"
else
  warn "ffmpeg fehlt – Video-Vorschauen und die Kino-Mediathek bleiben aus (apt install ffmpeg)."
fi
HAVE_PSQL=0
if command -v psql >/dev/null; then HAVE_PSQL=1; ok "psql"; else warn "psql nicht gefunden – Datenbank bitte selbst anlegen."; fi

# ── 2. Datenbank ─────────────────────────────────────────────────────────────
step "PostgreSQL"
DB_URL=""
if [ -f .env ] && grep -q '^DATABASE_URL=' .env; then
  DB_URL="$(sed -n 's/^DATABASE_URL=//p' .env | head -n1)"
  ok "DATABASE_URL aus vorhandener .env übernommen"
elif [ "$IS_ROOT" = 1 ] && [ "$HAVE_PSQL" = 1 ] && id postgres >/dev/null 2>&1 \
     && confirm "Datenbank und Benutzer „wcf“ lokal anlegen?" y; then
  DB_PW="$(randpw)"
  if sudo -u postgres psql -tAc "SELECT 1 FROM pg_roles WHERE rolname='wcf'" | grep -q 1; then
    sudo -u postgres psql -qc "ALTER ROLE wcf WITH LOGIN PASSWORD '$DB_PW'"
    ok "Rolle wcf existiert – Passwort neu gesetzt"
  else
    sudo -u postgres psql -qc "CREATE ROLE wcf WITH LOGIN PASSWORD '$DB_PW'"
    ok "Rolle wcf angelegt"
  fi
  if ! sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='wcf'" | grep -q 1; then
    sudo -u postgres createdb -O wcf wcf
    ok "Datenbank wcf angelegt"
  fi
  sudo -u postgres psql -q -d wcf -c "CREATE EXTENSION IF NOT EXISTS citext"
  ok "Erweiterung citext aktiv"
  DB_URL="postgres://wcf:${DB_PW}@localhost:5432/wcf"
else
  DB_URL="$(ask "DATABASE_URL" "postgres://wcf:passwort@localhost:5432/wcf")"
fi
if [ "$HAVE_PSQL" = 1 ]; then
  if psql "$DB_URL" -qtAc 'SELECT 1' >/dev/null 2>&1; then ok "Verbindung zur Datenbank klappt"
  else warn "Keine Verbindung mit DATABASE_URL – bitte prüfen (der Probestart zeigt den Fehler)."; fi
fi

# ── 3. Konfiguration ─────────────────────────────────────────────────────────
step "Konfiguration (.env)"
HASHED="$(sed -n 's/^LEPTOS_HASH_FILES=//p' .release-env 2>/dev/null || echo false)"
WRITE_ENV=1
if [ -f .env ]; then
  if confirm ".env existiert – neu schreiben? (Nein behält sie)" n; then cp .env ".env.bak.$(date +%s)"; else WRITE_ENV=0; fi
fi
if [ "$WRITE_ENV" = 1 ]; then
  [ -n "$DOMAIN" ] || DOMAIN="$(ask "Öffentliche Domain (ohne https://)" "community.example.org")"
  SCHEME=https
  confirm "Läuft die Seite hinter HTTPS?" y || SCHEME=http
  cat > .env <<EOF
# wcf $VERSION – erzeugt von scripts/setup.sh am $(date '+%F %T')
DATABASE_URL=$DB_URL
PUBLIC_URL=$SCHEME://$DOMAIN
DATA_DIR=data
COOKIE_SECURE=$([ "$SCHEME" = https ] && echo true || echo false)
TRUST_PROXY=true
RUST_LOG=info

# Leptos-Laufzeit (nicht ändern)
LEPTOS_OUTPUT_NAME=woelkicommunityframework
LEPTOS_SITE_ROOT=site
LEPTOS_SITE_PKG_DIR=pkg
LEPTOS_SITE_ADDR=127.0.0.1:$PORT
LEPTOS_ENV=PROD
LEPTOS_HASH_FILES=$HASHED

# Optional – siehe .env.example
#SMTP_HOST=smtp.example.org
#SMTP_FROM=Community <community@example.org>
#LIVEKIT_URL=wss://livekit.$DOMAIN
#LIVEKIT_API_KEY=
#LIVEKIT_API_SECRET=
#WATCH_IMPORT_DIR=/srv/filme
#WATCH_MAX_MEDIA_GB=200
EOF
  chmod 600 .env
  ok ".env geschrieben (nur für den Besitzer lesbar)"
else
  PORT="$(sed -n 's/^LEPTOS_SITE_ADDR=.*://p' .env | head -n1)"; PORT="${PORT:-3000}"
  DOMAIN="${DOMAIN:-$(sed -n 's#^PUBLIC_URL=https\{0,1\}://##p' .env | head -n1)}"
fi
mkdir -p data
ok "Datenverzeichnis $ROOT/data"

# ── 4. Dienstnutzer ──────────────────────────────────────────────────────────
if [ "$IS_ROOT" = 1 ]; then
  step "Dienstnutzer"
  if ! id "$SVC_USER" >/dev/null 2>&1; then
    useradd --system --home-dir "$ROOT" --shell /usr/sbin/nologin "$SVC_USER"
    ok "Systemnutzer $SVC_USER angelegt"
  else
    ok "Systemnutzer $SVC_USER vorhanden"
  fi
  chown -R "$SVC_USER:$SVC_USER" "$ROOT/data" "$ROOT/.env"
fi

# ── 5. Probestart ────────────────────────────────────────────────────────────
step "Probestart"
LOG="$(mktemp)"
if [ "$IS_ROOT" = 1 ]; then
  (cd "$ROOT" && exec sudo -u "$SVC_USER" "$BIN") >"$LOG" 2>&1 &
else
  (cd "$ROOT" && exec "$BIN") >"$LOG" 2>&1 &
fi
PID=$!
UP=0
for _ in $(seq 1 60); do
  if curl -fsS -o /dev/null "http://127.0.0.1:$PORT/"; then UP=1; break; fi
  kill -0 "$PID" 2>/dev/null || break
  sleep 1
done
kill "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
if [ "$UP" = 1 ]; then
  ok "Server antwortet auf Port $PORT (Migrationen sind eingespielt)"
else
  tail -n 25 "$LOG" >&2
  die "Der Server startet nicht – Ausgabe siehe oben."
fi
rm -f "$LOG"

# ── 6. systemd ───────────────────────────────────────────────────────────────
if [ "$SKIP_SERVICE" = 0 ]; then
  step "systemd-Dienst"
  if [ "$IS_ROOT" = 1 ] && command -v systemctl >/dev/null; then
    sed -e "s#@DIR@#$ROOT#g" -e "s#@USER@#$SVC_USER#g" deploy/wcf.service > /etc/systemd/system/wcf.service
    systemctl daemon-reload
    systemctl enable --now wcf.service
    ok "wcf.service aktiv (Logs: journalctl -u wcf -f)"
  else
    warn "Ohne root/systemd: Vorlage deploy/wcf.service anpassen und einrichten,"
    warn "oder als Nutzerdienst: ~/.config/systemd/user/ + loginctl enable-linger \$USER"
  fi
fi

# ── 7. Reverse-Proxy ─────────────────────────────────────────────────────────
step "Reverse-Proxy"
cat <<EOF
    Leite https://$DOMAIN auf http://127.0.0.1:$PORT weiter – WebSockets (/ws)
    müssen durchgereicht werden. Vorlagen:
      deploy/Caddyfile   (Caddy, TLS automatisch)
      deploy/nginx.conf  (nginx)
    Danach https://$DOMAIN öffnen und das erste Konto registrieren –
    es wird Administrator.
EOF
printf '\n%sFertig.%s\n' "$G" "$N"
