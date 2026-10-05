CREATE TABLE forum_categories (
    id          UUID PRIMARY KEY,
    slug        TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    description TEXT,
    position    INT NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE forum_topics (
    id           UUID PRIMARY KEY,
    category_id  UUID NOT NULL REFERENCES forum_categories(id),
    author_id    UUID NOT NULL REFERENCES users(id),
    title        TEXT NOT NULL,
    slug         TEXT NOT NULL,
    is_pinned    BOOLEAN NOT NULL DEFAULT FALSE,
    is_locked    BOOLEAN NOT NULL DEFAULT FALSE,
    reply_count  INT NOT NULL DEFAULT 0,
    last_post_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX ON forum_topics(category_id, slug);
CREATE INDEX ON forum_topics(category_id, last_post_at DESC, id DESC);

CREATE TABLE forum_posts (
    id              UUID PRIMARY KEY,
    topic_id        UUID NOT NULL REFERENCES forum_topics(id),
    author_id       UUID NOT NULL REFERENCES users(id),
    parent_post_id  UUID REFERENCES forum_posts(id),
    thread_root_id  UUID NOT NULL,
    depth           SMALLINT NOT NULL DEFAULT 0,
    body_md         TEXT NOT NULL,
    body_html       TEXT NOT NULL,
    is_deleted      BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON forum_posts(topic_id, created_at, id);
CREATE INDEX ON forum_posts(topic_id, thread_root_id, created_at);
