use std::{path::Path, time::Duration};

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{
    ConnectOptions, QueryBuilder, Row, Sqlite, SqlitePool,
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
    pub(crate) pool: SqlitePool,
}

// 원문 키가 Debug 출력이나 일반 키 조회에 섞이지 않도록 타입을 분리한다.
#[derive(Serialize)]
pub struct CreatedKey {
    pub id: String,
    pub api_key: String,
    pub name: Option<String>,
    pub display_name: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct KeyInfo {
    pub id: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
    pub requests_per_minute: i64,
    pub name: Option<String>,
    pub display_name: String,
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
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub token_recorded: i64,
    pub token_unknown: i64,
}

/// HTTP 인증 뒤에도 DB 변경 트랜잭션에서 세션 만료·폐기를 재검사한다. 감사에는 식별자만 남긴다.
#[derive(Clone, Copy)]
pub enum KeyActor {
    Cli,
    Admin { session_id: Uuid },
}

/// Unknown은 과거 NULL 분류이며 실제 요청의 종류를 추정하지 않는다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum RequestKind {
    Inference,
    Models,
    AuthCheck,
    Admin,
    Unknown,
}

impl RequestKind {
    fn stored(self) -> Option<&'static str> {
        match self {
            Self::Inference => Some("inference"),
            Self::Models => Some("models"),
            Self::AuthCheck => Some("auth_check"),
            Self::Admin => Some("admin"),
            Self::Unknown => None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// UTC Unix 초의 반열린 구간 [from, until)을 모든 집계에 동일하게 적용한다.
#[derive(Clone, Copy, Default)]
pub struct UsageFilter<'a> {
    pub key_id: Option<&'a str>,
    pub from: Option<i64>,
    pub until: Option<i64>,
    pub kind: Option<RequestKind>,
}

#[derive(Serialize)]
pub struct UsageReport {
    pub summary: UsageSummary,
    pub by_day: Vec<DailyUsage>,
    pub by_kind: Vec<KindUsage>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct DailyUsage {
    pub day: String,
    #[sqlx(flatten)]
    pub summary: UsageSummary,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct KindUsage {
    pub kind: RequestKind,
    #[sqlx(flatten)]
    pub summary: UsageSummary,
}

// SUM을 COALESCE하지 않아 미수집과 실제 0을 구별하고 정수 초과 오류를 보존한다.
const USAGE_AGGREGATES: &str = "COUNT(*) AS total, \
     COUNT(CASE WHEN status IS NULL THEN 1 END) AS pending, \
     COUNT(CASE WHEN status BETWEEN 200 AND 299 THEN 1 END) AS succeeded, \
     COUNT(CASE WHEN status BETWEEN 400 AND 499 THEN 1 END) AS rejected, \
     COUNT(CASE WHEN status BETWEEN 500 AND 599 THEN 1 END) AS failed, \
     COUNT(CASE WHEN status BETWEEN 100 AND 199 OR status BETWEEN 300 AND 399 \
         THEN 1 END) AS other_completed, \
     SUM(CASE WHEN request_kind = 'inference' THEN input_tokens END) AS input_tokens, \
     SUM(CASE WHEN request_kind = 'inference' THEN output_tokens END) AS output_tokens, \
     COUNT(CASE WHEN request_kind = 'inference' AND input_tokens IS NOT NULL \
         THEN 1 END) AS token_recorded, \
     COUNT(CASE WHEN request_kind = 'inference' AND input_tokens IS NULL \
         THEN 1 END) AS token_unknown";

fn usage_query<'a>(prefix: &str, filter: UsageFilter<'a>) -> QueryBuilder<'a, Sqlite> {
    let mut query = QueryBuilder::new(prefix);
    query.push(USAGE_AGGREGATES);
    query.push(" FROM request_usage WHERE 1 = 1");
    // 입력은 SQL에 이어 붙이지 않고 조건별 bind로 전달해 인덱스 활용을 유지한다.
    if let Some(key_id) = filter.key_id {
        query.push(" AND key_id = ").push_bind(key_id);
    }
    if let Some(from) = filter.from {
        query.push(" AND started_at >= ").push_bind(from);
    }
    if let Some(until) = filter.until {
        query.push(" AND started_at < ").push_bind(until);
    }
    if let Some(kind) = filter.kind {
        if let Some(value) = kind.stored() {
            query.push(" AND request_kind = ").push_bind(value);
        } else {
            query.push(" AND request_kind IS NULL");
        }
    }
    query
}

async fn record_key_event(
    connection: &mut sqlx::SqliteConnection,
    action: &str,
    key_id: &str,
    actor: KeyActor,
) -> Result<(), Error> {
    let (kind, session_id) = match actor {
        KeyActor::Cli => ("cli", None),
        KeyActor::Admin { session_id } => ("admin", Some(session_id.to_string())),
    };
    sqlx::query(
        "INSERT INTO admin_events (id, action, key_id, actor_kind, actor_session_id, created_at) \
         VALUES (?, ?, ?, ?, ?, unixepoch())",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(action)
    .bind(key_id)
    .bind(kind)
    .bind(session_id)
    .execute(connection)
    .await?;
    Ok(())
}

async fn validate_key_actor(
    connection: &mut sqlx::SqliteConnection,
    actor: KeyActor,
) -> Result<(), Error> {
    if let KeyActor::Admin { session_id } = actor {
        // HTTP 검사 뒤 만료·로그아웃과 경합해도 키 변경 트랜잭션에서 다시 거부한다.
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM admin_sessions WHERE id = ? \
             AND revoked_at IS NULL AND created_at <= unixepoch() AND expires_at > unixepoch())",
        )
        .bind(session_id.to_string())
        .fetch_one(connection)
        .await?;
        if !active {
            return Err(Error::AdminUnauthorized);
        }
    }
    Ok(())
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
            "SELECT id, token_hash, created_at, revoked_at, requests_per_minute, name FROM api_keys LIMIT 0",
            "SELECT key_id, window_start, admitted FROM rate_windows LIMIT 0",
            "SELECT id, key_id, started_at, status, duration_ms, request_kind, input_tokens, output_tokens FROM request_usage LIMIT 0",
            "SELECT id, token_hash, csrf_token_hash, google_sub, admin_email_hash, created_at, expires_at, revoked_at FROM admin_sessions LIMIT 0",
            "SELECT state_hash, nonce_hash, browser_binding_hash, created_at, expires_at, consumed_at FROM admin_login_states LIMIT 0",
            "SELECT id, action, key_id, actor_kind, actor_session_id, created_at FROM admin_events LIMIT 0",
            "SELECT config_hash FROM admin_sessions LIMIT 0",
            "SELECT bucket, window_start, admitted FROM admin_rate_windows LIMIT 0",
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
        // 실제 필수 테이블과 신규 열을 읽어 적용 후 스키마 손상도 준비 실패로 전파한다.
        sqlx::query(
            "SELECT EXISTS(SELECT name FROM api_keys), \
             EXISTS(SELECT 1 FROM rate_windows), \
             EXISTS(SELECT request_kind, input_tokens, output_tokens FROM request_usage), \
             EXISTS(SELECT id, token_hash, csrf_token_hash, google_sub, admin_email_hash, \
                 created_at, expires_at, revoked_at FROM admin_sessions), \
             EXISTS(SELECT state_hash, nonce_hash, browser_binding_hash, created_at, \
                 expires_at, consumed_at FROM admin_login_states), \
             EXISTS(SELECT id, action, key_id, actor_kind, actor_session_id, created_at FROM admin_events), \
             EXISTS(SELECT config_hash FROM admin_sessions), \
             EXISTS(SELECT bucket, window_start, admitted FROM admin_rate_windows)",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_key(
        &self,
        requests_per_minute: i64,
        name: Option<&str>,
        actor: KeyActor,
    ) -> Result<CreatedKey, Error> {
        if requests_per_minute < 1 {
            return Err(Error::InvalidLimit);
        }
        // 제어문자는 trim 전 거부해 이름 앞뒤의 개행도 묵인하지 않는다.
        if name.is_some_and(|value| value.chars().any(char::is_control)) {
            return Err(Error::InvalidKeyName);
        }
        let name = name.map(str::trim).filter(|value| !value.is_empty());
        if name.is_some_and(|value| value.chars().count() > 128) {
            return Err(Error::InvalidKeyName);
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
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        validate_key_actor(&mut transaction, actor).await?;
        sqlx::query(
            "INSERT INTO api_keys (id, token_hash, created_at, requests_per_minute, name) \
             VALUES (?, ?, unixepoch(), ?, ?)",
        )
        .bind(&id)
        .bind(token_hash(&api_key).as_slice())
        .bind(requests_per_minute)
        .bind(name)
        .execute(&mut *transaction)
        .await?;
        // 감사 쓰기 실패 시 키도 반환·커밋하지 않는다.
        record_key_event(&mut transaction, "key_created", &id, actor).await?;
        transaction.commit().await?;
        Ok(CreatedKey {
            display_name: name.unwrap_or(&id).to_owned(),
            id,
            api_key,
            name: name.map(str::to_owned),
        })
    }

    pub async fn list_keys(&self) -> Result<Vec<KeyInfo>, Error> {
        Ok(sqlx::query_as(
            "SELECT id, created_at, revoked_at, requests_per_minute, name, \
             COALESCE(name, id) AS display_name \
             FROM api_keys ORDER BY created_at, id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn revoke_key(&self, id: &str, actor: KeyActor) -> Result<(), Error> {
        // 쓰기를 먼저 직렬화하고 최초 폐기에만 시각과 감사 기록을 남긴다.
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        validate_key_actor(&mut transaction, actor).await?;
        let result = sqlx::query(
            "UPDATE api_keys SET revoked_at = unixepoch() WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 1 {
            record_key_event(&mut transaction, "key_revoked", id, actor).await?;
        } else {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_keys WHERE id = ?)")
                    .bind(id)
                    .fetch_one(&mut *transaction)
                    .await?;
            if !exists {
                return Err(Error::KeyNotFound);
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn usage(&self, filter: UsageFilter<'_>) -> Result<UsageReport, Error> {
        if filter.from.is_some_and(|value| value < 0)
            || filter.until.is_some_and(|value| value < 0)
            || matches!((filter.from, filter.until), (Some(from), Some(until)) if from >= until)
        {
            return Err(Error::InvalidUsageFilter);
        }
        // WAL 읽기 트랜잭션의 같은 snapshot으로 존재 확인·전체·일별·종류별을 집계한다.
        // 개별 요청 행을 메모리로 가져오지 않고 SQLite 집계 오류를 그대로 전파한다.
        let mut transaction = self.pool.begin().await?;
        if let Some(id) = filter.key_id {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_keys WHERE id = ?)")
                    .bind(id)
                    .fetch_one(&mut *transaction)
                    .await?;
            if !exists {
                return Err(Error::KeyNotFound);
            }
        }
        let summary = usage_query("SELECT ", filter)
            .build_query_as::<UsageSummary>()
            .fetch_one(&mut *transaction)
            .await?;
        let by_day = usage_query("SELECT date(started_at, 'unixepoch') AS day, ", filter)
            .push(" GROUP BY day ORDER BY day")
            .build_query_as::<DailyUsage>()
            .fetch_all(&mut *transaction)
            .await?;
        let by_kind = usage_query("SELECT COALESCE(request_kind, 'unknown') AS kind, ", filter)
            .push(" GROUP BY request_kind ORDER BY kind")
            .build_query_as::<KindUsage>()
            .fetch_all(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(UsageReport {
            summary,
            by_day,
            by_kind,
        })
    }

    pub(crate) async fn admit(
        &self,
        hash: &[u8; 32],
        request_id: &str,
        started: std::time::Instant,
        kind: RequestKind,
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
            "INSERT INTO request_usage (id, key_id, started_at, status, duration_ms, request_kind) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(request_id)
        .bind(key_id)
        .bind(now)
        .bind(limited.then_some(429_i64))
        .bind(limited.then(|| elapsed_ms(started)))
        .bind(kind.stored())
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
        tokens: Option<TokenUsage>,
    ) -> Result<(), Error> {
        // 단일 UPDATE 실패 시 기존 pending은 유지된다. 호출자는 성공 대신 503을 보낸다.
        // DB 커밋과 HTTP 전달은 원자적이지 않으며, 중단·커밋 결과 불명은 성공 추정 금지다.
        let result = sqlx::query(
            "UPDATE request_usage SET status = ?, duration_ms = ?, input_tokens = ?, \
             output_tokens = ? WHERE id = ? AND status IS NULL",
        )
        .bind(i64::from(status))
        .bind(duration_ms)
        .bind(tokens.map(|usage| usage.input_tokens))
        .bind(tokens.map(|usage| usage.output_tokens))
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(Error::Database);
        }
        Ok(())
    }

    pub(crate) async fn cancel(&self, request_id: &str, duration_ms: i64) -> Result<(), Error> {
        // 정상 완료와 경합해도 확정된 상태는 바꾸지 않고 남은 pending만 종료한다.
        sqlx::query(
            "UPDATE request_usage SET status = 499, duration_ms = ?, input_tokens = NULL, \
             output_tokens = NULL WHERE id = ? AND status IS NULL",
        )
        .bind(duration_ms)
        .bind(request_id)
        .execute(&self.pool)
        .await?;
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
