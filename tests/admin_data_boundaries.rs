use std::{path::PathBuf, time::Duration};

use picksy::{
    Error,
    db::{self, KeyActor, RequestKind, Store, TokenUsage, UsageFilter, UsageReport},
};
use sha2::{Digest, Sha256};
use sqlx::{
    ConnectOptions, Row, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use uuid::Uuid;

fn isolated_path() -> PathBuf {
    // 합성 데이터만 새 UUID 디렉터리에 만들고 실행 후에도 자동 삭제하지 않는다.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".local/picksy-dev/gateway-tests")
        .join(Uuid::new_v4().to_string())
        .join("gateway.sqlite3")
}

async fn raw_pool(path: &std::path::Path, create: bool) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(create)
                .foreign_keys(true)
                .busy_timeout(Duration::from_secs(5))
                .disable_statement_logging(),
        )
        .await
        .unwrap()
}

async fn fixture() -> (PathBuf, Store, SqlitePool) {
    let path = isolated_path();
    db::migrate(&path).await.unwrap();
    let store = Store::open(&path).await.unwrap();
    let pool = raw_pool(&path, false).await;
    (path, store, pool)
}

async fn insert_usage(
    pool: &SqlitePool,
    key_id: &str,
    at: i64,
    kind: Option<&str>,
    status: Option<i64>,
    tokens: Option<TokenUsage>,
) {
    sqlx::query(
        "INSERT INTO request_usage \
         (id, key_id, started_at, request_kind, status, duration_ms, input_tokens, output_tokens) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(key_id)
    .bind(at)
    .bind(kind)
    .bind(status)
    .bind(status.map(|_| 1_i64))
    .bind(tokens.map(|value| value.input_tokens))
    .bind(tokens.map(|value| value.output_tokens))
    .execute(pool)
    .await
    .unwrap();
}

fn assert_snapshot(report: &UsageReport) {
    assert_eq!(
        report.summary.total,
        report.summary.pending
            + report.summary.succeeded
            + report.summary.rejected
            + report.summary.failed
            + report.summary.other_completed
    );
    assert_eq!(
        report.summary.total,
        report
            .by_day
            .iter()
            .map(|day| day.summary.total)
            .sum::<i64>()
    );
    assert_eq!(
        report.summary.total,
        report
            .by_kind
            .iter()
            .map(|kind| kind.summary.total)
            .sum::<i64>()
    );
    assert_eq!(
        report.summary.token_recorded,
        report
            .by_day
            .iter()
            .map(|day| day.summary.token_recorded)
            .sum::<i64>()
    );
    assert_eq!(
        report.summary.token_unknown,
        report
            .by_kind
            .iter()
            .map(|kind| kind.summary.token_unknown)
            .sum::<i64>()
    );
}

#[tokio::test]
async fn v1_upgrade_preserves_keys_counters_and_unknown_usage() {
    let path = isolated_path();
    let migration_dir = path.parent().unwrap().join("v1-migrations");
    tokio::fs::create_dir_all(&migration_dir).await.unwrap();
    tokio::fs::write(
        migration_dir.join("0001_gateway.sql"),
        include_str!("../migrations/0001_gateway.sql"),
    )
    .await
    .unwrap();
    let pool = raw_pool(&path, true).await;
    Migrator::new(migration_dir.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    let id = Uuid::new_v4().to_string();
    let hash = [7_u8; 32];
    sqlx::query("INSERT INTO api_keys VALUES (?, ?, 10, 20, 17)")
        .bind(&id)
        .bind(hash.as_slice())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO rate_windows VALUES (?, 60, 2)")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO request_usage VALUES (?, ?, 70, 200, 3)")
        .bind(Uuid::new_v4().to_string())
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(Store::open(&path).await, Err(Error::Schema)));
    pool.close().await;

    db::migrate(&path).await.unwrap();
    let store = Store::open(&path).await.unwrap();
    let pool = raw_pool(&path, false).await;
    let keys = store.list_keys().await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].id, id);
    assert_eq!(keys[0].name, None);
    assert_eq!(keys[0].display_name, id);
    assert_eq!(keys[0].created_at, 10);
    assert_eq!(keys[0].revoked_at, Some(20));
    assert_eq!(keys[0].requests_per_minute, 17);
    let saved_hash: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM api_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(saved_hash, hash);
    let counter: (i64, i64) = sqlx::query_as("SELECT window_start, admitted FROM rate_windows")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(counter, (60, 2));
    let metadata = sqlx::query(
        "SELECT started_at, status, duration_ms, request_kind, input_tokens, output_tokens FROM request_usage",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(metadata.get::<i64, _>("started_at"), 70);
    assert_eq!(metadata.get::<i64, _>("status"), 200);
    assert_eq!(metadata.get::<i64, _>("duration_ms"), 3);
    assert_eq!(metadata.get::<Option<String>, _>("request_kind"), None);
    assert_eq!(metadata.get::<Option<i64>, _>("input_tokens"), None);
    assert_eq!(metadata.get::<Option<i64>, _>("output_tokens"), None);
    let report = store
        .usage(UsageFilter {
            key_id: Some(&id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(report.summary.total, 1);
    assert_eq!(report.summary.succeeded, 1);
    assert_eq!(report.summary.input_tokens, None);
    assert_eq!(report.summary.token_unknown, 0);
    assert_eq!(report.by_kind.len(), 1);
    assert_eq!(report.by_kind[0].kind, RequestKind::Unknown);
    assert_snapshot(&report);
    let audits: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(audits, 0);
    store.close().await;
    pool.close().await;
}

#[tokio::test]
async fn names_are_optional_unicode_bounded_and_never_replace_generated_secrets() {
    let (_, store, pool) = fixture().await;
    for name in [None, Some(""), Some(" \u{2003} ")] {
        let key = store.create_key(60, name, KeyActor::Cli).await.unwrap();
        assert_eq!(key.name, None);
        assert_eq!(key.display_name, key.id);
        assert!(Uuid::parse_str(&key.id).is_ok());
        assert_eq!(key.api_key.len(), 67);
        assert!(key.api_key.starts_with("pk_"));
        assert_eq!(hex::decode(&key.api_key[3..]).unwrap().len(), 32);
        let hash: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM api_keys WHERE id = ?")
            .bind(&key.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let expected_hash: [u8; 32] = Sha256::digest(key.api_key.as_bytes()).into();
        assert_eq!(hash, expected_hash);
    }
    let name = "한".repeat(128);
    let key = store
        .create_key(1, Some(&format!("  {name}\u{2003}")), KeyActor::Cli)
        .await
        .unwrap();
    assert_eq!(key.name.as_deref(), Some(name.as_str()));
    assert_eq!(key.display_name, name);
    let duplicate = store
        .create_key(1, Some(&name), KeyActor::Cli)
        .await
        .unwrap();
    assert!(key.id != duplicate.id && key.api_key != duplicate.api_key);
    let html_name = "<script>synthetic</script>";
    let html = store
        .create_key(1, Some(html_name), KeyActor::Cli)
        .await
        .unwrap();
    assert_eq!(html.name.as_deref(), Some(html_name));
    let listed = store.list_keys().await.unwrap();
    assert_eq!(
        listed
            .iter()
            .find(|item| item.id == key.id)
            .unwrap()
            .display_name,
        name
    );
    let too_long = "한".repeat(129);
    for invalid in [too_long.as_str(), "이름\n", "\t", "x\0y", "x\u{7f}y"] {
        assert!(matches!(
            store.create_key(1, Some(invalid), KeyActor::Cli).await,
            Err(Error::InvalidKeyName)
        ));
    }
    assert!(matches!(
        store.create_key(0, None, KeyActor::Cli).await,
        Err(Error::InvalidLimit)
    ));
    assert_eq!(store.list_keys().await.unwrap().len(), listed.len());
    store.close().await;
    pool.close().await;
}

#[tokio::test]
async fn creation_audit_failure_rolls_back_the_key() {
    let (_, store, pool) = fixture().await;
    sqlx::query(
        "CREATE TRIGGER reject_creation_audit BEFORE INSERT ON admin_events \
         WHEN NEW.action = 'key_created' BEGIN SELECT RAISE(ABORT, 'synthetic audit failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        store.create_key(60, None, KeyActor::Cli).await,
        Err(Error::Database)
    ));
    assert!(store.list_keys().await.unwrap().is_empty());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    store.close().await;
    pool.close().await;
}

#[tokio::test]
async fn revocation_audit_failure_preserves_active_key_and_usage() {
    let (_, store, pool) = fixture().await;
    let key = store.create_key(60, None, KeyActor::Cli).await.unwrap();
    insert_usage(&pool, &key.id, 1, Some("inference"), Some(200), None).await;
    sqlx::query(
        "CREATE TRIGGER reject_revocation_audit BEFORE INSERT ON admin_events \
         WHEN NEW.action = 'key_revoked' BEGIN SELECT RAISE(ABORT, 'synthetic audit failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        store.revoke_key(&key.id, KeyActor::Cli).await,
        Err(Error::Database)
    ));
    assert_eq!(store.list_keys().await.unwrap()[0].revoked_at, None);
    assert_eq!(
        store
            .usage(UsageFilter::default())
            .await
            .unwrap()
            .summary
            .total,
        1
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    store.close().await;
    pool.close().await;
}

#[tokio::test]
async fn independent_stores_serialize_creation_and_idempotent_revocation() {
    let (path, first, pool) = fixture().await;
    let second = Store::open(&path).await.unwrap();
    // 관리 작업은 실제 유효 세션을 같은 쓰기 트랜잭션에서 재확인한다.
    let session_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO admin_sessions
         (id, token_hash, csrf_token_hash, google_sub, admin_email_hash, created_at, expires_at)
         VALUES (?, ?, ?, 'synthetic-admin', ?, unixepoch(), unixepoch() + 3600)",
    )
    .bind(session_id.to_string())
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(&pool)
    .await
    .unwrap();
    let (left, right) = tokio::join!(
        first.create_key(60, Some("중복 허용"), KeyActor::Cli),
        second.create_key(60, Some("중복 허용"), KeyActor::Admin { session_id }),
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert!(left.id != right.id && left.api_key != right.api_key);
    insert_usage(&pool, &right.id, 1, Some("models"), Some(200), None).await;
    let (a, b) = tokio::join!(
        first.revoke_key(&right.id, KeyActor::Cli),
        second.revoke_key(&right.id, KeyActor::Admin { session_id }),
    );
    a.unwrap();
    b.unwrap();
    let revoked_at = first
        .list_keys()
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == right.id)
        .unwrap()
        .revoked_at;
    assert!(revoked_at.is_some());
    first.revoke_key(&right.id, KeyActor::Cli).await.unwrap();
    assert_eq!(
        first
            .list_keys()
            .await
            .unwrap()
            .into_iter()
            .find(|item| item.id == right.id)
            .unwrap()
            .revoked_at,
        revoked_at
    );
    let events = sqlx::query("SELECT action, actor_kind, actor_session_id FROM admin_events WHERE key_id = ? ORDER BY action")
        .bind(&right.id).fetch_all(&pool).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].get::<String, _>("action"), "key_created");
    assert_eq!(events[0].get::<String, _>("actor_kind"), "admin");
    assert_eq!(
        events[0].get::<String, _>("actor_session_id"),
        session_id.to_string()
    );
    assert_eq!(events[1].get::<String, _>("action"), "key_revoked");
    let report = second
        .usage(UsageFilter {
            key_id: Some(&right.id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(report.summary.total, 1);
    assert!(matches!(
        first
            .revoke_key(&Uuid::new_v4().to_string(), KeyActor::Cli)
            .await,
        Err(Error::KeyNotFound)
    ));
    first.close().await;
    second.close().await;
    pool.close().await;
}

#[tokio::test]
async fn reports_share_half_open_utc_key_kind_and_null_token_semantics() {
    let (_, store, pool) = fixture().await;
    let key = store.create_key(60, None, KeyActor::Cli).await.unwrap();
    let other = store.create_key(60, None, KeyActor::Cli).await.unwrap();
    insert_usage(
        &pool,
        &key.id,
        0,
        Some("inference"),
        Some(200),
        Some(TokenUsage {
            input_tokens: 8,
            output_tokens: 2,
        }),
    )
    .await;
    insert_usage(
        &pool,
        &key.id,
        86_399,
        Some("inference"),
        Some(200),
        Some(TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
        }),
    )
    .await;
    insert_usage(&pool, &key.id, 86_400, Some("inference"), None, None).await;
    insert_usage(&pool, &key.id, 86_401, Some("inference"), Some(499), None).await;
    insert_usage(&pool, &key.id, 86_402, Some("inference"), Some(429), None).await;
    insert_usage(&pool, &key.id, 86_403, Some("inference"), Some(502), None).await;
    insert_usage(&pool, &key.id, 86_404, Some("models"), Some(200), None).await;
    insert_usage(&pool, &key.id, 86_405, Some("auth_check"), Some(204), None).await;
    insert_usage(&pool, &key.id, 86_406, Some("admin"), Some(302), None).await;
    insert_usage(&pool, &key.id, 86_407, None, Some(200), None).await;
    insert_usage(
        &pool,
        &key.id,
        172_800,
        Some("inference"),
        Some(200),
        Some(TokenUsage {
            input_tokens: 30,
            output_tokens: 10,
        }),
    )
    .await;
    insert_usage(
        &pool,
        &other.id,
        86_400,
        Some("inference"),
        Some(200),
        Some(TokenUsage {
            input_tokens: 900,
            output_tokens: 900,
        }),
    )
    .await;
    let filter = UsageFilter {
        key_id: Some(&key.id),
        from: Some(86_399),
        until: Some(172_800),
        kind: None,
    };
    let report = store.usage(filter).await.unwrap();
    assert_snapshot(&report);
    assert_eq!(report.summary.total, 9);
    assert_eq!(report.summary.pending, 1);
    assert_eq!(report.summary.succeeded, 4);
    assert_eq!(report.summary.rejected, 2);
    assert_eq!(report.summary.failed, 1);
    assert_eq!(report.summary.other_completed, 1);
    assert_eq!(report.summary.input_tokens, Some(0));
    assert_eq!(report.summary.output_tokens, Some(0));
    assert_eq!(report.summary.token_recorded, 1);
    assert_eq!(report.summary.token_unknown, 4);
    assert_eq!(
        report
            .by_day
            .iter()
            .map(|day| day.day.as_str())
            .collect::<Vec<_>>(),
        ["1970-01-01", "1970-01-02"]
    );
    assert_eq!(report.by_day[0].summary.total, 1);
    assert_eq!(report.by_day[1].summary.total, 8);
    assert_eq!(report.by_day[1].summary.input_tokens, None);
    assert_eq!(
        report
            .by_kind
            .iter()
            .map(|kind| kind.kind)
            .collect::<Vec<_>>(),
        [
            RequestKind::Admin,
            RequestKind::AuthCheck,
            RequestKind::Inference,
            RequestKind::Models,
            RequestKind::Unknown
        ]
    );
    for (kind, total) in [
        (RequestKind::Inference, 5),
        (RequestKind::Models, 1),
        (RequestKind::AuthCheck, 1),
        (RequestKind::Admin, 1),
        (RequestKind::Unknown, 1),
    ] {
        let selected = store
            .usage(UsageFilter {
                kind: Some(kind),
                ..filter
            })
            .await
            .unwrap();
        assert_snapshot(&selected);
        assert_eq!(selected.summary.total, total);
        assert_eq!(selected.by_kind.len(), 1);
        assert_eq!(selected.by_kind[0].kind, kind);
        if kind != RequestKind::Inference {
            assert_eq!(selected.summary.input_tokens, None);
            assert_eq!(selected.summary.output_tokens, None);
            assert_eq!(selected.summary.token_recorded, 0);
            assert_eq!(selected.summary.token_unknown, 0);
        }
    }
    let end = store
        .usage(UsageFilter {
            key_id: Some(&key.id),
            from: Some(172_800),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(end.summary.total, 1);
    assert_eq!(end.summary.input_tokens, Some(30));
    let all = store.usage(UsageFilter::default()).await.unwrap();
    assert_eq!(all.summary.total, 12);
    assert_eq!(all.summary.input_tokens, Some(938));
    assert_eq!(all.summary.output_tokens, Some(912));
    store.close().await;
    pool.close().await;
}

#[tokio::test]
async fn empty_reports_and_invalid_filters_do_not_invent_data() {
    let (_, store, pool) = fixture().await;
    let empty = store.usage(UsageFilter::default()).await.unwrap();
    assert_snapshot(&empty);
    assert_eq!(empty.summary.total, 0);
    assert_eq!(empty.summary.input_tokens, None);
    assert_eq!(empty.summary.output_tokens, None);
    assert_eq!(empty.summary.token_unknown, 0);
    assert!(empty.by_day.is_empty() && empty.by_kind.is_empty());
    for (from, until) in [
        (Some(-1), None),
        (None, Some(-1)),
        (Some(10), Some(10)),
        (Some(11), Some(10)),
    ] {
        assert!(matches!(
            store
                .usage(UsageFilter {
                    from,
                    until,
                    ..Default::default()
                })
                .await,
            Err(Error::InvalidUsageFilter)
        ));
    }
    assert!(matches!(
        store
            .usage(UsageFilter {
                key_id: Some("' OR 1=1 --"),
                ..Default::default()
            })
            .await,
        Err(Error::KeyNotFound)
    ));
    assert_eq!(
        store
            .usage(UsageFilter {
                until: Some(0),
                ..Default::default()
            })
            .await
            .unwrap()
            .summary
            .total,
        0
    );
    store.close().await;
    pool.close().await;
}

#[tokio::test]
async fn token_integer_overflow_fails_instead_of_saturating_or_faking_zero() {
    let (_, store, pool) = fixture().await;
    let key = store.create_key(60, None, KeyActor::Cli).await.unwrap();
    insert_usage(
        &pool,
        &key.id,
        0,
        Some("inference"),
        Some(200),
        Some(TokenUsage {
            input_tokens: i64::MAX,
            output_tokens: i64::MAX,
        }),
    )
    .await;
    insert_usage(
        &pool,
        &key.id,
        86_400,
        Some("inference"),
        Some(200),
        Some(TokenUsage {
            input_tokens: 1,
            output_tokens: 0,
        }),
    )
    .await;
    assert!(matches!(
        store.usage(UsageFilter::default()).await,
        Err(Error::Database)
    ));
    let one = store
        .usage(UsageFilter {
            until: Some(86_400),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(one.summary.input_tokens, Some(i64::MAX));
    assert_eq!(one.summary.output_tokens, Some(i64::MAX));
    store.close().await;
    pool.close().await;
}

#[tokio::test]
async fn schema_rejects_unpaired_negative_or_non_inference_success_tokens() {
    let (_, store, pool) = fixture().await;
    let key = store.create_key(60, None, KeyActor::Cli).await.unwrap();
    for (kind, status, input, output) in [
        (Some("inference"), Some(200), Some(0), None),
        (Some("inference"), Some(200), None, Some(0)),
        (Some("inference"), Some(200), Some(-1), Some(0)),
        (Some("inference"), Some(200), Some(0), Some(-1)),
        (Some("models"), Some(200), Some(0), Some(0)),
        (None, Some(200), Some(0), Some(0)),
        (Some("inference"), None, Some(0), Some(0)),
        (Some("inference"), Some(429), Some(0), Some(0)),
        (Some("unknown"), Some(200), None, None),
        (Some("arbitrary"), Some(200), None, None),
    ] {
        assert!(sqlx::query(
            "INSERT INTO request_usage (id, key_id, started_at, request_kind, status, duration_ms, input_tokens, output_tokens) VALUES (?, ?, 0, ?, ?, ?, ?, ?)",
        ).bind(Uuid::new_v4().to_string()).bind(&key.id).bind(kind).bind(status)
            .bind(status.map(|_| 0_i64)).bind(input).bind(output).execute(&pool).await.is_err());
    }
    insert_usage(
        &pool,
        &key.id,
        0,
        Some("inference"),
        Some(200),
        Some(TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
        }),
    )
    .await;
    // 완료 상태나 종류만 사후 변경해도 같은 CHECK가 토큰 의미 훼손을 막는다.
    for statement in [
        "UPDATE request_usage SET status = 499",
        "UPDATE request_usage SET request_kind = NULL",
        "UPDATE request_usage SET input_tokens = NULL",
    ] {
        assert!(sqlx::query(statement).execute(&pool).await.is_err());
    }
    assert_eq!(
        store
            .usage(UsageFilter::default())
            .await
            .unwrap()
            .summary
            .succeeded,
        1
    );
    store.close().await;
    pool.close().await;
}

#[tokio::test]
async fn future_session_state_and_audit_schema_enforce_hash_time_and_actor_boundaries() {
    let (_, store, pool) = fixture().await;
    let key = store.create_key(60, None, KeyActor::Cli).await.unwrap();
    let session_insert = "INSERT INTO admin_sessions (id, token_hash, csrf_token_hash, google_sub, admin_email_hash, created_at, expires_at, revoked_at) VALUES (?, ?, ?, 'synthetic-subject', ?, ?, ?, ?)";
    for (id, token_len, csrf_len, email_len, created, expires, revoked) in [
        ("invalid".to_owned(), 32, 32, 32, 10, 20, None),
        (Uuid::new_v4().to_string(), 31, 32, 32, 10, 20, None),
        (Uuid::new_v4().to_string(), 32, 31, 32, 10, 20, None),
        (Uuid::new_v4().to_string(), 32, 32, 31, 10, 20, None),
        (Uuid::new_v4().to_string(), 32, 32, 32, -1, 20, None),
        (Uuid::new_v4().to_string(), 32, 32, 32, 10, 10, None),
        (Uuid::new_v4().to_string(), 32, 32, 32, 10, 20, Some(9)),
    ] {
        assert!(
            sqlx::query(session_insert)
                .bind(id)
                .bind(vec![1_u8; token_len])
                .bind(vec![2_u8; csrf_len])
                .bind(vec![3_u8; email_len])
                .bind(created)
                .bind(expires)
                .bind(revoked)
                .execute(&pool)
                .await
                .is_err()
        );
    }
    let session_id = Uuid::new_v4().to_string();
    sqlx::query(session_insert)
        .bind(&session_id)
        .bind([1_u8; 32].as_slice())
        .bind([2_u8; 32].as_slice())
        .bind([3_u8; 32].as_slice())
        .bind(10_i64)
        .bind(20_i64)
        .bind(None::<i64>)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sqlx::query(session_insert)
            .bind(Uuid::new_v4().to_string())
            .bind([1_u8; 32].as_slice())
            .bind([2_u8; 32].as_slice())
            .bind([3_u8; 32].as_slice())
            .bind(10_i64)
            .bind(20_i64)
            .bind(None::<i64>)
            .execute(&pool)
            .await
            .is_err()
    );
    let state_insert = "INSERT INTO admin_login_states VALUES (?, ?, ?, ?, ?, ?)";
    for (state_len, nonce_len, binding_len, created, expires, consumed) in [
        (31, 32, 32, 10, 20, None),
        (32, 31, 32, 10, 20, None),
        (32, 32, 31, 10, 20, None),
        (32, 32, 32, -1, 20, None),
        (32, 32, 32, 10, 10, None),
        (32, 32, 32, 10, 20, Some(9)),
        (32, 32, 32, 10, 20, Some(20)),
    ] {
        assert!(
            sqlx::query(state_insert)
                .bind(vec![4_u8; state_len])
                .bind(vec![5_u8; nonce_len])
                .bind(vec![6_u8; binding_len])
                .bind(created)
                .bind(expires)
                .bind(consumed)
                .execute(&pool)
                .await
                .is_err()
        );
    }
    sqlx::query(state_insert)
        .bind([4_u8; 32].as_slice())
        .bind([5_u8; 32].as_slice())
        .bind([6_u8; 32].as_slice())
        .bind(10_i64)
        .bind(20_i64)
        .bind(19_i64)
        .execute(&pool)
        .await
        .unwrap();
    for (action, kind, session, key_id) in [
        (
            "key_created",
            "cli",
            Some(session_id.as_str()),
            key.id.as_str(),
        ),
        ("key_created", "admin", None, key.id.as_str()),
        ("key_created", "admin", Some("invalid"), key.id.as_str()),
        ("key_created", "untrusted", None, key.id.as_str()),
        ("invalid", "cli", None, key.id.as_str()),
        ("key_created", "cli", None, "missing-key"),
    ] {
        assert!(
            sqlx::query("INSERT INTO admin_events VALUES (?, ?, ?, ?, ?, 10)")
                .bind(Uuid::new_v4().to_string())
                .bind(action)
                .bind(key_id)
                .bind(kind)
                .bind(session)
                .execute(&pool)
                .await
                .is_err()
        );
    }
    // 위 합성 SQL 삽입은 스키마 검사일 뿐 로그인·세션 발급 런타임 구현이 아니다.
    store.close().await;
    pool.close().await;
}

#[tokio::test]
async fn schema_validation_rejects_missing_new_columns_even_with_valid_migration_history() {
    let (path, store, pool) = fixture().await;
    store.close().await;
    // 데이터 삭제 없이 열 이름만 손상시켜 이력만 검사하는 구현을 구별한다.
    sqlx::query("ALTER TABLE admin_sessions RENAME COLUMN csrf_token_hash TO damaged_hash")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(Store::open(&path).await, Err(Error::Schema)));
    pool.close().await;
}

#[tokio::test]
async fn concurrent_writes_cannot_split_summary_day_and_kind_snapshots() {
    let (_, store, pool) = fixture().await;
    let key = store.create_key(60, None, KeyActor::Cli).await.unwrap();
    let writer_pool = pool.clone();
    let writer = tokio::spawn(async move {
        for index in 0..64 {
            insert_usage(
                &writer_pool,
                &key.id,
                index * 86_400,
                Some("inference"),
                None,
                None,
            )
            .await;
            tokio::task::yield_now().await;
        }
    });
    for _ in 0..64 {
        let report = store.usage(UsageFilter::default()).await.unwrap();
        assert_snapshot(&report);
        assert_eq!(report.summary.total, report.summary.pending);
        assert_eq!(report.summary.token_unknown, report.summary.total);
        tokio::task::yield_now().await;
    }
    writer.await.unwrap();
    assert_eq!(
        store
            .usage(UsageFilter::default())
            .await
            .unwrap()
            .summary
            .total,
        64
    );
    store.close().await;
    pool.close().await;
}
