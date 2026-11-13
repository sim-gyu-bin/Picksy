-- 원문 키가 아니라 OS 난수 토큰의 SHA-256 검증값만 영구 저장한다.
CREATE TABLE api_keys (
    id TEXT PRIMARY KEY NOT NULL,
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    revoked_at INTEGER CHECK (revoked_at IS NULL OR revoked_at >= 0),
    requests_per_minute INTEGER NOT NULL CHECK (requests_per_minute >= 1)
) STRICT;

-- 프로세스마다 카운터를 만들지 않고 공유 DB의 키별 현재 UTC 분을 갱신한다.
CREATE TABLE rate_windows (
    key_id TEXT PRIMARY KEY NOT NULL REFERENCES api_keys(id),
    window_start INTEGER NOT NULL CHECK (window_start >= 0 AND window_start % 60 = 0),
    admitted INTEGER NOT NULL CHECK (admitted >= 1)
) STRICT;

-- 상태와 소요 시간이 NULL이면 접수 후 완료 기록이 없는 pending 요청이다.
-- HTTP 전송과 DB 완료를 원자화할 수 없으므로 pending을 성공으로 추정하지 않는다.
CREATE TABLE request_usage (
    id TEXT PRIMARY KEY NOT NULL,
    key_id TEXT NOT NULL REFERENCES api_keys(id),
    started_at INTEGER NOT NULL CHECK (started_at >= 0),
    status INTEGER,
    duration_ms INTEGER,
    CHECK (
        (status IS NULL AND duration_ms IS NULL)
        OR (status IS NOT NULL AND status BETWEEN 100 AND 599
            AND duration_ms IS NOT NULL AND duration_ms >= 0)
    )
) STRICT;

CREATE INDEX request_usage_key_time ON request_usage (key_id, started_at);

-- 사용량은 자동 삭제하지 않는다. 보존 기간과 명시적 정리는 별도 승인 정책으로 정한다.
