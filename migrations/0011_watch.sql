-- Watch together / Kino (cytu.be-style rooms + HQTube shows and media library).

CREATE TABLE watch_rooms (
    id               UUID PRIMARY KEY,
    slug             TEXT NOT NULL UNIQUE,
    name             TEXT NOT NULL,
    description      TEXT NOT NULL DEFAULT '',
    owner_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    visibility       TEXT NOT NULL DEFAULT 'members',   -- members | private
    playlist_mode    TEXT NOT NULL DEFAULT 'open',      -- open | members | mods
    slow_mode_ms     BIGINT NOT NULL DEFAULT 0,
    voteskip_percent INT NOT NULL DEFAULT 50,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_active_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Room moderators and (for private rooms / members-only playlists) members.
CREATE TABLE watch_room_roles (
    room_id  UUID NOT NULL REFERENCES watch_rooms(id) ON DELETE CASCADE,
    user_id  UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role     TEXT NOT NULL,                              -- mod | member
    PRIMARY KEY (room_id, user_id)
);

-- Media library (HQTube "movies").
CREATE TABLE watch_media (
    id          UUID PRIMARY KEY,
    title       TEXT NOT NULL,
    source_type TEXT NOT NULL,                           -- hls_local | file_local
    source_path TEXT NOT NULL,
    size_bytes  BIGINT,
    duration_ms BIGINT,
    status      TEXT NOT NULL DEFAULT 'new',             -- new | probing | transcoding | ready | error | archived
    error       TEXT,
    poster_path TEXT,
    probe       JSONB,
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE watch_media_tracks (
    id          UUID PRIMARY KEY,
    media_id    UUID NOT NULL REFERENCES watch_media(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,                           -- audio | sub
    lang        TEXT,
    label       TEXT NOT NULL,
    track_index INT NOT NULL,
    is_default  BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX watch_media_tracks_media_idx ON watch_media_tracks (media_id);

-- Persistent media job queue (resumed after restarts).
CREATE TABLE watch_jobs (
    id         UUID PRIMARY KEY,
    kind       TEXT NOT NULL,                            -- package | direct
    media_id   UUID NOT NULL REFERENCES watch_media(id) ON DELETE CASCADE,
    payload    JSONB NOT NULL DEFAULT '{}',
    status     TEXT NOT NULL DEFAULT 'queued',           -- queued | running | done | error
    progress   DOUBLE PRECISION NOT NULL DEFAULT 0,
    error      TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- cytu.be-style queue: played items are removed.
CREATE TABLE watch_playlist (
    id          UUID PRIMARY KEY,
    room_id     UUID NOT NULL REFERENCES watch_rooms(id) ON DELETE CASCADE,
    position    DOUBLE PRECISION NOT NULL,
    kind        TEXT NOT NULL,                           -- library | youtube | url | file
    media_id    UUID REFERENCES watch_media(id) ON DELETE CASCADE,
    file_id     UUID REFERENCES files(id) ON DELETE CASCADE,
    url         TEXT,
    title       TEXT NOT NULL,
    duration_ms BIGINT,
    thumb_url   TEXT,
    added_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX watch_playlist_room_idx ON watch_playlist (room_id, position);

-- Scheduled shows with a persisted live cursor (restart-safe).
CREATE TABLE watch_events (
    id          UUID PRIMARY KEY,
    room_id     UUID NOT NULL REFERENCES watch_rooms(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    starts_at   BIGINT NOT NULL,                         -- epoch ms
    status      TEXT NOT NULL DEFAULT 'scheduled',       -- scheduled | live | ended | cancelled
    item_index  INT,
    phase       TEXT,                                    -- preshow | playing | paused | intermission
    base_pos_ms BIGINT,
    base_at_ms  BIGINT,                                  -- during intermission: its end
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX watch_events_room_idx ON watch_events (room_id, starts_at);
CREATE INDEX watch_events_due_idx ON watch_events (status, starts_at);

CREATE TABLE watch_event_items (
    id                    UUID PRIMARY KEY,
    event_id              UUID NOT NULL REFERENCES watch_events(id) ON DELETE CASCADE,
    position              INT NOT NULL,
    media_id              UUID NOT NULL REFERENCES watch_media(id),
    intermission_after_ms BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX watch_event_items_event_idx ON watch_event_items (event_id, position);
CREATE INDEX watch_event_items_media_idx ON watch_event_items (media_id);

CREATE TABLE watch_proposals (
    id         UUID PRIMARY KEY,
    room_id    UUID NOT NULL REFERENCES watch_rooms(id) ON DELETE CASCADE,
    title      TEXT NOT NULL,
    notes      TEXT,
    url        TEXT,
    status     TEXT NOT NULL DEFAULT 'open',             -- open | scheduled | rejected
    created_by UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX watch_proposals_room_idx ON watch_proposals (room_id, status);

CREATE TABLE watch_votes (
    proposal_id UUID NOT NULL REFERENCES watch_proposals(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (proposal_id, user_id)
);

CREATE TABLE watch_chat (
    id         UUID PRIMARY KEY,
    room_id    UUID NOT NULL REFERENCES watch_rooms(id) ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body       TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ
);
CREATE INDEX watch_chat_room_idx ON watch_chat (room_id, created_at DESC);

-- Room-level timeouts / bans (latest action per user wins).
CREATE TABLE watch_moderation (
    id             UUID PRIMARY KEY,
    room_id        UUID NOT NULL REFERENCES watch_rooms(id) ON DELETE CASCADE,
    target_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    actor_user_id  UUID REFERENCES users(id) ON DELETE SET NULL,
    kind           TEXT NOT NULL,                        -- timeout | ban | unban | delete_msg
    until_at       TIMESTAMPTZ,
    reason         TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX watch_moderation_idx ON watch_moderation (room_id, target_user_id, created_at DESC);

CREATE TABLE watch_emotes (
    id         UUID PRIMARY KEY,
    code       TEXT NOT NULL UNIQUE,
    file_id    UUID NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
