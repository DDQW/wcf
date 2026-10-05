# Roadmap & Feature-Katalog

Selbst gehostete Community-Plattform (Axum + Leptos/WASM + PostgreSQL, WebSockets für Realtime).
Alle Arbeit passiert auf `master`.

Legende: `[x]` fertig · `[ ]` offen

## Phase 0 – Fundament + Forum-MVP
- [x] Cargo-Workspace (`shared` / `app` / `server`), Leptos 0.8 SSR + Hydration, Axum 0.8
- [x] PostgreSQL + sqlx-Migrationen, Cookie-Sessions (tower-sessions, Postgres-Store)
- [x] Registrierung / Login / Logout, Argon2id, erster User wird Admin
- [x] Forum-Grundgerüst: Kategorien, Themen, verschachtelte Antworten, Thread-Blöcke
- [x] `/ws`-Echo-Route

## Phase 1 – Plattform-Kern (Basis für alle Module)
- [x] Neue Crate `backend` (DB, Services, Realtime-Hub, Storage) – getrennt von UI-Code
- [x] Einheitliche Fehler (`AppError`), deutsche Fehlermeldungen
- [x] Markdown: Sanitizing (XSS-Schutz), @Mentions, Code-Blöcke, Tabellen, Aufgabenlisten
- [x] Rollen (Admin / Moderator / Mitglied), Sperren mit Dauer + Grund, Deaktivieren
- [x] Profile: Avatar, Bio, Ort, Website, Mitglied seit, zuletzt online, Statistiken
- [x] Einstellungen: Anzeigename, E-Mail, Passwort ändern, Theme, Benachrichtigungs-Optionen
- [x] Passwort-Reset (E-Mail per SMTP, falls konfiguriert – sonst Reset-Link durch Admin)
- [ ] Einladungscodes: Registrierung optional nur mit Code (aus HQTube übernommen)
- [x] Mitgliederliste mit Suche, Rollenfilter, Online-Status
- [x] Benachrichtigungen (In-App, live per WebSocket): Antworten, Mentions, Reaktionen, Kommentare, Abos
- [x] Reaktionen (Emoji) generisch für alle Inhalte
- [x] Melden von Inhalten + Moderations-Warteschlange, Moderations-Log (Audit)
- [x] Admin-Bereich: Dashboard (Statistiken), Benutzerverwaltung, Seiteneinstellungen
      (Name, Beschreibung, Registrierung offen/zu, Module an/aus, Upload-Limits)
- [x] Realtime-Hub: ein multiplexter WebSocket pro Client, Topics, Presence, Auto-Reconnect
- [x] Globale Volltextsuche (Postgres `tsvector`, deutsch) über alle Module
- [x] App-Shell: Seitenleiste, Suche, Benachrichtigungs-Glocke, Benutzermenü, Hell/Dunkel, mobil
- [x] Startseite / Aktivitäts-Dashboard (neueste Themen, Blogposts, Bilder, wer ist online)
- [x] Sicherheit: Security-Header, Rate-Limits (Login, Registrierung, Chat), SameSite-Cookies

## Phase 2 – Forum komplett
- [x] Kategorien: Unterkategorien, Farbe, Reihenfolge, Bearbeiten/Löschen, Schreibrechte pro Kategorie
- [x] Themen: Tags, Anpinnen, Sperren, Verschieben, Löschen, Titel bearbeiten, Aufrufzähler
- [x] Sortierung: Aktivität / Neu / Top; „Neueste Themen" über alle Kategorien; Tag-Seiten
- [x] Beiträge: Bearbeiten (mit Versionshistorie), Löschen, Zitieren, Reaktionen, Melden, Permalink
- [x] Thread-Blöcke seitenweise laden (echtes Endlos-Scrollen per IntersectionObserver)
- [x] Ungelesen-Markierung pro Thema, „Neu"-Badges, Sprung zum ersten ungelesenen Beitrag
- [x] Themen beobachten / stummschalten → Benachrichtigungen
- [x] Live-Updates: neue Beiträge erscheinen per WebSocket
- [x] Editor mit Toolbar, Vorschau, @Mention-Vorschläge

## Phase 3 – Filebase
- [x] Upload (Mehrfach, Drag & Drop, Fortschrittsbalken), Größenlimit + Speicherkontingent pro User
- [x] Ordner (verschachtelt), Umbenennen, Verschieben, Papierkorb mit Wiederherstellen
- [x] Vorschau: Bilder, Video, Audio, PDF, Text; Thumbnails für Bilder (+ Video via ffmpeg, optional)
- [x] Sichtbarkeit: privat / Mitglieder / öffentlich; Freigabe-Links mit Ablaufdatum
- [x] Download mit HTTP-Range (Video-Streaming), sichere Auslieferung (CSP-Sandbox, nosniff)
- [x] Suche, Sortierung, Raster-/Listenansicht, Kontingentanzeige
- [x] Speicher-Backend hinter Trait (lokal; S3 später möglich), SHA-256-Prüfsummen
- [x] Wiederverwendung durch Avatare, Chat-Anhänge, Galerie, Blog-Cover, CMS-Felder, Watch

## Phase 4 – Chat
- [x] Öffentliche + private Kanäle, Direktnachrichten, Gruppen-DMs
- [x] Echtzeit per WebSocket, Verlauf mit Endlos-Scroll nach oben
- [x] Bearbeiten, Löschen, Antworten (Zitat), Reaktionen, Mentions, Markdown
- [x] Datei- und Bildanhänge (über Filebase), Bilder inline
- [x] Tipp-Anzeige, Online-Status, ungelesen-Zähler, „Neue Nachrichten"-Linie
- [x] Mitgliederliste, Kanal-Thema, angepinnte Nachrichten, Suche im Kanal
- [x] Kanäle erstellen/archivieren, einladen, verlassen, Benachrichtigungen stummschalten

## Phase 5 – Blogs
- [x] Blog pro User (Titel, Beschreibung), Übersicht aller Blogs + globaler Feed
- [x] Beiträge: Titel, Slug, Markdown, Auszug, Titelbild, Tags, Entwurf / veröffentlicht / geplant
- [x] Kommentare (verschachtelt) mit Moderation durch Blog-Inhaber, Reaktionen
- [x] Blog abonnieren → Benachrichtigung bei neuen Beiträgen
- [x] RSS-Feeds (pro Blog + global), Lesezeit, Aufrufzähler, Tag-Seiten

## Phase 6 – CMS mit benutzerdefinierten Feldern
- [x] Inhaltstypen mit Feld-Builder: Text, Langtext, Markdown, Zahl, Ja/Nein, Datum, Datum+Zeit,
      Auswahl (einfach/mehrfach), URL, E-Mail, Farbe, Bild, Datei, Relation zu anderen Einträgen
- [x] Pflichtfelder, Standardwerte, Min/Max, Hilfetexte; serverseitige Validierung (JSONB)
- [x] Einträge: Entwurf / veröffentlicht / archiviert, geplante Veröffentlichung, Revisionen + Wiederherstellen
- [x] Rechte pro Typ (wer darf lesen / erstellen / alles bearbeiten)
- [x] Öffentliche Listen- und Detailseiten, Filter/Suche, statische Seiten (Typ „Seite")
- [x] Navigationsmenü-Editor, headless JSON-API (`/api/cms/...`)

## Phase 7 – Galerie
- [x] Alben (persönlich + Community), Sichtbarkeit, Titelbild, Reihenfolge
- [x] Bild-Upload mit automatischen Thumbnails, EXIF (Kamera, Aufnahmedatum; kein GPS), Auto-Rotation
- [x] Lightbox mit Tastatur-Navigation, Diashow, Original-Download
- [x] Kommentare, Reaktionen, Bildbeschreibungen, neueste Bilder

## Phase 8 – Video-/Audio-Calls (SFU: LiveKit)
- [x] Token-Ausgabe durch Axum (JWT), LiveKit als eigener Dienst
- [x] 1:1-Anrufe mit Klingeln (annehmen/ablehnen/verpasst) über den Realtime-Hub
- [x] Räume (dauerhaft + spontan), Sichtbarkeit, Teilnehmerliste mit Live-Anzahl
- [x] Mikro/Kamera an/aus, Bildschirmfreigabe, Geräteauswahl, aktiver Sprecher, Raster-Layout
- [x] Moderation: Teilnehmer stummschalten / entfernen; Chat im Call

## Phase 9 – Watch & Kino (cytu.be-Stil + HQTube-Integration)
HQTube (`hqtube`, bisher Node/React/SQLite) wird nach Rust portiert und hier aufgenommen –
ein Paket, ein Binary, eine Datenbank.
- [x] Räume mit Inhaber + Moderatoren, Sichtbarkeit, Playlist-Regeln (offen / Mitglieder / nur Mods)
- [x] Raumverzeichnis mit „Live"- und „Nächste Vorstellung"-Anzeige
- [x] Quellen: YouTube, direkte Video-URLs, Dateien aus der Filebase, eigene Mediathek
- [x] Server-autoritativer Wiedergabezustand, Drift-Korrektur, Uhrzeit-Abgleich, Nachzügler-Sync
- [x] Playlist: hinzufügen, entfernen, verschieben, als Nächstes; Voteskip mit Schwelle
- [x] Leader-Rolle (übernehmen/übergeben), Raum-Chat mit Emotes, Zuschauerliste
- [x] Mediathek (HQTube): Import aus `incoming/`, ffprobe, Direct-Play oder verlustfreies HLS-Remux
      mit umschaltbaren Tonspuren, Transcode nur wenn nötig, Untertitel → WebVTT, Speicherlimit
- [x] Geplante Vorstellungen (HQTube): Line-up mit Pausen pro Film, Scheduler, neustartsichere Show-Engine
- [x] Showmaster-Konsole: Phase, Timecode, Ablauf, Transport, Overrides (jetzt starten, überspringen,
      ±1 Min, Pause beenden, Show beenden); Vorprogramm-/Pausen-/Ende-Bildschirme mit Countdown
- [x] Raum-Moderation: Slow-Mode, Timeout/Bann im Raum, Flood-Schutz
- [x] Filmwünsche mit Abstimmung
- [x] Popout-Player und -Chat, Audio-Election (nur ein Fenster mit Ton), Join-Overlay (Autoplay)

## Phase 10 – Betrieb
- [ ] README: Entwicklung (Windows) + Produktion (Linux, systemd, Reverse-Proxy mit WebSocket)
- [ ] LiveKit-Konfiguration, Backups (pg_dump + Datenverzeichnis)
- [ ] Hintergrundjobs: geplante Beiträge, Papierkorb leeren, abgelaufene Freigaben, Sessions
- [ ] Tests (Unit + Integrationstests für Kernlogik), clippy sauber
- [ ] Cache-Busting für CSS/JS/WASM im Release-Build (`hash-files`), lange Cache-Header für `/pkg`
- [ ] Release-Paket + Setup-Skript (wie bei HQTube), Push nach github.com/DDQW/wcf (`master`)
