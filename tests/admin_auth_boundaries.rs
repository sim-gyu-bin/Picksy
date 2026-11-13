use std::{path::PathBuf, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    response::Response,
};
use picksy::{
    admin::AdminConfig,
    db::{self, KeyActor, Store},
    gateway,
    inference::{Inference, Limits},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    ConnectOptions, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tower::ServiceExt;
use uuid::Uuid;

const EMAIL: &str = "admin@example.invalid";
const CLIENT: &str = "synthetic-client";
const SECRET: &str = "synthetic-client-secret";
const ORIGIN: &str = "http://127.0.0.1:18081";
const REDIRECT: &str = "http://127.0.0.1:18081/auth/google/callback";

struct Fixture {
    app: Router,
    store: Store,
    pool: SqlitePool,
    path: PathBuf,
    token: String,
    csrf: String,
    session_id: String,
}

fn config() -> AdminConfig {
    AdminConfig::new(EMAIL, CLIENT, SECRET, REDIRECT).unwrap()
}
fn hash(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

fn fingerprint() -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"picksy-admin-config-v1");
    for value in [EMAIL, CLIENT, SECRET, REDIRECT] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    hash.finalize().to_vec()
}

fn app(store: Store, config: Option<AdminConfig>) -> Router {
    gateway::router_with_admin(
        store,
        Inference::new(
            "http://127.0.0.1:9",
            "synthetic-upstream-key",
            Limits::default(),
        )
        .unwrap(),
        config,
    )
}

async fn fixture() -> Fixture {
    // 테스트 DB의 합성 세션 seed일 뿐 제품의 Google 인증을 우회하는 endpoint·환경변수는 없다.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".local/picksy-dev/admin-auth-tests")
        .join(Uuid::new_v4().to_string())
        .join("gateway.sqlite3");
    db::migrate(&path).await.unwrap();
    let store = Store::open(&path).await.unwrap();
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .foreign_keys(true)
                .busy_timeout(Duration::from_secs(5))
                .disable_statement_logging(),
        )
        .await
        .unwrap();
    let token = "a".repeat(64);
    let mut digest = Sha256::new();
    digest.update(b"picksy-admin-csrf-v1");
    digest.update(token.as_bytes());
    let csrf = hex::encode(digest.finalize());
    let session_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO admin_sessions (id, token_hash, csrf_token_hash, google_sub, admin_email_hash, config_hash, created_at, expires_at) VALUES (?, ?, ?, 'synthetic-sub', ?, ?, unixepoch()-60, unixepoch()+3600)")
        .bind(&session_id).bind(hash(&token)).bind(hash(&csrf)).bind(hash(EMAIL)).bind(fingerprint())
        .execute(&pool).await.unwrap();
    Fixture {
        app: app(store.clone(), Some(config())),
        store,
        pool,
        path,
        token,
        csrf,
        session_id,
    }
}

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Response {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    response
}

async fn json_body(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1_048_576).await.unwrap()).unwrap()
}

impl Fixture {
    fn cookie(&self) -> String {
        format!("picksy_admin={}", self.token)
    }
    async fn get(&self, uri: &str) -> Response {
        request(&self.app, "GET", uri, &[("cookie", &self.cookie())], "").await
    }
    async fn change(&self, method: &str, uri: &str, body: &str) -> Response {
        request(
            &self.app,
            method,
            uri,
            &[
                ("cookie", &self.cookie()),
                ("origin", ORIGIN),
                ("x-csrf-token", &self.csrf),
                ("content-type", "application/json"),
            ],
            body,
        )
        .await
    }
}

async fn start_login(app: &Router) -> (String, String, String) {
    let response = request(app, "GET", "/auth/google/login", &[], "").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        cookie.contains("HttpOnly")
            && cookie.contains("SameSite=Lax")
            && cookie.contains("Max-Age=600")
    );
    let url = reqwest::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    assert_eq!(
        url.origin().ascii_serialization(),
        "https://accounts.google.com"
    );
    let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(pairs["response_type"], "code");
    assert!(pairs["scope"].split(' ').any(|scope| scope == "openid"));
    assert!(pairs["scope"].split(' ').any(|scope| scope == "email"));
    assert_eq!(pairs["code_challenge_method"], "S256");
    assert_eq!(pairs["redirect_uri"], REDIRECT);
    (
        pairs["state"].clone(),
        cookie.split(';').next().unwrap().to_owned(),
        pairs["nonce"].clone(),
    )
}

#[tokio::test]
async fn inference_bearer_never_authenticates_admin_and_disabled_routes_fail_closed() {
    let f = fixture().await;
    let key = f.store.create_key(60, None, KeyActor::Cli).await.unwrap();
    for uri in ["/admin/session", "/admin/keys", "/admin/usage"] {
        let response = request(
            &f.app,
            "GET",
            uri,
            &[("authorization", &format!("Bearer {}", key.api_key))],
            "",
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let disabled = app(f.store.clone(), None);
    for uri in [
        "/admin/session",
        "/auth/google/login",
        "/auth/google/callback?state=x",
    ] {
        assert_eq!(
            request(&disabled, "GET", uri, &[], "").await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    assert_eq!(
        request(
            &disabled,
            "GET",
            "/v1/auth/check",
            &[("authorization", &format!("Bearer {}", key.api_key))],
            ""
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM request_usage")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn session_csrf_round_trip_and_mutations_require_exact_origin_and_single_header() {
    let f = fixture().await;
    let info = json_body(f.get("/admin/session").await).await;
    assert_eq!(info["authenticated"], true);
    assert_eq!(info["csrf_token"], f.csrf);
    let cookie = f.cookie();
    let cases = vec![
        vec![("cookie", cookie.as_str())],
        vec![("cookie", cookie.as_str()), ("origin", ORIGIN)],
        vec![
            ("cookie", cookie.as_str()),
            ("x-csrf-token", f.csrf.as_str()),
        ],
        vec![
            ("cookie", cookie.as_str()),
            ("origin", "http://127.0.0.1:18082"),
            ("x-csrf-token", f.csrf.as_str()),
        ],
        vec![
            ("cookie", cookie.as_str()),
            ("origin", "null"),
            ("x-csrf-token", f.csrf.as_str()),
        ],
        vec![
            ("cookie", cookie.as_str()),
            ("origin", ORIGIN),
            ("x-csrf-token", "incorrect"),
        ],
        vec![
            ("cookie", cookie.as_str()),
            ("origin", ORIGIN),
            ("origin", ORIGIN),
            ("x-csrf-token", f.csrf.as_str()),
        ],
        vec![
            ("cookie", cookie.as_str()),
            ("origin", ORIGIN),
            ("x-csrf-token", f.csrf.as_str()),
            ("x-csrf-token", f.csrf.as_str()),
        ],
    ];
    for headers in cases {
        assert_eq!(
            request(&f.app, "POST", "/admin/keys", &headers, "{}")
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert!(f.store.list_keys().await.unwrap().is_empty());
    assert_eq!(
        request(
            &f.app,
            "GET",
            "/admin/session",
            &[("cookie", &cookie), ("sec-fetch-site", "cross-site")],
            ""
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let response = f
        .change("POST", "/admin/keys", "{\"name\":\" 합성 키 \"}")
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = json_body(response).await;
    assert_eq!(created["name"], "합성 키");
    assert_eq!(created["api_key"].as_str().unwrap().len(), 67);
    let listed = json_body(f.get("/admin/keys").await).await;
    assert!(listed[0].get("api_key").is_none());
    assert_eq!(listed[0]["requests_per_minute"], 60);
    assert_eq!(listed[0]["display_name"], "합성 키");
    let event: (String, String) =
        sqlx::query_as("SELECT actor_kind, actor_session_id FROM admin_events")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(event.0, "admin");
    assert_eq!(event.1, f.session_id);
    assert_eq!(
        json_body(f.get("/admin/usage").await).await["summary"]["total"],
        0
    );
}

#[tokio::test]
async fn tampered_expired_revoked_and_config_changed_sessions_are_rejected() {
    let f = fixture().await;
    for token in [
        "b".repeat(64),
        "a".repeat(63),
        format!("{}; picksy_admin={}", f.token, f.token),
    ] {
        assert_eq!(
            request(
                &f.app,
                "GET",
                "/admin/session",
                &[("cookie", &format!("picksy_admin={token}"))],
                ""
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    for changed in [
        AdminConfig::new("other@example.invalid", CLIENT, SECRET, REDIRECT).unwrap(),
        AdminConfig::new(EMAIL, "other-client", SECRET, REDIRECT).unwrap(),
        AdminConfig::new(EMAIL, CLIENT, "other-secret", REDIRECT).unwrap(),
        AdminConfig::new(
            EMAIL,
            CLIENT,
            SECRET,
            "http://127.0.0.1:18082/auth/google/callback",
        )
        .unwrap(),
    ] {
        assert_eq!(
            request(
                &app(f.store.clone(), Some(changed)),
                "GET",
                "/admin/session",
                &[("cookie", &f.cookie())],
                ""
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    for statement in [
        "UPDATE admin_sessions SET expires_at = unixepoch()-1",
        "UPDATE admin_sessions SET expires_at = unixepoch()+3600, revoked_at = unixepoch()",
        "UPDATE admin_sessions SET revoked_at = NULL, config_hash = NULL",
    ] {
        sqlx::query(statement).execute(&f.pool).await.unwrap();
        assert_eq!(
            f.get("/admin/session").await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn session_hash_corruption_is_not_accepted_and_logout_revokes_server_side() {
    let f = fixture().await;
    sqlx::query("UPDATE admin_sessions SET csrf_token_hash = ?")
        .bind(hash("tampered"))
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        f.get("/admin/session").await.status(),
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE admin_sessions SET csrf_token_hash = ?")
        .bind(hash(&f.csrf))
        .execute(&f.pool)
        .await
        .unwrap();
    let response = f.change("POST", "/admin/logout", "").await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(
        f.get("/admin/session").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.change("POST", "/admin/keys", "{}").await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn login_state_is_browser_bound_one_time_and_consumed_on_provider_denial() {
    let f = fixture().await;
    let (state, cookie, nonce) = start_login(&f.app).await;
    let uri = format!("/auth/google/callback?state={state}&error=access_denied&unknown=ignored");
    assert_eq!(
        request(&f.app, "GET", &uri, &[], "").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &f.app,
            "GET",
            &uri,
            &[("cookie", &format!("picksy_login={}", "b".repeat(64)))],
            ""
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let stored: (Vec<u8>, Vec<u8>, Option<i64>) =
        sqlx::query_as("SELECT state_hash, nonce_hash, consumed_at FROM admin_login_states")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(stored.0, hash(&state));
    assert_eq!(stored.1, hash(&nonce));
    assert_eq!(stored.2, None);
    assert_eq!(
        request(&f.app, "GET", &uri, &[("cookie", &cookie)], "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let consumed: Option<i64> = sqlx::query_scalar("SELECT consumed_at FROM admin_login_states")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(consumed.is_some());
    assert_eq!(
        request(&f.app, "GET", &uri, &[("cookie", &cookie)], "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let unchanged: Option<i64> = sqlx::query_scalar("SELECT consumed_at FROM admin_login_states")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(consumed, unchanged);
    assert_eq!(f.get("/admin/session").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn login_state_tampering_expiry_nonce_and_config_changes_fail_before_google() {
    let f = fixture().await;
    let (state, cookie, _) = start_login(&f.app).await;
    let uri = format!("/auth/google/callback?state={state}&error=access_denied");
    let changed = app(
        f.store.clone(),
        Some(AdminConfig::new(EMAIL, CLIENT, "other-secret", REDIRECT).unwrap()),
    );
    assert_eq!(
        request(&changed, "GET", &uri, &[("cookie", &cookie)], "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mutated = format!(
        "/auth/google/callback?state={}&error=access_denied",
        "0".repeat(64)
    );
    assert_eq!(
        request(&f.app, "GET", &mutated, &[("cookie", &cookie)], "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE admin_login_states SET nonce_hash = ?")
        .bind(hash("wrong-nonce"))
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        request(&f.app, "GET", &uri, &[("cookie", &cookie)], "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let (expired, expired_cookie, _) = start_login(&f.app).await;
    sqlx::query("UPDATE admin_login_states SET created_at = unixepoch()-100, expires_at = unixepoch()-1 WHERE state_hash = ?")
        .bind(hash(&expired)).execute(&f.pool).await.unwrap();
    let uri = format!("/auth/google/callback?state={expired}&error=access_denied");
    assert_eq!(
        request(&f.app, "GET", &uri, &[("cookie", &expired_cookie)], "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let consumed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM admin_login_states WHERE consumed_at IS NOT NULL")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(consumed, 0);
}

#[tokio::test]
async fn callback_duplicate_parameters_and_consume_database_failure_are_safe() {
    let f = fixture().await;
    let (state, cookie, _) = start_login(&f.app).await;
    let duplicate =
        format!("/auth/google/callback?state={state}&state={state}&error=access_denied");
    assert_eq!(
        request(&f.app, "GET", &duplicate, &[("cookie", &cookie)], "")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    sqlx::query("CREATE TRIGGER fail_consume BEFORE UPDATE ON admin_login_states BEGIN SELECT RAISE(ABORT, 'synthetic'); END")
        .execute(&f.pool).await.unwrap();
    let uri = format!("/auth/google/callback?state={state}&error=access_denied");
    assert_eq!(
        request(&f.app, "GET", &uri, &[("cookie", &cookie)], "")
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let consumed: Option<i64> = sqlx::query_scalar("SELECT consumed_at FROM admin_login_states")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(consumed, None);
}

#[tokio::test]
async fn concurrent_callbacks_cannot_consume_state_twice() {
    let f = fixture().await;
    let (state, cookie, _) = start_login(&f.app).await;
    sqlx::query("CREATE TABLE consumption_count (n INTEGER NOT NULL)")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER count_consumption AFTER UPDATE ON admin_login_states BEGIN INSERT INTO consumption_count VALUES (1); END")
        .execute(&f.pool).await.unwrap();
    let uri = format!("/auth/google/callback?state={state}&error=access_denied");
    let second_store = Store::open(&f.path).await.unwrap();
    let second = app(second_store, Some(config()));
    let headers = [("cookie", cookie.as_str())];
    let (one, two) = tokio::join!(
        request(&f.app, "GET", &uri, &headers, ""),
        request(&second, "GET", &uri, &headers, "")
    );
    assert_eq!(one.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(two.status(), StatusCode::UNAUTHORIZED);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM consumption_count")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn audit_failure_rolls_back_http_creation_and_revocation_without_usage_pollution() {
    let f = fixture().await;
    let key = f.store.create_key(60, None, KeyActor::Cli).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_audit BEFORE INSERT ON admin_events BEGIN SELECT RAISE(ABORT, 'synthetic'); END")
        .execute(&f.pool).await.unwrap();
    assert_eq!(
        f.change("POST", "/admin/keys", "{}").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        f.change("DELETE", &format!("/admin/keys/{}", key.id), "")
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let keys = f.store.list_keys().await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].revoked_at, None);
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_events")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(events, 1);
    assert_eq!(
        json_body(f.get("/admin/usage").await).await["summary"]["total"],
        0
    );
    assert_eq!(
        request(
            &f.app,
            "GET",
            "/v1/auth/check",
            &[("authorization", &format!("Bearer {}", key.api_key))],
            ""
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn revoke_is_idempotent_and_revoked_session_cannot_use_shared_key_operations() {
    let f = fixture().await;
    let key = json_body(f.change("POST", "/admin/keys", "{}").await).await;
    let id = key["id"].as_str().unwrap();
    assert_eq!(key["display_name"], id);
    let uri = format!("/admin/keys/{id}");
    assert_eq!(
        f.change("DELETE", &uri, "").await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        f.change("DELETE", &uri, "").await.status(),
        StatusCode::NO_CONTENT
    );
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM admin_events WHERE action = 'key_revoked'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        f.change("POST", "/admin/logout", "").await.status(),
        StatusCode::NO_CONTENT
    );
    let actor = KeyActor::Admin {
        session_id: Uuid::parse_str(&f.session_id).unwrap(),
    };
    assert!(matches!(
        f.store.create_key(60, None, actor).await,
        Err(picksy::Error::AdminUnauthorized)
    ));
    assert!(matches!(
        f.store.revoke_key(id, actor).await,
        Err(picksy::Error::AdminUnauthorized)
    ));
}

#[tokio::test]
async fn size_and_query_validation_do_not_create_keys_or_leak_request_contents() {
    let f = fixture().await;
    for body in [
        json!({"requests_per_minute":0}),
        json!({"name":"line\nbreak"}),
        json!({"name":"x".repeat(129)}),
        json!({"unexpected":"sensitive-synthetic"}),
        json!({"requests_per_minute":1.5}),
    ] {
        let response = f.change("POST", "/admin/keys", &body.to_string()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            json_body(response).await,
            json!({"error":"invalid_request"})
        );
    }
    assert_eq!(
        f.change("POST", "/admin/keys", &"x".repeat(4097))
            .await
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    for uri in [
        "/admin/usage?from=-1",
        "/admin/usage?from=10&until=10",
        "/admin/usage?from=bad",
        "/admin/usage?kind=invalid",
        "/admin/usage?key_id=invalid",
        "/admin/usage?unexpected=x",
        "/admin/usage?from=1&from=2",
        "/admin/keys?unexpected=x",
    ] {
        assert_eq!(f.get(uri).await.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        f.change("DELETE", "/admin/keys/not-a-uuid", "")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.change("DELETE", &format!("/admin/keys/{}", Uuid::new_v4()), "")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(f.store.list_keys().await.unwrap().is_empty());
}

#[tokio::test]
async fn rate_limits_are_shared_bounded_and_separate_from_inference_usage() {
    let f = fixture().await;
    let second = app(Store::open(&f.path).await.unwrap(), Some(config()));
    let mut per_window = std::collections::BTreeMap::<i64, i64>::new();
    let mut limited = 0;
    // DB에 기록된 실제 분별로 세므로 테스트가 UTC 분 경계를 넘더라도 30회를 넘길 수 없다.
    for index in 0..61 {
        let router = if index % 2 == 0 { &f.app } else { &second };
        let response = request(router, "GET", "/auth/google/login", &[], "").await;
        let (window, admitted): (i64, i64) = sqlx::query_as(
            "SELECT window_start, admitted FROM admin_rate_windows WHERE bucket = 'login'",
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert!(admitted <= 30);
        if response.status() == StatusCode::SEE_OTHER {
            *per_window.entry(window).or_default() += 1;
        } else {
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(response.headers()[header::RETRY_AFTER], "60");
            limited += 1;
        }
    }
    assert!(per_window.values().all(|count| *count <= 30));
    assert!(limited > 0);
    assert_eq!(f.get("/admin/session").await.status(), StatusCode::OK);
    assert_eq!(
        request(&second, "GET", "/auth/google/callback?state=x", &[], "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.store
            .usage(Default::default())
            .await
            .unwrap()
            .summary
            .total,
        0
    );
}

#[test]
fn configuration_rejects_unsafe_redirects_empty_secrets_and_public_http_bind() {
    for redirect in [
        "http://example.invalid/auth/google/callback",
        "ftp://127.0.0.1/auth/google/callback",
        "http://127.0.0.1:18081/wrong",
        "http://127.0.0.1:18081/auth/google/callback?x=1",
        "http://user@127.0.0.1:18081/auth/google/callback",
        "http://127.0.0.1:18081/auth/google/callback#x",
    ] {
        assert!(AdminConfig::new(EMAIL, CLIENT, SECRET, redirect).is_err());
    }
    assert!(AdminConfig::new(EMAIL, CLIENT, "", REDIRECT).is_err());
    assert!(AdminConfig::new("bad", CLIENT, SECRET, REDIRECT).is_err());
    assert!(
        config()
            .validate_bind("127.0.0.1:18081".parse().unwrap())
            .is_ok()
    );
    assert!(
        config()
            .validate_bind("0.0.0.0:18081".parse().unwrap())
            .is_err()
    );
    assert!(
        config()
            .validate_bind("127.0.0.1:18082".parse().unwrap())
            .is_err()
    );
    assert!(
        AdminConfig::new(
            EMAIL,
            CLIENT,
            SECRET,
            "https://picksy.example.invalid/auth/google/callback"
        )
        .unwrap()
        .validate_bind("0.0.0.0:18081".parse().unwrap())
        .is_ok()
    );
}

#[tokio::test]
async fn partial_environment_fails_startup_without_reading_dotenv_or_disclosing_values() {
    let f = fixture().await;
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_picksy"))
        .env_clear()
        .env("PICKSY_ADMIN_GOOGLE_EMAIL", EMAIL)
        .env("PICKSY_INFERENCE_API_KEY", "synthetic-upstream-key")
        .arg("--db")
        .arg(&f.path)
        .args(["serve", "--inference-url", "http://127.0.0.1:9"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("관리자 Google 인증"));
    assert!(!stderr.contains(EMAIL));
    assert!(!stderr.contains("synthetic-upstream-key"));
    assert!(result.stdout.is_empty());
}

#[tokio::test]
async fn anonymous_requests_and_invalid_csrf_do_not_consume_authenticated_quotas() {
    let f = fixture().await;
    let login = start_login(&f.app).await;
    for _ in 0..305 {
        assert_eq!(
            request(&f.app, "GET", "/admin/session", &[], "")
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &f.app,
                "GET",
                "/auth/google/callback?state=invalid",
                &[],
                ""
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    for _ in 0..305 {
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/admin/logout",
                &[
                    ("cookie", &f.cookie()),
                    ("origin", ORIGIN),
                    ("x-csrf-token", "invalid")
                ],
                ""
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
    let charged: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_rate_windows WHERE bucket IN ('admin', 'callback')",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(charged, 0);
    assert_eq!(f.get("/admin/session").await.status(), StatusCode::OK);
    let (state, cookie, _) = login;
    let uri = format!("/auth/google/callback?state={state}&error=access_denied");
    assert_eq!(
        request(&f.app, "GET", &uri, &[("cookie", &cookie)], "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let admitted: i64 =
        sqlx::query_scalar("SELECT admitted FROM admin_rate_windows WHERE bucket = 'callback'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(admitted, 1);
    assert_eq!(
        f.change("POST", "/admin/logout", "").await.status(),
        StatusCode::NO_CONTENT
    );
}
