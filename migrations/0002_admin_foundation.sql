-- 기존 키·사용량을 재작성하거나 의미를 추정하지 않고 NULL 확장한다.
ALTER TABLE api_keys ADD COLUMN name TEXT
    CHECK (name IS NULL OR length(name) BETWEEN 1 AND 128);
ALTER TABLE request_usage ADD COLUMN request_kind TEXT
    CHECK (request_kind IN ('inference', 'models', 'auth_check', 'admin'));
ALTER TABLE request_usage ADD COLUMN input_tokens INTEGER;
ALTER TABLE request_usage ADD COLUMN output_tokens INTEGER
    CHECK (
        (input_tokens IS NULL AND output_tokens IS NULL)
        OR (input_tokens IS NOT NULL AND output_tokens IS NOT NULL
            AND input_tokens >= 0 AND output_tokens >= 0
            AND request_kind IS NOT NULL AND request_kind = 'inference'
            AND status IS NOT NULL AND status BETWEEN 200 AND 299)
    );
CREATE INDEX request_usage_time ON request_usage (started_at);
CREATE INDEX request_usage_kind_time ON request_usage (request_kind, started_at);

-- 로그인 런타임은 후속 단계에서 구현한다. 여기서는 해시·시간·식별자 경계만 제공한다.
CREATE TABLE admin_sessions (
    id TEXT PRIMARY KEY NOT NULL CHECK (
        length(id) = 36 AND substr(id, 9, 1) = '-' AND substr(id, 14, 1) = '-'
        AND substr(id, 19, 1) = '-' AND substr(id, 24, 1) = '-'
        AND length(replace(id, '-', '')) = 32
        AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    csrf_token_hash BLOB NOT NULL CHECK (length(csrf_token_hash) = 32),
    google_sub TEXT NOT NULL CHECK (length(google_sub) > 0),
    admin_email_hash BLOB NOT NULL CHECK (length(admin_email_hash) = 32),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    expires_at INTEGER NOT NULL CHECK (expires_at > created_at),
    revoked_at INTEGER CHECK (revoked_at IS NULL OR revoked_at >= created_at)
) STRICT;
CREATE INDEX admin_sessions_expiry ON admin_sessions (expires_at);

CREATE TABLE admin_login_states (
    state_hash BLOB PRIMARY KEY NOT NULL CHECK (length(state_hash) = 32),
    nonce_hash BLOB NOT NULL CHECK (length(nonce_hash) = 32),
    browser_binding_hash BLOB NOT NULL CHECK (length(browser_binding_hash) = 32),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    expires_at INTEGER NOT NULL CHECK (expires_at > created_at),
    consumed_at INTEGER CHECK (
        consumed_at IS NULL OR (consumed_at >= created_at AND consumed_at < expires_at)
    )
) STRICT;
CREATE INDEX admin_login_states_expiry ON admin_login_states (expires_at);

CREATE TABLE admin_events (
    id TEXT PRIMARY KEY NOT NULL CHECK (
        length(id) = 36 AND substr(id, 9, 1) = '-' AND substr(id, 14, 1) = '-'
        AND substr(id, 19, 1) = '-' AND substr(id, 24, 1) = '-'
        AND length(replace(id, '-', '')) = 32
        AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    action TEXT NOT NULL CHECK (action IN ('key_created', 'key_revoked')),
    key_id TEXT NOT NULL REFERENCES api_keys(id),
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('cli', 'admin')),
    -- 세션 삭제 후에도 역사 식별자를 보존하므로 FK로 묶지 않는다.
    actor_session_id TEXT CHECK (
        (actor_kind = 'cli' AND actor_session_id IS NULL)
        OR (actor_kind = 'admin' AND actor_session_id IS NOT NULL
            AND length(actor_session_id) = 36
            AND substr(actor_session_id, 9, 1) = '-' AND substr(actor_session_id, 14, 1) = '-'
            AND substr(actor_session_id, 19, 1) = '-' AND substr(actor_session_id, 24, 1) = '-'
            AND length(replace(actor_session_id, '-', '')) = 32
            AND replace(actor_session_id, '-', '') NOT GLOB '*[^0-9a-f]*')
    ),
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;
CREATE INDEX admin_events_key_time ON admin_events (key_id, created_at);
