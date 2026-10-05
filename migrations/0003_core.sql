-- Platform core: profiles, moderation, notifications, reactions, reports,
-- audit log, password resets and site settings.

ALTER TABLE users
    ADD COLUMN bio_md              TEXT NOT NULL DEFAULT '',
    ADD COLUMN bio_html            TEXT NOT NULL DEFAULT '',
    ADD COLUMN location            TEXT,
    ADD COLUMN website             TEXT,
    ADD COLUMN avatar_file_id      UUID,
    ADD COLUMN banned_until        TIMESTAMPTZ,
    ADD COLUMN ban_reason          TEXT,
    ADD COLUMN last_seen_at        TIMESTAMPTZ,
    ADD COLUMN theme               TEXT NOT NULL DEFAULT 'system',
    ADD COLUMN notify_prefs        JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN storage_quota_bytes BIGINT;

ALTER TABLE users
    ADD COLUMN search tsvector GENERATED ALWAYS AS
        (to_tsvector('simple', username::text || ' ' || display_name)) STORED;
CREATE INDEX users_search_idx ON users USING GIN (search);
CREATE INDEX users_created_idx ON users (created_at DESC);

-- Exactly one row holding the admin-editable configuration as JSON.
CREATE TABLE site_settings (
    id         BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    data       JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Bumped when the markdown renderer changes so stored HTML is rebuilt.
    render_version INT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO site_settings (id) VALUES (TRUE);

CREATE TABLE notifications (
    id           UUID PRIMARY KEY,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    actor_id     UUID REFERENCES users(id) ON DELETE SET NULL,
    title        TEXT NOT NULL,
    body         TEXT NOT NULL DEFAULT '',
    link         TEXT NOT NULL DEFAULT '',
    subject_type TEXT,
    subject_id   UUID,
    is_read      BOOLEAN NOT NULL DEFAULT FALSE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX notifications_user_idx ON notifications (user_id, created_at DESC, id DESC);
CREATE INDEX notifications_unread_idx ON notifications (user_id) WHERE NOT is_read;

CREATE TABLE reactions (
    subject_type TEXT NOT NULL,
    subject_id   UUID NOT NULL,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    emoji        TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (subject_type, subject_id, user_id, emoji)
);
CREATE INDEX reactions_subject_idx ON reactions (subject_type, subject_id);

CREATE TABLE reports (
    id              UUID PRIMARY KEY,
    reporter_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    subject_type    TEXT NOT NULL,
    subject_id      UUID NOT NULL,
    reason          TEXT NOT NULL,
    details         TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL DEFAULT 'open',
    resolved_by     UUID REFERENCES users(id) ON DELETE SET NULL,
    resolution_note TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at     TIMESTAMPTZ
);
CREATE INDEX reports_status_idx ON reports (status, created_at DESC);
-- One open report per user and subject.
CREATE UNIQUE INDEX reports_open_unique ON reports (reporter_id, subject_type, subject_id)
    WHERE status = 'open';

CREATE TABLE mod_log (
    id             UUID PRIMARY KEY,
    actor_id       UUID REFERENCES users(id) ON DELETE SET NULL,
    action         TEXT NOT NULL,
    subject_type   TEXT,
    subject_id     UUID,
    target_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    details        TEXT NOT NULL DEFAULT '',
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mod_log_created_idx ON mod_log (created_at DESC, id DESC);

CREATE TABLE password_resets (
    token_hash TEXT PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
