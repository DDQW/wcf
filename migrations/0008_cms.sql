-- CMS: content types with custom fields (JSONB), entries, revisions and
-- the site navigation menu.

CREATE TABLE cms_types (
    id             UUID PRIMARY KEY,
    slug           TEXT NOT NULL UNIQUE,
    name           TEXT NOT NULL,
    name_plural    TEXT NOT NULL,
    description    TEXT NOT NULL DEFAULT '',
    icon           TEXT NOT NULL DEFAULT 'layout',
    fields         JSONB NOT NULL DEFAULT '[]'::jsonb,
    read_access    TEXT NOT NULL DEFAULT 'public',
    create_role    TEXT NOT NULL DEFAULT 'moderator',
    allow_comments BOOLEAN NOT NULL DEFAULT FALSE,
    list_layout    TEXT NOT NULL DEFAULT 'cards',
    is_page_type   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE cms_entries (
    id           UUID PRIMARY KEY,
    type_id      UUID NOT NULL REFERENCES cms_types(id) ON DELETE CASCADE,
    slug         TEXT NOT NULL,
    title        TEXT NOT NULL,
    data         JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Pre-rendered HTML of markdown fields, keyed like `data`.
    rendered     JSONB NOT NULL DEFAULT '{}'::jsonb,
    status       TEXT NOT NULL DEFAULT 'draft',   -- draft | published | archived
    published_at TIMESTAMPTZ,
    author_id    UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    search tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('german', title), 'A')
        || setweight(jsonb_to_tsvector('german', data, '["string"]'), 'B')
    ) STORED,
    UNIQUE (type_id, slug)
);
CREATE INDEX cms_entries_type_idx ON cms_entries (type_id, status, published_at DESC);
CREATE INDEX cms_entries_data_idx ON cms_entries USING GIN (data jsonb_path_ops);
CREATE INDEX cms_entries_search_idx ON cms_entries USING GIN (search);

CREATE TABLE cms_revisions (
    id         UUID PRIMARY KEY,
    entry_id   UUID NOT NULL REFERENCES cms_entries(id) ON DELETE CASCADE,
    title      TEXT NOT NULL,
    data       JSONB NOT NULL,
    status     TEXT NOT NULL,
    editor_id  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX cms_revisions_entry_idx ON cms_revisions (entry_id, created_at DESC);

CREATE TABLE cms_menu (
    id         UUID PRIMARY KEY,
    label      TEXT NOT NULL,
    url        TEXT NOT NULL,
    position   INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Built-in content type for static pages (served at /p/<slug>).
INSERT INTO cms_types (id, slug, name, name_plural, description, icon, fields, is_page_type, create_role)
VALUES (
    '01a00000-0000-7000-8000-0000000000c1', 'seite', 'Seite', 'Seiten',
    'Statische Seiten wie Impressum, Regeln oder Über uns.', 'file-text',
    '[{"key":"inhalt","label":"Inhalt","kind":"markdown","required":true,"help":"","options":[],"min":null,"max":null,"relation_type":null,"multiple":false,"show_in_list":false,"default":null}]'::jsonb,
    TRUE, 'admin'
);
