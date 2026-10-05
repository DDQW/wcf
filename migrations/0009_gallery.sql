-- Gallery: albums (personal or community) and images backed by files.

CREATE TABLE gallery_albums (
    id             UUID PRIMARY KEY,
    owner_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title          TEXT NOT NULL,
    description    TEXT NOT NULL DEFAULT '',
    visibility     TEXT NOT NULL DEFAULT 'public',   -- private | members | public
    is_community   BOOLEAN NOT NULL DEFAULT FALSE,   -- all members may add images
    cover_image_id UUID,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX gallery_albums_owner_idx ON gallery_albums (owner_id, updated_at DESC);

CREATE TABLE gallery_images (
    id          UUID PRIMARY KEY,
    album_id    UUID NOT NULL REFERENCES gallery_albums(id) ON DELETE CASCADE,
    owner_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    file_id     UUID NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    title       TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    position    INT NOT NULL DEFAULT 0,
    taken_at    TIMESTAMPTZ,
    view_count  INT NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    search tsvector GENERATED ALWAYS AS (to_tsvector('german', title || ' ' || description)) STORED
);
CREATE INDEX gallery_images_album_idx ON gallery_images (album_id, position, created_at);
CREATE INDEX gallery_images_created_idx ON gallery_images (created_at DESC);
CREATE INDEX gallery_images_search_idx ON gallery_images USING GIN (search);

ALTER TABLE gallery_albums
    ADD CONSTRAINT gallery_albums_cover_fk FOREIGN KEY (cover_image_id) REFERENCES gallery_images(id) ON DELETE SET NULL;
