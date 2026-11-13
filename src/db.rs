use std::{path::Path, time::Duration};

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{
    ConnectOptions, Row, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use uuid::Uuid;

use crate::Error;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");
const KEY_PREFIX: &str = "pk_";

/// 같은 파일을 여는 Gateway들이 동일한 인증·제한·사용량 정책을 공유한다.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

// 원문 키가 Debug 출력이나 일반 키 조회에 섞이지 않도록 타입을 분리한다.
#[derive(Serialize)]
pub struct CreatedKey {
    pub id: String,
    pub api_key: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct KeyInfo {
    pub id: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
    pub requests_per_minute: i64,
}

/// total은 아래 다섯 범주의 합이며, 인증 전 401은 키를 확정할 수 없어 제외한다.
#[derive(Serialize, sqlx::FromRow)]
pub struct UsageSummary {
    pub total: i64,
    pub pending: i64,
    pub succeeded: i64,
    pub rejected: i64,
    pub failed: i64,
    pub other_completed: i64,
}

pub(crate) enum Admission {
    Unauthorized,
    Allowed,
    Limited { retry_after: i64 },
}

fn options(path: &Path, create: bool) -> Result<SqliteConnectOptions, Error> {
    if path.as_os_str().is_empty() || path == Path::new(":memory:") {
        return Err(Error::InvalidPath);
    }
    Ok(SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(create)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5))
        .disable_statement_logging())
}

async fn connect(path: &Path, create: bool) -> Result<SqlitePool, Error> {
    Ok(SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options(path, create)?)
        .await?)
}

/// DB 생성과 스키마 변경은 이 명시적 진입점에서만 수행한다.
pub async fn migrate(path: &Path) -> Result<(), Error> {
    options(path, true)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent).await?;
    }
    let pool = connect(path, true).await?;
    // SQLite의 쓰기 잠금을 먼저 확보해 여러 명시적 migrate도 직렬화한다.
    // SQLx 내부 Migration 트랜잭션은 이 바깥 트랜잭션의 savepoint가 된다.
    let result = async {
        let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
        MIGRATOR
            .run(&mut *transaction)
            .await
            .map_err(|_| Error::Migration)?;
        transaction.commit().await?;
        Ok(())
    }
    .await;
    pool.close().await;
    result
}

impl Store {
    /// 기존 DB와 적용된 Migration의 버전·체크섬·필수 열을 읽기만 검증한다.
    pub async fn open(path: &Path) -> Result<Self, Error> {
        let store = Self {
            pool: connect(path, false).await?,
        };
        if let Err(error) = store.validate_schema().await {
            store.close().await;
            return Err(error);
        }
        Ok(store)
    }

    async fn validate_schema(&self) -> Result<(), Error> {
        let applied =
            sqlx::query("SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&self.pool)
                .await
                .map_err(|_| Error::Schema)?;
        if applied.len() != MIGRATOR.iter().len() {
            return Err(Error::Schema);
        }
        for (row, expected) in applied.iter().zip(MIGRATOR.iter()) {
            let version: i64 = row.try_get("version").map_err(|_| Error::Schema)?;
            let checksum: Vec<u8> = row.try_get("checksum").map_err(|_| Error::Schema)?;
            let success: bool = row.try_get("success").map_err(|_| Error::Schema)?;
            if !success || version != expected.version || checksum != expected.checksum.as_ref() {
                return Err(Error::Schema);
            }
        }
        // 이력만 남고 실제 테이블이 손상된 경우도 서버 시작 전에 거부한다.
        for statement in [
            "SELECT id, token_hash, created_at, revoked_at, requests_per_minute FROM api_keys LIMIT 0",
            "SELECT key_id, window_start, admitted FROM rate_windows LIMIT 0",
            "SELECT id, key_id, started_at, status, duration_ms FROM request_usage LIMIT 0",
        ] {
            sqlx::query(statement)
                .execute(&self.pool)
                .await
                .map_err(|_| Error::Schema)?;
        }
        Ok(())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub(crate) async fn ready(&self) -> Result<(), Error> {
        // 상수 SELECT 1이 아니라 실제 인증·제한·사용량 테이블을 읽는다.
        sqlx::query(
            "SELECT EXISTS(SELECT 1 FROM api_keys), \
             EXISTS(SELECT 1 FROM rate_windows), EXISTS(SELECT 1 FROM request_usage)",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_key(&self, requests_per_minute: i64) -> Result<CreatedKey, Error> {
        if requests_per_minute < 1 {
            return Err(Error::InvalidLimit);
        }
        // 사람이 만든 비밀번호가 아니라 OS 난수 256비트이므로 SHA-256 검증값을 쓴다.
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).map_err(|_| Error::Random)?;
        let mut api_key = String::with_capacity(KEY_PREFIX.len() + 64);
        api_key.push_str(KEY_PREFIX);
        let mut encoded = [0_u8; 64];
        hex::encode_to_slice(random, &mut encoded).map_err(|_| Error::Random)?;
        api_key.push_str(std::str::from_utf8(&encoded).map_err(|_| Error::Random)?);
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO api_keys (id, token_hash, created_at, requests_per_minute) \
             VALUES (?, ?, unixepoch(), ?)",
        )
        .bind(&id)
        .bind(token_hash(&api_key).as_slice())
        .bind(requests_per_minute)
        .execute(&self.pool)
        .await?;
        Ok(CreatedKey { id, api_key })
    }

    pub async fn list_keys(&self) -> Result<Vec<KeyInfo>, Error> {
        Ok(sqlx::query_as(
            "SELECT id, created_at, revoked_at, requests_per_minute \
             FROM api_keys ORDER BY created_at, id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn revoke_key(&self, id: &str) -> Result<(), Error> {
        // 폐기는 재실행해도 최초 시각을 보존하며 키·사용량을 삭제하지 않는다.
        let result = sqlx::query(
            "UPDATE api_keys SET revoked_at = COALESCE(revoked_at, unixepoch()) WHERE id = ?",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(Error::KeyNotFound);
        }
        Ok(())
    }

    pub async fn usage(&self, key_id: Option<&str>) -> Result<UsageSummary, Error> {
        if let Some(id) = key_id {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_keys WHERE id = ?)")
                    .bind(id)
                    .fetch_one(&self.pool)
                    .await?;
            if !exists {
                return Err(Error::KeyNotFound);
            }
        }
        // DB 집계 실패를 빈 통계로 바꾸지 않는다. pending도 total에 포함한다.
        Ok(sqlx::query_as(
            "SELECT COUNT(*) AS total, \
             COUNT(CASE WHEN status IS NULL THEN 1 END) AS pending, \
             COUNT(CASE WHEN status BETWEEN 200 AND 299 THEN 1 END) AS succeeded, \
             COUNT(CASE WHEN status BETWEEN 400 AND 499 THEN 1 END) AS rejected, \
             COUNT(CASE WHEN status BETWEEN 500 AND 599 THEN 1 END) AS failed, \
             COUNT(CASE WHEN status BETWEEN 100 AND 199 OR status BETWEEN 300 AND 399 \
                 THEN 1 END) AS other_completed \
             FROM request_usage WHERE ? IS NULL OR key_id = ?",
        )
        .bind(key_id)
        .bind(key_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub(crate) async fn admit(
        &self,
        hash: &[u8; 32],
        request_id: &str,
        started: std::time::Instant,
    ) -> Result<Admission, Error> {
        // 읽기 후 잠금 승격 경합을 피하려고 인증 전에 공유 DB의 쓰기 잠금을 확보한다.
        // 키 확인·쿼터 증가·접수 기록이 함께 커밋되어 두 프로세스도 한도를 우회하지 못한다.
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let key: Option<(String, i64)> = sqlx::query_as(
            "SELECT id, requests_per_minute FROM api_keys \
             WHERE token_hash = ? AND revoked_at IS NULL",
        )
        .bind(hash.as_slice())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((key_id, limit)) = key else {
            transaction.rollback().await?;
            return Ok(Admission::Unauthorized);
        };
        // 쓰기 잠금 대기 이후 DB 시계를 읽는다. 경계 직후 요청을 이전 분에 넣지 않는다.
        // 고정 UTC 분 경계에서는 앞뒤 각 한도만큼의 burst가 가능하다.
        let now: i64 = sqlx::query_scalar("SELECT unixepoch()")
            .fetch_one(&mut *transaction)
            .await?;
        let window_start = now - now.rem_euclid(60);
        let counter = sqlx::query(
            "INSERT INTO rate_windows (key_id, window_start, admitted) VALUES (?, ?, 1) \
             ON CONFLICT(key_id) DO UPDATE SET \
                 window_start = excluded.window_start, \
                 admitted = CASE WHEN rate_windows.window_start = excluded.window_start \
                     THEN rate_windows.admitted + 1 ELSE 1 END \
             WHERE rate_windows.window_start <> excluded.window_start \
                 OR rate_windows.admitted < ?",
        )
        .bind(&key_id)
        .bind(window_start)
        .bind(limit)
        .execute(&mut *transaction)
        .await?;
        let limited = counter.rows_affected() == 0;
        sqlx::query(
            "INSERT INTO request_usage (id, key_id, started_at, status, duration_ms) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(request_id)
        .bind(key_id)
        .bind(now)
        .bind(limited.then_some(429_i64))
        .bind(limited.then(|| elapsed_ms(started)))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        if limited {
            Ok(Admission::Limited {
                retry_after: 60 - now.rem_euclid(60),
            })
        } else {
            Ok(Admission::Allowed)
        }
    }

    pub(crate) async fn complete(
        &self,
        request_id: &str,
        status: u16,
        duration_ms: i64,
    ) -> Result<(), Error> {
        // 단일 UPDATE 실패 시 기존 pending은 유지된다. 호출자는 성공 대신 503을 보낸다.
        // DB 커밋과 HTTP 전달은 원자적이지 않으며, 중단·커밋 결과 불명은 성공 추정 금지다.
        let result = sqlx::query(
            "UPDATE request_usage SET status = ?, duration_ms = ? WHERE id = ? AND status IS NULL",
        )
        .bind(i64::from(status))
        .bind(duration_ms)
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(Error::Database);
        }
        Ok(())
    }
}

pub(crate) fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

pub(crate) fn valid_token_format(token: &str) -> bool {
    token.strip_prefix(KEY_PREFIX).is_some_and(|secret| {
        secret.len() == 64
            && secret
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

pub(crate) fn elapsed_ms(started: std::time::Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}
