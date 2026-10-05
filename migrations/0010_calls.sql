-- Calls: persistent/spontaneous rooms and 1:1 calls (media via LiveKit).

CREATE TABLE call_rooms (
    id             UUID PRIMARY KEY,
    owner_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    description    TEXT NOT NULL DEFAULT '',
    visibility     TEXT NOT NULL DEFAULT 'members',  -- members | private
    persistent     BOOLEAN NOT NULL DEFAULT TRUE,     -- spontaneous rooms vanish when empty
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_active_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX call_rooms_active_idx ON call_rooms (last_active_at DESC);

-- Invited members of private rooms and room moderators.
CREATE TABLE call_room_members (
    room_id  UUID NOT NULL REFERENCES call_rooms(id) ON DELETE CASCADE,
    user_id  UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role     TEXT NOT NULL DEFAULT 'member',            -- member | moderator
    added_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (room_id, user_id)
);
CREATE INDEX call_room_members_user_idx ON call_room_members (user_id);

CREATE TABLE direct_calls (
    id          UUID PRIMARY KEY,
    caller_id   UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    callee_id   UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    video       BOOLEAN NOT NULL DEFAULT FALSE,
    status      TEXT NOT NULL DEFAULT 'ringing',        -- ringing | accepted | declined | missed | cancelled | ended
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    answered_at TIMESTAMPTZ,
    ended_at    TIMESTAMPTZ
);
CREATE INDEX direct_calls_caller_idx ON direct_calls (caller_id, created_at DESC);
CREATE INDEX direct_calls_callee_idx ON direct_calls (callee_id, created_at DESC);
