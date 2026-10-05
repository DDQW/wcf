-- Full forum: subcategories, permissions, tags, soft deletes, edit history,
-- read markers, subscriptions and full-text search.

ALTER TABLE forum_categories
    ADD COLUMN parent_id   UUID REFERENCES forum_categories(id) ON DELETE SET NULL,
    ADD COLUMN color       TEXT NOT NULL DEFAULT '#2563eb',
    ADD COLUMN read_access TEXT NOT NULL DEFAULT 'public',
    ADD COLUMN post_role   TEXT NOT NULL DEFAULT 'member',
    ADD COLUMN updated_at  TIMESTAMPTZ NOT NULL DEFAULT now();

ALTER TABLE forum_topics
    ADD COLUMN tags           TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN view_count     INT NOT NULL DEFAULT 0,
    ADD COLUMN is_deleted     BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN deleted_by     UUID REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN last_poster_id UUID REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN first_post_id  UUID,
    ADD COLUMN search tsvector GENERATED ALWAYS AS (to_tsvector('german', title)) STORED;

CREATE INDEX forum_topics_tags_idx ON forum_topics USING GIN (tags);
CREATE INDEX forum_topics_search_idx ON forum_topics USING GIN (search);
CREATE INDEX forum_topics_activity_idx ON forum_topics (last_post_at DESC, id DESC);
CREATE INDEX forum_topics_created_idx ON forum_topics (created_at DESC, id DESC);
CREATE INDEX forum_topics_author_idx ON forum_topics (author_id, created_at DESC);

UPDATE forum_topics t
SET first_post_id = (
    SELECT p.id FROM forum_posts p
    WHERE p.topic_id = t.id AND p.parent_post_id IS NULL
    ORDER BY p.created_at, p.id LIMIT 1
);
UPDATE forum_topics t
SET last_poster_id = (
    SELECT p.author_id FROM forum_posts p
    WHERE p.topic_id = t.id
    ORDER BY p.created_at DESC, p.id DESC LIMIT 1
);

ALTER TABLE forum_posts
    ADD COLUMN edited_at  TIMESTAMPTZ,
    ADD COLUMN edit_count INT NOT NULL DEFAULT 0,
    ADD COLUMN deleted_by UUID REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN search tsvector GENERATED ALWAYS AS (to_tsvector('german', body_md)) STORED;

CREATE INDEX forum_posts_search_idx ON forum_posts USING GIN (search);
CREATE INDEX forum_posts_author_idx ON forum_posts (author_id, created_at DESC);
CREATE INDEX forum_posts_thread_idx ON forum_posts (thread_root_id, created_at, id);

CREATE TABLE forum_post_revisions (
    id         UUID PRIMARY KEY,
    post_id    UUID NOT NULL REFERENCES forum_posts(id) ON DELETE CASCADE,
    body_md    TEXT NOT NULL,
    body_html  TEXT NOT NULL,
    editor_id  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX forum_post_revisions_post_idx ON forum_post_revisions (post_id, created_at DESC);

CREATE TABLE forum_topic_reads (
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    topic_id     UUID NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    last_read_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, topic_id)
);

CREATE TABLE forum_topic_subs (
    user_id  UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    topic_id UUID NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    level    TEXT NOT NULL,
    PRIMARY KEY (user_id, topic_id)
);
CREATE INDEX forum_topic_subs_watch_idx ON forum_topic_subs (topic_id) WHERE level = 'watching';
