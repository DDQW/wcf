-- Chat: channels (public/private), direct messages, group DMs, messages.

CREATE TABLE chat_channels (
    id              UUID PRIMARY KEY,
    kind            TEXT NOT NULL,              -- public | private | direct | group | room
    name            TEXT NOT NULL DEFAULT '',
    topic           TEXT NOT NULL DEFAULT '',
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    is_archived     BOOLEAN NOT NULL DEFAULT FALSE,
    -- "<uuid>:<uuid>" (sorted) for direct messages, so each pair has one channel.
    dm_key          TEXT UNIQUE,
    last_message_at TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX chat_channels_kind_idx ON chat_channels (kind, last_message_at DESC);

CREATE TABLE chat_members (
    channel_id   UUID NOT NULL REFERENCES chat_channels(id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role         TEXT NOT NULL DEFAULT 'member',  -- owner | admin | member
    last_read_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    muted        BOOLEAN NOT NULL DEFAULT FALSE,
    hidden       BOOLEAN NOT NULL DEFAULT FALSE,  -- DMs closed in the sidebar
    joined_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);
CREATE INDEX chat_members_user_idx ON chat_members (user_id);

CREATE TABLE chat_messages (
    id          UUID PRIMARY KEY,
    channel_id  UUID NOT NULL REFERENCES chat_channels(id) ON DELETE CASCADE,
    author_id   UUID REFERENCES users(id) ON DELETE SET NULL,
    kind        TEXT NOT NULL DEFAULT 'text',     -- text | system
    body_md     TEXT NOT NULL,
    body_html   TEXT NOT NULL,
    reply_to_id UUID REFERENCES chat_messages(id) ON DELETE SET NULL,
    attachments UUID[] NOT NULL DEFAULT '{}',
    mentions    UUID[] NOT NULL DEFAULT '{}',
    edited_at   TIMESTAMPTZ,
    deleted_at  TIMESTAMPTZ,
    pinned_at   TIMESTAMPTZ,
    pinned_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    search tsvector GENERATED ALWAYS AS (to_tsvector('german', body_md)) STORED
);
CREATE INDEX chat_messages_channel_idx ON chat_messages (channel_id, created_at DESC, id DESC);
CREATE INDEX chat_messages_pinned_idx ON chat_messages (channel_id) WHERE pinned_at IS NOT NULL;
CREATE INDEX chat_messages_mentions_idx ON chat_messages USING GIN (mentions);
CREATE INDEX chat_messages_search_idx ON chat_messages USING GIN (search);

-- A default public channel so the chat is not empty.
INSERT INTO chat_channels (id, kind, name, topic, last_message_at)
VALUES ('01a00000-0000-7000-8000-000000000001', 'public', 'allgemein', 'Alles, was sonst nirgends hinpasst', now());
