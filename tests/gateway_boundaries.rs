use std::{collections::HashSet, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    http::{HeaderValue, Request, StatusCode, header::AUTHORIZATION},
    response::Response,
};
use picksy::{
    db, gateway,
    inference::{Inference, Limits, MODEL_ID},
};
use sha2::{Digest, Sha256};
use sqlx::{
    ConnectOptions, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use tokio::sync::Barrier;
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    path: PathBuf,
    store: db::Store,
    admin: SqlitePool,
}

fn new_path() -> PathBuf {
    // 테스트 DB도 자동 삭제하지 않는다. 실행별 고유한 로컬 합성 데이터만 보존한다.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".local/picksy-dev/gateway-tests")
        .join(Uuid::new_v4().to_string())
        .join("gateway.sqlite3")
}

async fn fixture() -> Fixture {
    let path = new_path();
    db::migrate(&path).await.unwrap();
    let store = db::Store::open(&path).await.unwrap();
    let admin = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .journal_mode(SqliteJournalMode::Wal)
                .foreign_keys(true)
                .busy_timeout(Duration::from_secs(5))
                .disable_statement_logging(),
        )
        .await
        .unwrap();
    Fixture { path, store, admin }
}

fn auth_router(store: db::Store) -> axum::Router {
    // 인증 전용 검사는 상류를 호출하지 않는다. 실제 추론 경계는 inference_boundaries에서 검사한다.
    gateway::router(
        store,
        Inference::new(
            "http://127.0.0.1:9",
            "synthetic-internal",
            Limits::default(),
        )
        .unwrap(),
    )
}

fn check_request(token: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .uri("/v1/auth/check")
        .header("x-request-id", "untrusted-client-id")
        .body(Body::empty())
        .unwrap();
    if let Some(token) = token {
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
    }
    request
}

async fn check(app: axum::Router, token: &str) -> Response {
    app.oneshot(check_request(Some(token))).await.unwrap()
}

async fn assert_unauthorized(response: Response) {
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()["www-authenticate"], "Bearer");
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
        br#"{"error":"unauthorized"}"#,
    );
}

#[tokio::test]
async fn ambiguous_credentials_and_revoked_keys_share_one_unauthorized_response() {
    let fixture = fixture().await;
    let key = fixture
        .store
        .create_key(60, None, db::KeyActor::Cli)
        .await
        .unwrap();
    let app = auth_router(fixture.store.clone());
    assert_unauthorized(app.clone().oneshot(check_request(None)).await.unwrap()).await;

    for authorization in [
        "Basic synthetic".to_owned(),
        "Bearer".to_owned(),
        "Bearer invalid".to_owned(),
        format!("Bearer {}, Bearer {}", key.api_key, key.api_key),
        format!("Bearer {} ", key.api_key),
        format!("Bearer pk_{}", "0".repeat(64)),
    ] {
        let mut request = check_request(None);
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&authorization).unwrap(),
        );
        assert_unauthorized(app.clone().oneshot(request).await.unwrap()).await;
    }
    let mut duplicate = check_request(Some(&key.api_key));
    duplicate
        .headers_mut()
        .append(AUTHORIZATION, HeaderValue::from_static("Bearer synthetic"));
    assert_unauthorized(app.clone().oneshot(duplicate).await.unwrap()).await;
    assert_eq!(
        fixture
            .store
            .usage(db::UsageFilter::default())
            .await
            .unwrap()
            .summary
            .total,
        0
    );

    let response = check(app.clone(), &key.api_key).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let request_id = response.headers()["x-request-id"].to_str().unwrap();
    assert!(Uuid::parse_str(request_id).is_ok());
    assert_ne!(request_id, "untrusted-client-id");
    assert_eq!(response.headers()["cache-control"], "no-store");

    // 이미 열어 둔 독립 풀도 폐기 후 DB를 다시 조회해야 한다.
    let other = db::Store::open(&fixture.path).await.unwrap();
    fixture
        .store
        .revoke_key(&key.id, db::KeyActor::Cli)
        .await
        .unwrap();
    assert_unauthorized(check(auth_router(other.clone()), &key.api_key).await).await;
    assert_unauthorized(check(app, &key.api_key).await).await;
    other.close().await;
    fixture.store.close().await;
    let reopened = db::Store::open(&fixture.path).await.unwrap();
    assert_unauthorized(check(auth_router(reopened.clone()), &key.api_key).await).await;
    let summary = reopened
        .usage(db::UsageFilter {
            key_id: Some(&key.id),
            ..Default::default()
        })
        .await
        .unwrap()
        .summary;
    assert_eq!(summary.total, 1);
    assert_eq!(summary.succeeded, 1);
    assert_eq!(summary.pending, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_pools_share_atomic_quota_and_record_denials() {
    let fixture = fixture().await;
    let key = fixture
        .store
        .create_key(3, None, db::KeyActor::Cli)
        .await
        .unwrap();
    let other = db::Store::open(&fixture.path).await.unwrap();
    let apps = [auth_router(fixture.store.clone()), auth_router(other)];
    let barrier = Arc::new(Barrier::new(33));
    let mut tasks = Vec::new();
    for index in 0..32 {
        let app = apps[index % 2].clone();
        let token = key.api_key.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            let response = check(app, &token).await;
            let status = response.status();
            assert!(status == StatusCode::NO_CONTENT || status == StatusCode::TOO_MANY_REQUESTS);
            if status == StatusCode::TOO_MANY_REQUESTS {
                let retry: u64 = response.headers()["retry-after"]
                    .to_str()
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!((1..=60).contains(&retry));
            }
            response.headers()["x-request-id"]
                .to_str()
                .unwrap()
                .to_owned()
        }));
    }
    barrier.wait().await;
    let mut ids = HashSet::new();
    for task in tasks {
        assert!(ids.insert(task.await.unwrap()));
    }
    // 실제 UTC 분 경계에 걸려도 각 분의 합산 허용량이 한도를 넘지 않아야 한다.
    let windows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT started_at / 60, COUNT(*) FROM request_usage \
         WHERE status = 204 GROUP BY started_at / 60",
    )
    .fetch_all(&fixture.admin)
    .await
    .unwrap();
    assert!(!windows.is_empty());
    for (_, admitted) in windows {
        assert!((1..=3).contains(&admitted));
    }
    let report = fixture
        .store
        .usage(db::UsageFilter {
            key_id: Some(&key.id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(report.by_kind.len(), 1);
    assert!(matches!(report.by_kind[0].kind, db::RequestKind::AuthCheck));
    let summary = report.summary;
    assert_eq!((summary.input_tokens, summary.output_tokens), (None, None));
    assert_eq!((summary.token_recorded, summary.token_unknown), (0, 0));
    assert_eq!(summary.total, 32);
    assert_eq!(summary.pending, 0);
    assert_eq!(summary.failed, 0);
    assert!(summary.rejected > 0);
    assert_eq!(summary.total, summary.succeeded + summary.rejected);
    let counter: i64 = sqlx::query_scalar("SELECT admitted FROM rate_windows WHERE key_id = ?")
        .bind(&key.id)
        .fetch_one(&fixture.admin)
        .await
        .unwrap();
    assert!((1..=3).contains(&counter));
}

#[tokio::test]
async fn admission_record_failure_rolls_back_quota_and_returns_unavailable() {
    let fixture = fixture().await;
    let key = fixture
        .store
        .create_key(1, None, db::KeyActor::Cli)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_admission BEFORE INSERT ON request_usage \
         BEGIN SELECT RAISE(ABORT, '합성 접수 기록 실패'); END",
    )
    .execute(&fixture.admin)
    .await
    .unwrap();
    assert_eq!(
        check(auth_router(fixture.store.clone()), &key.api_key)
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE,
    );
    let counters: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rate_windows")
        .fetch_one(&fixture.admin)
        .await
        .unwrap();
    assert_eq!(counters, 0);
    assert_eq!(
        fixture
            .store
            .usage(db::UsageFilter::default())
            .await
            .unwrap()
            .summary
            .total,
        0
    );
}

#[tokio::test]
async fn completion_failure_keeps_pending_and_never_reports_success() {
    let fixture = fixture().await;
    let key = fixture
        .store
        .create_key(60, None, db::KeyActor::Cli)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_completion BEFORE UPDATE ON request_usage \
         BEGIN SELECT RAISE(ABORT, '합성 완료 기록 실패'); END",
    )
    .execute(&fixture.admin)
    .await
    .unwrap();
    assert_eq!(
        check(auth_router(fixture.store.clone()), &key.api_key)
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE,
    );
    let summary = fixture
        .store
        .usage(db::UsageFilter::default())
        .await
        .unwrap()
        .summary;
    assert_eq!(summary.total, 1);
    assert_eq!(summary.pending, 1);
    assert_eq!(summary.succeeded, 0);
    assert_eq!(summary.failed, 0);
    let unfinished: (Option<i64>, Option<i64>) =
        sqlx::query_as("SELECT status, duration_ms FROM request_usage")
            .fetch_one(&fixture.admin)
            .await
            .unwrap();
    assert_eq!(unfinished, (None, None));
}

#[tokio::test]
async fn database_outage_fails_closed_without_breaking_liveness() {
    let fixture = fixture().await;
    let key = fixture
        .store
        .create_key(60, None, db::KeyActor::Cli)
        .await
        .unwrap();
    let app = auth_router(fixture.store.clone());
    fixture.store.close().await;
    assert_eq!(
        check(app.clone(), &key.api_key).await.status(),
        StatusCode::SERVICE_UNAVAILABLE,
    );
    for (path, expected) in [
        ("/health/live", StatusCode::OK),
        ("/health/ready", StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    assert!(
        fixture
            .store
            .usage(db::UsageFilter::default())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn opening_never_creates_or_repairs_an_unmigrated_database() {
    let path = new_path();
    assert!(db::Store::open(&path).await.is_err());
    assert!(!path.exists());
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .disable_statement_logging(),
        )
        .await
        .unwrap();
    assert!(db::Store::open(&path).await.is_err());
    let migration_tables: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE name = '_sqlx_migrations'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(migration_tables, 0);

    let fixture = fixture().await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00'")
        .execute(&fixture.admin)
        .await
        .unwrap();
    assert!(db::Store::open(&fixture.path).await.is_err());
}

#[tokio::test]
async fn key_listing_excludes_secrets_and_database_contains_only_sha256_verifier() {
    let fixture = fixture().await;
    assert!(
        fixture
            .store
            .create_key(0, None, db::KeyActor::Cli)
            .await
            .is_err()
    );
    assert!(
        fixture
            .store
            .create_key(-1, None, db::KeyActor::Cli)
            .await
            .is_err()
    );
    let key = fixture
        .store
        .create_key(7, None, db::KeyActor::Cli)
        .await
        .unwrap();
    assert_eq!(key.name, None);
    assert_eq!(key.display_name, key.id);
    let hash: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM api_keys WHERE id = ?")
        .bind(&key.id)
        .fetch_one(&fixture.admin)
        .await
        .unwrap();
    assert_eq!(hash.len(), 32);
    let expected: [u8; 32] = Sha256::digest(key.api_key.as_bytes()).into();
    assert_eq!(hash.as_slice(), expected.as_slice());
    let listed = fixture.store.list_keys().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].requests_per_minute, 7);
    assert_eq!(listed[0].name, None);
    assert_eq!(listed[0].display_name, key.id);
    let serialized = serde_json::to_value(listed).unwrap();
    let fields = serialized[0].as_object().unwrap();
    let names: HashSet<_> = fields.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        HashSet::from([
            "id",
            "name",
            "display_name",
            "created_at",
            "revoked_at",
            "requests_per_minute"
        ]),
    );
    assert!(
        !serde_json::to_string(&serialized)
            .unwrap()
            .contains(&key.api_key)
    );
    fixture
        .store
        .revoke_key(&key.id, db::KeyActor::Cli)
        .await
        .unwrap();
    let first = fixture.store.list_keys().await.unwrap()[0].revoked_at;
    fixture
        .store
        .revoke_key(&key.id, db::KeyActor::Cli)
        .await
        .unwrap();
    assert_eq!(
        fixture.store.list_keys().await.unwrap()[0].revoked_at,
        first
    );
    assert!(
        fixture
            .store
            .revoke_key("unknown-key-id", db::KeyActor::Cli)
            .await
            .is_err()
    );
    assert!(
        fixture
            .store
            .usage(db::UsageFilter {
                key_id: Some("unknown-key-id"),
                ..Default::default()
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn model_and_inference_routes_share_authentication_and_model_listing_usage() {
    let fixture = fixture().await;
    let app = auth_router(fixture.store.clone());
    for (method, path) in [("GET", "/v1/models"), ("POST", "/v1/systemone")] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_unauthorized(response).await;
    }
    assert_eq!(
        fixture
            .store
            .usage(db::UsageFilter::default())
            .await
            .unwrap()
            .summary
            .total,
        0
    );
    let key = fixture
        .store
        .create_key(60, None, db::KeyActor::Cli)
        .await
        .unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .header(AUTHORIZATION, format!("Bearer {}", key.api_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["models"].as_array().unwrap().len(), 1);
    assert_eq!(body["models"][0]["name"], MODEL_ID);
    assert!(body["models"][0]["description"].is_string());
    assert_eq!(body["models"][0]["release_date"], "");
    assert_eq!(
        fixture
            .store
            .usage(db::UsageFilter::default())
            .await
            .unwrap()
            .summary
            .succeeded,
        1
    );
}

#[tokio::test]
async fn denial_record_failure_returns_unavailable_without_charging_more_quota() {
    let fixture = fixture().await;
    let key = fixture
        .store
        .create_key(1, None, db::KeyActor::Cli)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_denial BEFORE INSERT ON request_usage WHEN NEW.status = 429 \
         BEGIN SELECT RAISE(ABORT, '합성 제한 기록 실패'); END",
    )
    .execute(&fixture.admin)
    .await
    .unwrap();
    let app = auth_router(fixture.store.clone());
    let mut succeeded = 0;
    let mut unavailable = 0;
    // 분 경계에서 한 번 더 허용될 수 있으므로 단일 응답 대신 연속 접수 결과를 본다.
    for _ in 0..4 {
        match check(app.clone(), &key.api_key).await.status() {
            StatusCode::NO_CONTENT => succeeded += 1,
            StatusCode::SERVICE_UNAVAILABLE => unavailable += 1,
            status => panic!("기록 실패 시 허용하지 않은 HTTP 상태: {status}"),
        }
    }
    assert!(unavailable > 0);
    let summary = fixture
        .store
        .usage(db::UsageFilter {
            key_id: Some(&key.id),
            ..Default::default()
        })
        .await
        .unwrap()
        .summary;
    assert_eq!(summary.total, succeeded);
    assert_eq!(summary.succeeded, succeeded);
    assert_eq!(summary.rejected, 0);
    assert_eq!(summary.pending, 0);
    let admitted: i64 = sqlx::query_scalar("SELECT admitted FROM rate_windows WHERE key_id = ?")
        .bind(&key.id)
        .fetch_one(&fixture.admin)
        .await
        .unwrap();
    assert_eq!(admitted, 1);
}

#[tokio::test]
async fn request_kinds_come_from_routes_not_client_claims() {
    let fixture = fixture().await;
    let key = fixture
        .store
        .create_key(60, None, db::KeyActor::Cli)
        .await
        .unwrap();
    let app = auth_router(fixture.store.clone());
    for (method, path, status) in [
        ("GET", "/v1/models?kind=admin", StatusCode::OK),
        (
            "GET",
            "/v1/auth/check?request_kind=inference",
            StatusCode::NO_CONTENT,
        ),
        ("POST", "/v1/systemone?kind=models", StatusCode::BAD_REQUEST),
    ] {
        let response = app.clone().oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(AUTHORIZATION, format!("Bearer {}", key.api_key))
                .header("content-type", "application/json")
                .header("x-request-kind", "admin")
                .header("x-input-tokens", "999")
                .body(Body::from(r#"{"request_kind":"admin","usage":{"input_tokens":999,"output_tokens":999}}"#))
                .unwrap(),
        ).await.unwrap();
        assert_eq!(response.status(), status);
    }
    for (kind, total, unknown) in [
        (db::RequestKind::Inference, 1, 1),
        (db::RequestKind::Models, 1, 0),
        (db::RequestKind::AuthCheck, 1, 0),
        (db::RequestKind::Admin, 0, 0),
        (db::RequestKind::Unknown, 0, 0),
    ] {
        let report = fixture
            .store
            .usage(db::UsageFilter {
                kind: Some(kind),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(report.summary.total, total);
        assert_eq!(
            (report.summary.input_tokens, report.summary.output_tokens),
            (None, None)
        );
        assert_eq!(
            (report.summary.token_recorded, report.summary.token_unknown),
            (0, unknown)
        );
        assert_eq!(report.by_kind.len(), total as usize);
        assert_eq!(
            report
                .by_day
                .iter()
                .map(|day| day.summary.total)
                .sum::<i64>(),
            total
        );
    }
}

#[tokio::test]
async fn inference_quota_denials_keep_route_kind_without_synthesizing_tokens() {
    let fixture = fixture().await;
    let key = fixture
        .store
        .create_key(1, None, db::KeyActor::Cli)
        .await
        .unwrap();
    let app = auth_router(fixture.store.clone());
    let mut denied = 0;
    // UTC 분 경계에서 추가 허용될 수 있어 연속 접수하고 실제 거절 건수를 비교한다.
    for _ in 0..4 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/systemone?kind=admin")
                    .header(AUTHORIZATION, format!("Bearer {}", key.api_key))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        match response.status() {
            StatusCode::BAD_REQUEST => {}
            StatusCode::TOO_MANY_REQUESTS => denied += 1,
            status => panic!("추론 한도 검사에서 예상하지 않은 상태: {status}"),
        }
    }
    assert!(denied > 0);
    let stored: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM request_usage WHERE status = 429 \
         AND request_kind = 'inference' AND input_tokens IS NULL AND output_tokens IS NULL",
    )
    .fetch_one(&fixture.admin)
    .await
    .unwrap();
    assert_eq!(stored, denied);
    let report = fixture
        .store
        .usage(db::UsageFilter {
            kind: Some(db::RequestKind::Inference),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!((report.summary.total, report.summary.rejected), (4, 4));
    assert_eq!(
        (report.summary.token_recorded, report.summary.token_unknown),
        (0, 4)
    );
    assert_eq!(
        (report.summary.input_tokens, report.summary.output_tokens),
        (None, None)
    );
}
