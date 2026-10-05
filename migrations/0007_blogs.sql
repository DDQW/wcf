-- Blogs per user, blog posts, generic comments and blog subscriptions.

CREATE TABLE blogs (
    id          UUID PRIMARY KEY,
    owner_id    UUID NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE blog_posts (
    id              UUID PRIMARY KEY,
    blog_id         UUID NOT NULL REFERENCES blogs(id) ON DELETE CASCADE,
    author_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title           TEXT NOT NULL,
    slug            TEXT NOT NULL,
    excerpt         TEXT NOT NULL DEFAULT '',
    body_md         TEXT NOT NULL,
    body_html       TEXT NOT NULL,
    cover_file_id   UUID REFERENCES files(id) ON DELETE SET NULL,
    tags            TEXT[] NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL DEFAULT 'draft',   -- draft | published
    published_at    TIMESTAMPTZ,                     -- in the future = scheduled
    notified        BOOLEAN NOT NULL DEFAULT FALSE,
    allow_comments  BOOLEAN NOT NULL DEFAULT TRUE,
    view_count      INT NOT NULL DEFAULT 0,
    reading_minutes INT NOT NULL DEFAULT 1,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    search tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('german', title), 'A') || setweight(to_tsvector('german', body_md), 'B')
    ) STORED,
    UNIQUE (blog_id, slug)
);
CREATE INDEX blog_posts_published_idx ON blog_posts (published_at DESC, id DESC) WHERE status = 'published';
CREATE INDEX blog_posts_blog_idx ON blog_posts (blog_id, published_at DESC);
CREATE INDEX blog_posts_tags_idx ON blog_posts USING GIN (tags);
CREATE INDEX blog_posts_search_idx ON blog_posts USING GIN (search);

CREATE TABLE blog_subscriptions (
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blog_id    UUID NOT NULL REFERENCES blogs(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, blog_id)
);

-- Generic threaded comments for blog posts, gallery images, CMS entries …
CREATE TABLE comments (
    id           UUID PRIMARY KEY,
    subject_type TEXT NOT NULL,
    subject_id   UUID NOT NULL,
    author_id    UUID REFERENCES users(id) ON DELETE SET NULL,
    parent_id    UUID REFERENCES comments(id) ON DELETE CASCADE,
    body_md      TEXT NOT NULL,
    body_html    TEXT NOT NULL,
    edited_at    TIMESTAMPTZ,
    deleted_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX comments_subject_idx ON comments (subject_type, subject_id, created_at);
