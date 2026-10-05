# Woelki Community Framework (wcf)

Eine selbst gehostete Community-Plattform in Rust – ein Programm, eine Datenbank:

- **Forum** mit Kategorien, Tags, Zitaten, Erwähnungen, Reaktionen, Abos, Volltextsuche
- **Chat** mit Kanälen, Direktnachrichten, Gruppen, Antworten, Anhängen, Live-Tippanzeige
- **Blogs** pro Mitglied mit Editor, geplanten Beiträgen, Kommentaren, Abos und RSS
- **Dateien** (Filebase): Ordner, Papierkorb, Vorschauen, Freigabelinks, Speicherkontingente
- **Galerie** mit Alben, Community-Alben, EXIF, Lightbox und Diashow
- **Inhalte (CMS)**: eigene Inhaltstypen mit Feldern, Seiten, Menü, JSON-API
- **Anrufe**: Sprach-/Videoräume und 1:1-Anrufe mit Klingeln, Bildschirmfreigabe (LiveKit)
- **Kino** (Watch together, aus [HQTube](#herkunft-hqtube) übernommen): synchrones Schauen mit
  Playlist (YouTube, Links, eigene Dateien, Mediathek), Vorstellungen mit Pausen,
  Showmaster-Konsole, Filmwünsche, Emotes, Popout-Player
- **Verwaltung**: Rollen, Meldungen, Moderationsprotokoll, Module an/aus, Einladungscodes

Die Oberfläche ist deutsch, rendert serverseitig (schnell, suchmaschinenfreundlich) und
wird im Browser per WebAssembly interaktiv; Live-Funktionen laufen über einen einzigen
WebSocket.

## Inhalt

- [Technik](#technik)
- [Entwicklung unter Windows](#entwicklung-unter-windows)
- [Produktion unter Linux](#produktion-unter-linux)
- [Konfiguration](#konfiguration)
- [Anrufe mit LiveKit](#anrufe-mit-livekit)
- [Kino und Mediathek](#kino-und-mediathek)
- [Backups und Updates](#backups-und-updates)
- [Tests und Qualität](#tests-und-qualität)

## Technik

| Teil | Technik |
| --- | --- |
| Server | [Axum](https://github.com/tokio-rs/axum) 0.8, Tokio, Cookie-Sessions in PostgreSQL |
| Oberfläche | [Leptos](https://leptos.dev) 0.8 (SSR + Hydration), SCSS |
| Datenbank | PostgreSQL ≥ 14 (citext, Volltextsuche `german`), Migrationen im Binary |
| Medien | `image` + EXIF für Bilder, optional ffmpeg/ffprobe für Videos und die Mediathek |
| Echtzeit | ein WebSocket (`/ws`) mit Themen für Forum, Chat, Präsenz, Anrufe, Kino |
| Anrufe | [LiveKit](https://livekit.io) als eigener Dienst; wcf gibt die Zugangstokens aus |

```
crates/shared    Datentypen und Protokoll (Server + Browser)
crates/backend   Datenbank, Rechte, Dienste, Realtime-Hub, HTTP-Routen
crates/app       Leptos-Oberfläche und Server-Funktionen
crates/server    Programmstart (Axum + Leptos)
migrations/      SQL-Migrationen (werden beim Start automatisch eingespielt)
assets/          statische Dateien: Call-/Kino-Skripte, livekit-client, hls.js
style/main.scss  Design-System (hell/dunkel)
```

## Entwicklung unter Windows

**Voraussetzungen**

1. Rust (stable) über [rustup](https://rustup.rs); das WebAssembly-Ziel installiert
   `rust-toolchain.toml` automatisch.
2. `cargo install cargo-leptos --locked`
3. PostgreSQL 17 (z. B. `winget install PostgreSQL.PostgreSQL.17`), dann eine Datenbank:
   ```sql
   CREATE ROLE wcf LOGIN PASSWORD 'change-me';
   CREATE DATABASE wcf OWNER wcf;
   \c wcf
   CREATE EXTENSION IF NOT EXISTS citext;
   ```
4. Optional: ffmpeg (`winget install Gyan.FFmpeg`) – wird auch ohne PATH-Eintrag in
   winget-Installationen gefunden.

**Starten**

```powershell
copy .env.example .env        # DATABASE_URL anpassen
cargo leptos watch            # baut Server + WASM, startet auf http://127.0.0.1:3000
```

Das erste registrierte Konto wird Administrator. Änderungen an Rust-Code, SCSS und
`assets/` werden automatisch neu gebaut.

**Anrufe lokal testen**: [`livekit-server`](https://github.com/livekit/livekit/releases)
herunterladen und mit `livekit-server --dev` starten; in der `.env`:
`LIVEKIT_URL=ws://127.0.0.1:7880`, `LIVEKIT_API_KEY=devkey`, `LIVEKIT_API_SECRET=secret`.

## Produktion unter Linux

### Mit dem Release-Paket (empfohlen)

Release-Pakete baut GitHub Actions bei jedem Tag `v*` (Ubuntu 22.04, läuft auf allen
aktuellen glibc-Distributionen). Selbst bauen: `bash scripts/make-release.sh` →
`dist-release/wcf-<version>-linux-x86_64.tar.gz`. Die Dateien unter `site/` tragen
Inhalts-Hashes im Namen (Browser cachen sie dauerhaft) und liegen zusätzlich als
`.br`/`.gz` vor, die der Server direkt ausliefert.

```sh
sudo apt install postgresql ffmpeg curl
tar xzf wcf-1.0.0-linux-x86_64.tar.gz -C /opt
cd /opt/wcf-1.0.0
sudo bash scripts/setup.sh --domain community.example.org
```

Das Setup legt auf Wunsch Datenbank und Benutzer an, schreibt eine `.env`
(nur für den Dienst lesbar), richtet den Systemnutzer `wcf` ein, macht einen Probestart
(spielt dabei die Migrationen ein) und aktiviert `wcf.service`. Logs:
`journalctl -u wcf -f`.

### Reverse-Proxy

wcf lauscht nur lokal (`LEPTOS_SITE_ADDR=127.0.0.1:3000`). Davor gehört ein Proxy mit
TLS, der **WebSockets** durchreicht und den `Host`-Header weitergibt:

- **Caddy** – [`deploy/Caddyfile`](deploy/Caddyfile), Zertifikate automatisch
- **nginx** – [`deploy/nginx.conf`](deploy/nginx.conf) (Upgrade-Header, keine Puffer
  für Uploads und Streaming, lange Timeouts)

Hinter HTTPS `COOKIE_SECURE=true` und `TRUST_PROXY=true` setzen (macht das Setup).

### Manuell

1. Datenbank wie oben anlegen.
2. `.env` aus `.env.example` erstellen; im Release zusätzlich
   `LEPTOS_OUTPUT_NAME=woelkicommunityframework`, `LEPTOS_SITE_ROOT=site`,
   `LEPTOS_SITE_PKG_DIR=pkg`, `LEPTOS_ENV=PROD` und `LEPTOS_HASH_FILES=true`
   (wenn das Paket eine `hash.txt` enthält).
3. [`deploy/wcf.service`](deploy/wcf.service) anpassen (`@DIR@`, `@USER@`) und
   nach `/etc/systemd/system/` kopieren, dann `systemctl enable --now wcf`.

## Konfiguration

Alles läuft über Umgebungsvariablen (bzw. die `.env` im Arbeitsverzeichnis). Inhaltliche
Einstellungen – Seitenname, Module, Upload-Grenzen, Registrierung, Einladungen – liegen in
der Datenbank und werden unter **Verwaltung → Einstellungen** gepflegt.

| Variable | Bedeutung |
| --- | --- |
| `DATABASE_URL` | PostgreSQL-Verbindung (Pflicht) |
| `LEPTOS_SITE_ADDR` | Adresse:Port des Servers, Standard `127.0.0.1:3000` |
| `PUBLIC_URL` | öffentliche Basis-URL für Links in Mails und Feeds |
| `DATA_DIR` | Uploads, Vorschauen, Mediathek (Standard `data`) |
| `COOKIE_SECURE` | `true` hinter HTTPS |
| `TRUST_PROXY` | `X-Forwarded-For` vertrauen (nur hinter einem Proxy) |
| `SMTP_HOST`, `SMTP_PORT`, `SMTP_TLS`, `SMTP_USERNAME`, `SMTP_PASSWORD`, `SMTP_FROM` | E-Mail für Passwort-Reset; ohne SMTP setzen Admins Links manuell |
| `LIVEKIT_URL`, `LIVEKIT_API_KEY`, `LIVEKIT_API_SECRET`, `LIVEKIT_API_URL` | Anrufe (siehe unten) |
| `FFMPEG_PATH` | ffmpeg, falls nicht im PATH (ffprobe wird daneben gesucht) |
| `WATCH_IMPORT_DIR` | Import-Ordner der Kino-Mediathek (Standard `DATA_DIR/watch/incoming`) |
| `WATCH_MAX_MEDIA_GB` | Obergrenze für verpackte HLS-Medien |
| `RUST_LOG` | Log-Level, z. B. `info` oder `info,backend=debug` |

## Anrufe mit LiveKit

wcf übernimmt Rechte, Räume, Klingeln und Präsenz; die Medienströme laufen über einen
eigenen [LiveKit](https://docs.livekit.io/home/self-hosting/)-Server (SFU).

1. `livekit-server` installieren (Binary oder Docker) und mit
   [`deploy/livekit.yaml`](deploy/livekit.yaml) starten – Schlüsselpaar eintragen.
2. LiveKit über eine eigene Subdomain per Proxy erreichbar machen
   (`livekit.community.example.org` → `127.0.0.1:7880`, WebSockets).
3. Firewall: `7881/tcp` und `50000–60000/udp` öffnen. Hinter strengen NATs zusätzlich
   TURN in LiveKit aktivieren.
4. In der `.env` von wcf: `LIVEKIT_URL=wss://livekit.community.example.org`,
   `LIVEKIT_API_KEY`, `LIVEKIT_API_SECRET`. Für Moderation (stummschalten, entfernen)
   nutzt wcf die RoomService-API unter derselben Adresse (`LIVEKIT_API_URL` überschreibt).

Ohne LiveKit bleiben Räume, Verlauf und Chat nutzbar, nur Gespräche sind deaktiviert.

## Kino und Mediathek

Kinoräume spielen **YouTube-Links**, **direkte Videolinks** (mp4, webm, m3u8), **eigene
Videodateien** aus „Dateien“ und Filme der **Mediathek**. Die Wiedergabe ist
serverautoritativ: alle Zuschauer sehen dieselbe Stelle, Nachzügler springen hinein,
kleine Abweichungen werden unmerklich über die Abspielrate ausgeglichen.

Die Mediathek (nur Administratoren) importiert Videodateien aus dem Import-Ordner:

- **Direct Play** – fertige mp4/webm-Dateien werden unverändert gestreamt (nur die
  Standard-Tonspur).
- **HLS-Remux** – mehrere Tonspuren werden verlustfrei in umschaltbares HLS (fMP4)
  verpackt; nur nicht browsertaugliche Codecs (HEVC, AC3, DTS …) werden umkodiert.
- **Automatisch** wählt Direct Play für passende Einzelton-Dateien, sonst HLS.

Textuntertitel werden zu WebVTT, ein Posterbild wird erzeugt, Quelldateien bleiben
unangetastet. Vorstellungen kombinieren Mediathek-Filme mit Pausen; sie starten
pünktlich, laufen auch ohne Zuschauer und setzen nach einem Neustart an der richtigen
Stelle fort. Die Konsole (`/watch/<raum>/console`) steuert Ablauf, Pausen, Moderation
und Wünsche.

## Backups und Updates

**Backup** – [`deploy/backup.sh`](deploy/backup.sh) sichert die Datenbank (`pg_dump`)
und den Datenordner (ohne verpackte Kino-Medien und Filmquellen):

```sh
15 3 * * * cd /opt/wcf-1.0.0 && sudo -u wcf bash deploy/backup.sh /var/backups/wcf 14
```

Wiederherstellen: `pg_restore -d wcf db-<zeit>.dump` und `data-<zeit>.tar.gz` entpacken.

**Update** – neues Paket daneben entpacken, `.env` und `data/` übernehmen (oder
verlinken), `deploy/wcf.service` auf den neuen Ordner zeigen lassen und den Dienst neu
starten. Migrationen laufen beim Start automatisch; vorher ein Backup ziehen.

## Tests und Qualität

```sh
cargo test -p shared -p backend         # Unit- und Integrationstests
cargo test -p app --features ssr --lib
cargo clippy -p shared -p backend -p server --all-targets -- -D warnings
cargo clippy -p app --features hydrate --target wasm32-unknown-unknown -- -D warnings
```

Die Integrationstests (`crates/backend/tests`) legen pro Test eine eigene
Wegwerf-Datenbank an (`DATABASE_URL` oder `TEST_DATABASE_URL`, das Konto braucht
`CREATEDB`) und werden ohne Datenbank übersprungen. GitHub Actions führt alles bei jedem
Push aus.

## Herkunft HQTube

Das Kino ist eine Rust-Portierung von HQTube (Node/React/SQLite): Show-Engine mit
Neustart-Erholung, Mediathek-Pipeline, Showmaster-Konsole, Audio-Election für mehrere
Fenster, Filmwünsche, Emotes und Einladungscodes – jetzt im selben Programm, mit
denselben Konten und derselben Datenbank wie der Rest der Community.

## Eingebundene Bibliotheken

`assets/vendor` enthält [livekit-client](https://github.com/livekit/client-sdk-js) und
[hls.js](https://github.com/video-dev/hls.js) (beide Apache-2.0, Lizenztexte daneben).
