-- Invite codes: registration can require a code (ported from HQTube).

CREATE TABLE invite_codes (
    code       TEXT PRIMARY KEY,
    created_by UUID REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ,
    max_uses   INT NOT NULL DEFAULT 1,
    uses       INT NOT NULL DEFAULT 0,
    note       TEXT NOT NULL DEFAULT '',
    revoked_at TIMESTAMPTZ
);
CREATE INDEX invite_codes_creator_idx ON invite_codes (created_by, created_at DESC);

CREATE TABLE invite_uses (
    code    TEXT NOT NULL REFERENCES invite_codes(code) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (code, user_id)
);
