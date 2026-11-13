use super::*;
use openidconnect::{JsonWebKeyId, PrivateSigningKey, core::CoreRsaPrivateSigningKey};
use serde_json::{Value, json};

fn config() -> AdminConfig {
    AdminConfig::new(
        "admin@example.invalid",
        "synthetic-client",
        "synthetic-secret",
        "http://127.0.0.1:18081/auth/google/callback",
    )
    .unwrap()
}

fn signing_key() -> CoreRsaPrivateSigningKey {
    // 저장소의 키는 이 테스트 전용 합성 RSA fixture이며 실제 계정·Google 서명키와 관계없다.
    CoreRsaPrivateSigningKey::from_pem(
        include_str!("admin_test_key.pem"),
        Some(JsonWebKeyId::new("synthetic-key".to_owned())),
    )
    .unwrap()
}

fn claims() -> Value {
    // 고정 claim을 사용해 네트워크·수면·실제 Google 계정에 의존하지 않는다.
    json!({"iss":ISSUER,"aud":"synthetic-client","sub":"synthetic-sub",
        "iat":1_700_000_000,"exp":4_102_444_800_i64,"nonce":"synthetic-nonce",
        "email":"admin@example.invalid","email_verified":true})
}

fn signed(value: Value, key: &CoreRsaPrivateSigningKey, access: &AccessToken) -> CoreIdToken {
    CoreIdToken::new(
        serde_json::from_value(value).unwrap(),
        key,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        Some(access),
        None,
    )
    .unwrap()
}

#[test]
fn signed_oidc_verifies_signature_issuer_audience_expiry_nonce_and_access_token_hash() {
    let config = config();
    let key = signing_key();
    let client = config.client(CoreJsonWebKeySet::new(vec![key.as_verification_key()]));
    let access = AccessToken::new("synthetic-access".to_owned());
    let nonce = Nonce::new("synthetic-nonce".to_owned());
    let token = signed(claims(), &key, &access);
    assert_eq!(
        verify_identity(&config, &client, &token, &access, &nonce).unwrap(),
        "synthetic-sub"
    );
    for (field, value) in [
        ("iss", json!("https://attacker.example.invalid")),
        ("aud", json!("other-client")),
        ("exp", json!(1)),
        ("nonce", json!("wrong-nonce")),
        ("azp", json!("other-client")),
    ] {
        let mut invalid = claims();
        invalid[field] = value;
        let token = signed(invalid, &key, &access);
        assert_eq!(
            verify_identity(&config, &client, &token, &access, &nonce),
            Err(StatusCode::UNAUTHORIZED)
        );
    }
    assert_eq!(
        verify_identity(
            &config,
            &client,
            &token,
            &AccessToken::new("substituted-access".to_owned()),
            &nonce
        ),
        Err(StatusCode::UNAUTHORIZED)
    );
    let mut encoded = token.to_string().into_bytes();
    let signature_start = encoded.iter().rposition(|byte| *byte == b'.').unwrap() + 1;
    encoded[signature_start] = if encoded[signature_start] == b'A' {
        b'B'
    } else {
        b'A'
    };
    let altered: CoreIdToken = String::from_utf8(encoded).unwrap().parse().unwrap();
    assert_eq!(
        verify_identity(&config, &client, &altered, &access, &nonce),
        Err(StatusCode::UNAUTHORIZED)
    );
    let empty_keys = config.client(CoreJsonWebKeySet::default());
    assert_eq!(
        verify_identity(&config, &empty_keys, &token, &access, &nonce),
        Err(StatusCode::UNAUTHORIZED)
    );
}

#[test]
fn valid_google_signatures_do_not_authorize_other_or_unverified_email() {
    let config = config();
    let key = signing_key();
    let client = config.client(CoreJsonWebKeySet::new(vec![key.as_verification_key()]));
    let access = AccessToken::new("synthetic-access".to_owned());
    let nonce = Nonce::new("synthetic-nonce".to_owned());
    for (field, value) in [
        ("email", json!("other@example.invalid")),
        ("email_verified", json!(false)),
        ("sub", json!("")),
    ] {
        let mut invalid = claims();
        invalid[field] = value;
        let token = signed(invalid, &key, &access);
        assert_eq!(
            verify_identity(&config, &client, &token, &access, &nonce),
            Err(StatusCode::FORBIDDEN)
        );
    }
    for field in ["email", "email_verified"] {
        let mut invalid = claims();
        invalid.as_object_mut().unwrap().remove(field);
        let token = signed(invalid, &key, &access);
        assert_eq!(
            verify_identity(&config, &client, &token, &access, &nonce),
            Err(StatusCode::FORBIDDEN)
        );
    }
    let mut uppercase = claims();
    uppercase["email"] = json!("ADMIN@EXAMPLE.INVALID");
    assert!(
        verify_identity(
            &config,
            &client,
            &signed(uppercase, &key, &access),
            &access,
            &nonce
        )
        .is_ok()
    );
}

#[test]
fn unsigned_and_hmac_tokens_cannot_replace_google_rs256() {
    let config = config();
    let access = AccessToken::new("synthetic-access".to_owned());
    let nonce = Nonce::new("synthetic-nonce".to_owned());
    let key = signing_key();
    let client = config.client(CoreJsonWebKeySet::new(vec![key.as_verification_key()]));
    let token = signed(claims(), &key, &access).to_string();
    let parts: Vec<_> = token.split('.').collect();
    // alg=none을 파서가 거부하거나 검증기가 거부해야 한다. 어느 경우도 승인하지 않는다.
    let unsigned = format!("eyJhbGciOiJub25lIn0.{}.", parts[1]);
    if let Ok(token) = unsigned.parse::<CoreIdToken>() {
        assert!(verify_identity(&config, &client, &token, &access, &nonce).is_err());
    }
    let hmac_key = openidconnect::core::CoreHmacKey::new(config.client_secret.as_bytes());
    let token = CoreIdToken::new(
        serde_json::from_value(claims()).unwrap(),
        &hmac_key,
        CoreJwsSigningAlgorithm::HmacSha256,
        Some(&access),
        None,
    )
    .unwrap();
    assert!(verify_identity(&config, &client, &token, &access, &nonce).is_err());
}

async fn store() -> Store {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".local/picksy-dev/admin-auth-unit-tests")
        .join(Uuid::new_v4().to_string())
        .join("gateway.sqlite3");
    crate::db::migrate(&path).await.unwrap();
    Store::open(&path).await.unwrap()
}

#[tokio::test]
async fn session_issuance_hashes_secrets_rotates_browser_session_and_pins_subject() {
    let store = store().await;
    let config = config();
    let (first, info) = issue_session(&store, &config, "synthetic-sub", None)
        .await
        .unwrap();
    assert!(secret_format(&first));
    assert_eq!(info.csrf_token, csrf_token(&first));
    let stored: (Vec<u8>, Vec<u8>, Vec<u8>, String, Vec<u8>) = sqlx::query_as(
        "SELECT token_hash, csrf_token_hash, admin_email_hash, google_sub, config_hash FROM admin_sessions"
    ).fetch_one(&store.pool).await.unwrap();
    assert_eq!(stored.0, token_hash(&first));
    assert_eq!(stored.1, token_hash(&info.csrf_token));
    assert_eq!(stored.2, token_hash(&config.email));
    assert_eq!(stored.3, "synthetic-sub");
    assert_eq!(stored.4, config.fingerprint);
    let (second, _) = issue_session(&store, &config, "synthetic-sub", Some(&first))
        .await
        .unwrap();
    assert_ne!(first, second);
    let active: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM admin_sessions WHERE revoked_at IS NULL")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(active, 1);
    assert!(matches!(
        issue_session(&store, &config, "reassigned-sub", Some(&second)).await,
        Err(Error::AdminUnauthorized)
    ));
    let active: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM admin_sessions WHERE revoked_at IS NULL")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(active, 1);
}

#[tokio::test]
async fn session_insert_failure_rolls_back_previous_session_revocation() {
    let store = store().await;
    let config = config();
    let (first, _) = issue_session(&store, &config, "synthetic-sub", None)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_session BEFORE INSERT ON admin_sessions BEGIN SELECT RAISE(ABORT, 'synthetic'); END")
        .execute(&store.pool).await.unwrap();
    assert!(
        issue_session(&store, &config, "synthetic-sub", Some(&first))
            .await
            .is_err()
    );
    let active: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM admin_sessions WHERE revoked_at IS NULL")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(active, 1);
}

#[test]
fn cookies_use_host_prefix_and_secure_only_for_https_and_secret_domains_are_separate() {
    let local = config();
    let secure = AdminConfig::new(
        "admin@example.invalid",
        "synthetic-client",
        "synthetic-secret",
        "https://picksy.example.invalid/auth/google/callback",
    )
    .unwrap();
    let token = "a".repeat(64);
    let header = secure.cookie(secure.session_cookie(), &token, SESSION_SECONDS);
    let value = header.to_str().unwrap();
    assert!(value.starts_with("__Host-picksy_admin="));
    assert!(value.contains("; Secure"));
    assert!(value.contains("Path=/; HttpOnly; SameSite=Lax"));
    assert!(!value.contains("Domain="));
    assert!(
        !local
            .cookie(local.session_cookie(), &token, SESSION_SECONDS)
            .to_str()
            .unwrap()
            .contains("Secure")
    );
    assert_ne!(
        local.login_secret(b"nonce", &token),
        local.login_secret(b"pkce", &token)
    );
    assert_ne!(
        local.login_secret(b"browser", &token),
        local.login_secret(b"pkce", &token)
    );
    assert_ne!(
        local.login_secret(b"nonce", &token),
        secure.login_secret(b"nonce", &token)
    );
}
