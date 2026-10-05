-- Filebase: folders, files (content-addressed blobs on disk), share links.

CREATE TABLE folders (
    id         UUID PRIMARY KEY,
    owner_id   UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    parent_id  UUID REFERENCES folders(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX folders_owner_idx ON folders (owner_id, parent_id);

CREATE TABLE files (
    id             UUID PRIMARY KEY,
    owner_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    folder_id      UUID REFERENCES folders(id) ON DELETE SET NULL,
    name           TEXT NOT NULL,
    storage_key    TEXT NOT NULL,
    size_bytes     BIGINT NOT NULL,
    mime_type      TEXT NOT NULL,
    sha256         TEXT NOT NULL,
    width          INT,
    height         INT,
    duration_secs  DOUBLE PRECISION,
    has_thumb      BOOLEAN NOT NULL DEFAULT FALSE,
    meta           JSONB NOT NULL DEFAULT '{}'::jsonb,
    visibility     TEXT NOT NULL DEFAULT 'private',
    purpose        TEXT NOT NULL DEFAULT 'drive',
    context_id     UUID,
    description    TEXT NOT NULL DEFAULT '',
    download_count INT NOT NULL DEFAULT 0,
    deleted_at     TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    search tsvector GENERATED ALWAYS AS (to_tsvector('simple', name || ' ' || description)) STORED
);
CREATE INDEX files_owner_folder_idx ON files (owner_id, folder_id) WHERE deleted_at IS NULL;
CREATE INDEX files_owner_purpose_idx ON files (owner_id, purpose, created_at DESC);
CREATE INDEX files_storage_key_idx ON files (storage_key);
CREATE INDEX files_sha_idx ON files (sha256, size_bytes);
CREATE INDEX files_deleted_idx ON files (deleted_at) WHERE deleted_at IS NOT NULL;
CREATE INDEX files_search_idx ON files USING GIN (search);

CREATE TABLE file_shares (
    id             UUID PRIMARY KEY,
    token          TEXT NOT NULL UNIQUE,
    file_id        UUID REFERENCES files(id) ON DELETE CASCADE,
    folder_id      UUID REFERENCES folders(id) ON DELETE CASCADE,
    created_by     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at     TIMESTAMPTZ,
    password_hash  TEXT,
    max_downloads  INT,
    download_count INT NOT NULL DEFAULT 0,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (file_id IS NOT NULL OR folder_id IS NOT NULL)
);
CREATE INDEX file_shares_file_idx ON file_shares (file_id);
CREATE INDEX file_shares_folder_idx ON file_shares (folder_id);

ALTER TABLE users
    ADD CONSTRAINT users_avatar_fk FOREIGN KEY (avatar_file_id) REFERENCES files(id) ON DELETE SET NULL;
