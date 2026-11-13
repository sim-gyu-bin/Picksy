-- 기존 세션에는 설정 결합 증거가 없으므로 NULL을 유지하고 HTTP 인증에서 거부한다.
ALTER TABLE admin_sessions ADD COLUMN config_hash BLOB
    CHECK (config_hash IS NULL OR length(config_hash) = 32);

-- 클라이언트 IP·헤더를 신뢰하지 않는 고정 버킷으로 복제본 전체의 관리 부하를 제한한다.
CREATE TABLE admin_rate_windows (
    bucket TEXT PRIMARY KEY NOT NULL CHECK (bucket IN ('login', 'callback', 'admin')),
    window_start INTEGER NOT NULL CHECK (window_start >= 0),
    admitted INTEGER NOT NULL CHECK (admitted > 0)
) STRICT;
