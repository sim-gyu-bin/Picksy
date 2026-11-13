//! Google 인증은 추론 Bearer 인증과 분리하며 모든 관리 변경은 세션·Origin·CSRF를 요구한다.
use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Extension, Json, Router,
    body::{Body, to_bytes},
    extract::{FromRequest, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
};
use openidconnect::{
    AccessToken, AccessTokenHash, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
    core::{
        CoreAuthenticationFlow, CoreClient, CoreIdToken, CoreIdTokenClaims, CoreJsonWebKeySet,
        CoreJwsSigningAlgorithm,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    Error,
    db::{KeyActor, RequestKind, Store, UsageFilter, token_hash},
    gateway::api_error,
};

const ISSUER: &str = "https://accounts.google.com";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
const LOGIN_SECONDS: i64 = 600;
const SESSION_SECONDS: i64 = 28_800;
const BODY_LIMIT: usize = 4096;
const GOOGLE_BODY_LIMIT: usize = 524_288;

type GoogleClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
    EndpointNotSet,
>;

/// 비밀값을 Debug로 출력하지 않는다. 설정 변경은 재시작 후 기존 로그인·세션을 거부한다.
#[derive(Clone)]
pub struct AdminConfig {
    email: String,
    client_id: String,
    client_secret: String,
    redirect: RedirectUrl,
    origin: String,
    secure: bool,
    fingerprint: [u8; 32],
    http: reqwest::Client,
}

impl AdminConfig {
    /// 네 값 모두 없으면 비활성화하며 빈 값·부분 설정·비 UTF-8은 시작 실패다.
    pub fn from_env() -> Result<Option<Self>, Error> {
        let values = [
            "PICKSY_ADMIN_GOOGLE_EMAIL",
            "PICKSY_GOOGLE_CLIENT_ID",
            "PICKSY_GOOGLE_CLIENT_SECRET",
            "PICKSY_GOOGLE_REDIRECT_URI",
        ]
        .map(std::env::var_os);
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let [email, client_id, client_secret, redirect] =
            values.map(|value| value.and_then(|value| value.into_string().ok()));
        Self::new(
            &email.ok_or(Error::AdminConfig)?,
            &client_id.ok_or(Error::AdminConfig)?,
            &client_secret.ok_or(Error::AdminConfig)?,
            &redirect.ok_or(Error::AdminConfig)?,
        )
        .map(Some)
    }

    pub fn new(
        email: &str,
        client_id: &str,
        client_secret: &str,
        redirect: &str,
    ) -> Result<Self, Error> {
        if [email, client_id, client_secret, redirect]
            .iter()
            .any(|value| {
                value.is_empty()
                    || value.len() > 4096
                    || value.trim() != *value
                    || value.chars().any(char::is_control)
            })
            || !email.is_ascii()
            || email.chars().any(char::is_whitespace)
            || email.split('@').count() != 2
            || email.starts_with('@')
            || email.ends_with('@')
        {
            return Err(Error::AdminConfig);
        }
        let redirect = RedirectUrl::new(redirect.to_owned()).map_err(|_| Error::AdminConfig)?;
        let url = redirect.url();
        let secure = url.scheme() == "https";
        let loopback = url
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|ip| ip.is_loopback());
        if !(secure || url.scheme() == "http" && loopback)
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/auth/google/callback"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::AdminConfig);
        }
        let email = email.to_ascii_lowercase();
        // 길이 구분과 도메인 분리로 네 설정의 결합에 모호성이 없게 한다.
        let mut hash = Sha256::new();
        hash.update(b"picksy-admin-config-v1");
        for value in [email.as_str(), client_id, client_secret, redirect.as_str()] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| Error::AdminConfig)?;
        Ok(Self {
            email,
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            origin: url.origin().ascii_serialization(),
            secure,
            fingerprint: hash.finalize().into(),
            redirect,
            http,
        })
    }

    /// Secure가 없는 개발 쿠키를 공개 리스너로 발급하는 설정은 허용하지 않는다.
    pub fn validate_bind(&self, bind: SocketAddr) -> Result<(), Error> {
        if !self.secure
            && (!bind.ip().is_loopback()
                || self.redirect.url().port_or_known_default() != Some(bind.port()))
        {
            return Err(Error::AdminConfig);
        }
        Ok(())
    }

    fn client(&self, keys: CoreJsonWebKeySet) -> GoogleClient {
        CoreClient::new(
            ClientId::new(self.client_id.clone()),
            IssuerUrl::new(ISSUER.to_owned()).expect("고정 Google issuer"),
            keys,
        )
        .set_client_secret(ClientSecret::new(self.client_secret.clone()))
        .set_auth_uri(AuthUrl::new(AUTH_URL.to_owned()).expect("고정 Google 인증 URL"))
        .set_token_uri(TokenUrl::new(TOKEN_URL.to_owned()).expect("고정 Google 토큰 URL"))
        .set_redirect_uri(self.redirect.clone())
    }

    fn session_cookie(&self) -> &'static str {
        if self.secure {
            "__Host-picksy_admin"
        } else {
            "picksy_admin"
        }
    }

    fn login_cookie(&self) -> &'static str {
        if self.secure {
            "__Host-picksy_login"
        } else {
            "picksy_login"
        }
    }

    fn cookie(&self, name: &str, value: &str, seconds: i64) -> HeaderValue {
        let secure = if self.secure { "; Secure" } else { "" };
        let mut value = HeaderValue::from_str(&format!(
            "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={seconds}{secure}"
        ))
        .expect("쿠키에는 고정 이름과 16진수 난수만 들어간다");
        value.set_sensitive(true);
        value
    }

    fn login_secret(&self, label: &[u8], seed: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(label);
        hash.update(self.fingerprint);
        hash.update(seed.as_bytes());
        hex::encode(hash.finalize())
    }
}

#[derive(Clone)]
struct AdminState {
    store: Store,
    config: Option<Arc<AdminConfig>>,
}

#[derive(Clone)]
struct Session {
    id: Uuid,
    csrf: String,
    expires_at: i64,
}

/// 비활성화한 관리 경로도 명시적으로 503을 반환한다. 추론 사용량에는 접수하지 않는다.
pub fn router(store: Store, config: Option<AdminConfig>) -> Router {
    let state = AdminState {
        store,
        config: config.map(Arc::new),
    };
    let protected = Router::new()
        .route("/admin/session", get(session))
        .route("/admin/logout", post(logout))
        .route("/admin/keys", get(keys).post(create_key))
        .route("/admin/keys/{id}", delete(revoke_key))
        .route("/admin/usage", get(usage))
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    Router::new()
        .route("/auth/google/login", get(login))
        .route("/auth/google/callback", get(callback))
        .merge(protected)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, boundary))
        .layer(middleware::from_fn(no_store))
}

async fn no_store(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn denied() -> Response {
    api_error(StatusCode::UNAUTHORIZED, "unauthorized")
}
fn forbidden() -> Response {
    api_error(StatusCode::FORBIDDEN, "forbidden")
}
fn unavailable() -> Response {
    api_error(StatusCode::SERVICE_UNAVAILABLE, "unavailable")
}
fn invalid() -> Response {
    api_error(StatusCode::BAD_REQUEST, "invalid_request")
}

async fn boundary(State(state): State<AdminState>, request: Request, next: Next) -> Response {
    let Some(config) = state.config.as_ref() else {
        return unavailable();
    };
    if request
        .uri()
        .path_and_query()
        .is_some_and(|path| path.as_str().len() > 4096)
        || request
            .headers()
            .iter()
            .map(|(name, value)| name.as_str().len() + value.len())
            .sum::<usize>()
            > 16_384
    {
        return api_error(StatusCode::PAYLOAD_TOO_LARGE, "request_too_large");
    }
    // 교차 사이트 fetch로 세션 JSON·CSRF를 읽지 못하며 전달 헤더는 신뢰하지 않는다.
    if single_header(request.headers(), "sec-fetch-site").is_some_and(|site| site == "cross-site")
        && request.uri().path().starts_with("/admin/")
    {
        return forbidden();
    }
    if request.headers().contains_key(header::ORIGIN)
        && single_header(request.headers(), "origin") != Some(config.origin.as_str())
    {
        return forbidden();
    }
    // Content-Length를 믿지 않고 실제 수신량과 시간을 제한한다. JSON 오류 본문은 반사하지 않는다.
    let (parts, body) = request.into_parts();
    let body = match tokio::time::timeout(Duration::from_secs(5), to_bytes(body, BODY_LIMIT)).await
    {
        Ok(Ok(body)) => body,
        Ok(Err(_)) => return api_error(StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"),
        Err(_) => return api_error(StatusCode::REQUEST_TIMEOUT, "request_timeout"),
    };
    next.run(Request::from_parts(parts, Body::from(body))).await
}

async fn rate_response(store: &Store, bucket: &str, limit: i64) -> Option<Response> {
    match take_rate(store, bucket, limit).await {
        Ok(true) => None,
        Ok(false) => {
            let mut response = api_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
            Some(response)
        }
        Err(_) => Some(unavailable()),
    }
}

async fn take_rate(store: &Store, bucket: &str, limit: i64) -> Result<bool, Error> {
    let result = sqlx::query(
        "INSERT INTO admin_rate_windows (bucket, window_start, admitted) \
         VALUES (?, unixepoch() / 60 * 60, 1) ON CONFLICT(bucket) DO UPDATE SET \
         window_start = excluded.window_start, admitted = CASE \
         WHEN admin_rate_windows.window_start = excluded.window_start \
         THEN admin_rate_windows.admitted + 1 ELSE 1 END \
         WHERE admin_rate_windows.window_start <> excluded.window_start OR admin_rate_windows.admitted < ?"
    ).bind(bucket).bind(limit).execute(&store.pool).await?;
    Ok(result.rows_affected() == 1)
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for pair in header.to_str().ok()?.split(';') {
            let (key, value) = pair.trim().split_once('=')?;
            if key == name {
                if found.is_some() || !secret_format(value) {
                    return None;
                }
                found = Some(value);
            }
        }
    }
    found
}

fn secret_format(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn random_secret() -> Result<String, Error> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| Error::Random)?;
    Ok(hex::encode(bytes))
}

fn csrf_token(token: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"picksy-admin-csrf-v1");
    hash.update(token.as_bytes());
    hex::encode(hash.finalize())
}

async fn authenticate(
    State(state): State<AdminState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(config) = state.config.as_ref() else {
        return unavailable();
    };
    let Some(token) = cookie(request.headers(), config.session_cookie()) else {
        return denied();
    };
    let csrf = csrf_token(token);
    let row: Result<Option<(String, i64, Vec<u8>)>, _> = sqlx::query_as(
        "SELECT id, expires_at, csrf_token_hash FROM admin_sessions WHERE token_hash = ? \
         AND admin_email_hash = ? AND config_hash = ? AND length(google_sub) > 0 \
         AND revoked_at IS NULL AND created_at <= unixepoch() AND expires_at > unixepoch()",
    )
    .bind(token_hash(token).as_slice())
    .bind(token_hash(&config.email).as_slice())
    .bind(config.fingerprint.as_slice())
    .fetch_optional(&state.store.pool)
    .await;
    let (id, expires_at, csrf_hash) = match row {
        Ok(Some(row)) => row,
        Ok(None) => return denied(),
        Err(_) => return unavailable(),
    };
    if csrf_hash != token_hash(&csrf) {
        return denied();
    }
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        // SameSite는 보조 방어다. Origin 하나의 정확 일치와 별도 헤더 CSRF를 함께 요구한다.
        if single_header(request.headers(), "origin") != Some(config.origin.as_str())
            || single_header(request.headers(), "x-csrf-token")
                .filter(|value| secret_format(value))
                .map(token_hash)
                .as_ref()
                != Some(&token_hash(&csrf))
        {
            return forbidden();
        }
    }
    let Ok(id) = Uuid::parse_str(&id) else {
        return unavailable();
    };
    // 익명 요청·위조 CSRF가 정상 관리자의 로그아웃·폐기 쿼터를 소진하지 못한다.
    if let Some(response) = rate_response(&state.store, "admin", 300).await {
        return response;
    }
    request.headers_mut().remove(header::AUTHORIZATION);
    request.extensions_mut().insert(Session {
        id,
        csrf,
        expires_at,
    });
    next.run(request).await
}

async fn login(State(state): State<AdminState>, request: Request) -> Response {
    if request.uri().query().is_some() {
        return invalid();
    }
    let Some(config) = state.config.as_ref() else {
        return unavailable();
    };
    if let Some(response) = rate_response(&state.store, "login", 30).await {
        return response;
    }
    let (Ok(seed), Ok(state_token)) = (random_secret(), random_secret()) else {
        return unavailable();
    };
    let nonce = config.login_secret(b"nonce", &seed);
    let binding = config.login_secret(b"browser", &seed);
    let pkce = PkceCodeVerifier::new(config.login_secret(b"pkce", &seed));
    let client = config.client(CoreJsonWebKeySet::default());
    let (url, state_token, nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            move || CsrfToken::new(state_token),
            move || Nonce::new(nonce),
        )
        .add_scope(Scope::new("email".to_owned()))
        .set_pkce_challenge(PkceCodeChallenge::from_code_verifier_sha256(&pkce))
        .url();
    let inserted = sqlx::query(
        "INSERT INTO admin_login_states (state_hash, nonce_hash, browser_binding_hash, created_at, expires_at) \
         VALUES (?, ?, ?, unixepoch(), unixepoch() + ?)"
    ).bind(token_hash(state_token.secret()).as_slice()).bind(token_hash(nonce.secret()).as_slice())
        .bind(token_hash(&binding).as_slice()).bind(LOGIN_SECONDS).execute(&state.store.pool).await;
    if inserted.is_err() {
        return unavailable();
    }
    let mut response = Redirect::to(url.as_str()).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        config.cookie(config.login_cookie(), &seed, LOGIN_SECONDS),
    );
    response
}

#[derive(Deserialize)]
struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
    iss: Option<String>,
}

async fn callback(State(state): State<AdminState>, request: Request) -> Response {
    let Some(config) = state.config.as_ref() else {
        return unavailable();
    };
    let Ok(Query(query)) = Query::<Callback>::try_from_uri(request.uri()) else {
        return invalid();
    };
    let Some(seed) = cookie(request.headers(), config.login_cookie()) else {
        return denied();
    };
    if !secret_format(&query.state) {
        return denied();
    }
    let nonce = config.login_secret(b"nonce", seed);
    let binding = config.login_secret(b"browser", seed);
    let pkce = config.login_secret(b"pkce", seed);
    // 코드 교환보다 먼저 원자적으로 소비한다. Google 오류·취소·통신 실패에도 재사용하지 않는다.
    let consumed = consume_login(&state.store, &query.state, &nonce, &binding).await;
    let mut response = match consumed {
        Err(_) => unavailable(),
        Ok(false) => denied(),
        Ok(true) => {
            // 유효한 브라우저의 일회성 상태를 소비한 요청만 Google 교환 한도를 차감한다.
            if let Some(response) = rate_response(&state.store, "callback", 60).await {
                response
            } else if query.error.is_some()
                || query.iss.as_deref().is_some_and(|issuer| issuer != ISSUER)
            {
                denied()
            } else if let Some(code) = query
                .code
                .filter(|code| !code.is_empty() && code.len() <= 2048)
            {
                match google_identity(config, code, nonce, pkce).await {
                    Ok(subject) => match issue_session(
                        &state.store,
                        config,
                        &subject,
                        cookie(request.headers(), config.session_cookie()),
                    )
                    .await
                    {
                        Ok((token, _)) => {
                            // 성공만 고정 화면으로 전환한다. 코드·state·사용자 redirect는 전달하지 않는다.
                            let mut response = Redirect::to("/console").into_response();
                            response.headers_mut().append(
                                header::SET_COOKIE,
                                config.cookie(config.session_cookie(), &token, SESSION_SECONDS),
                            );
                            response
                        }
                        Err(Error::AdminUnauthorized) => forbidden(),
                        Err(_) => unavailable(),
                    },
                    Err(status) => api_error(
                        status,
                        if status == StatusCode::FORBIDDEN {
                            "forbidden"
                        } else if status == StatusCode::UNAUTHORIZED {
                            "unauthorized"
                        } else {
                            "unavailable"
                        },
                    ),
                }
            } else {
                invalid()
            }
        }
    };
    response.headers_mut().append(
        header::SET_COOKIE,
        config.cookie(config.login_cookie(), "", 0),
    );
    response
}

async fn consume_login(
    store: &Store,
    state: &str,
    nonce: &str,
    binding: &str,
) -> Result<bool, Error> {
    Ok(sqlx::query(
        "UPDATE admin_login_states SET consumed_at = unixepoch() WHERE state_hash = ? \
         AND nonce_hash = ? AND browser_binding_hash = ? AND consumed_at IS NULL \
         AND created_at <= unixepoch() AND expires_at > unixepoch()",
    )
    .bind(token_hash(state).as_slice())
    .bind(token_hash(nonce).as_slice())
    .bind(token_hash(binding).as_slice())
    .execute(&store.pool)
    .await?
    .rows_affected()
        == 1)
}

// Google의 공식 discovery에 명시된 HTTPS endpoint만 사용한다. 환경으로 issuer/JWKS를 바꿀 수 없다.
struct GoogleHttp(reqwest::Client);

impl<'c> openidconnect::AsyncHttpClient<'c> for GoogleHttp {
    type Error = std::io::Error;
    type Future = std::pin::Pin<
        Box<dyn Future<Output = Result<openidconnect::HttpResponse, Self::Error>> + Send + 'c>,
    >;

    fn call(&'c self, request: openidconnect::HttpRequest) -> Self::Future {
        // 어댑터가 Client의 공유 핸들을 소유해 모든 호출 수명에 AsyncHttpClient를 구현한다.
        // 설정 참조 수명에 묶인 구현은 OIDC의 고차 수명 Send 판정을 만족하지 못한다.
        // 응답 크기·타임아웃·민감 오류 제한은 공통 HTTP 구현을 그대로 통과한다.
        Box::pin(google_http(&self.0, request))
    }
}

async fn google_http(
    client: &reqwest::Client,
    request: openidconnect::HttpRequest,
) -> Result<openidconnect::HttpResponse, std::io::Error> {
    let (parts, body) = request.into_parts();
    let mut response = client
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .map_err(|_| std::io::Error::other("인증 서버 통신 실패"))?;
    let mut builder = axum::http::Response::builder().status(response.status());
    *builder.headers_mut().expect("상태는 유효하다") = response.headers().clone();
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| std::io::Error::other("인증 서버 수신 실패"))?
    {
        if body.len().saturating_add(chunk.len()) > GOOGLE_BODY_LIMIT {
            return Err(std::io::Error::other("인증 서버 응답 상한 초과"));
        }
        body.extend_from_slice(&chunk);
    }
    builder
        .body(body)
        .map_err(|_| std::io::Error::other("인증 서버 응답 실패"))
}

async fn google_identity(
    config: &AdminConfig,
    code: String,
    nonce: String,
    pkce: String,
) -> Result<String, StatusCode> {
    let http = GoogleHttp(config.http.clone());
    let jwks_request = axum::http::Request::builder()
        .uri(JWKS_URL)
        .body(Vec::new())
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let jwks = google_http(&config.http, jwks_request)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if jwks.status() != StatusCode::OK {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let keys: CoreJsonWebKeySet =
        serde_json::from_slice(jwks.body()).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let client = config.client(keys);
    let tokens = client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(PkceCodeVerifier::new(pkce))
        .request_async(&http)
        .await
        .map_err(|error| match error {
            openidconnect::RequestTokenError::ServerResponse(_) => StatusCode::UNAUTHORIZED,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })?;
    let token = tokens.id_token().ok_or(StatusCode::UNAUTHORIZED)?;
    verify_identity(
        config,
        &client,
        token,
        tokens.access_token(),
        &Nonce::new(nonce),
    )
}

fn verify_identity(
    config: &AdminConfig,
    client: &GoogleClient,
    token: &CoreIdToken,
    access_token: &AccessToken,
    nonce: &Nonce,
) -> Result<String, StatusCode> {
    let verifier = client
        .id_token_verifier()
        .set_allowed_algs([CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256]);
    let claims = token
        .claims(&verifier, nonce)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    if let Some(expected) = claims.access_token_hash() {
        let actual = AccessTokenHash::from_token(
            access_token,
            token.signing_alg().map_err(|_| StatusCode::UNAUTHORIZED)?,
            token
                .signing_key(&verifier)
                .map_err(|_| StatusCode::UNAUTHORIZED)?,
        )
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
        if &actual != expected {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    authorize_claims(config, claims)?;
    Ok(claims.subject().as_str().to_owned())
}

fn authorize_claims(config: &AdminConfig, claims: &CoreIdTokenClaims) -> Result<(), StatusCode> {
    if claims.email_verified() != Some(true)
        || claims
            .email()
            .is_none_or(|email| !email.as_str().eq_ignore_ascii_case(&config.email))
        || claims.subject().as_str().is_empty()
        || claims.subject().as_str().len() > 255
    {
        return Err(StatusCode::FORBIDDEN);
    }
    if claims
        .authorized_party()
        .is_some_and(|party| party.as_str() != config.client_id)
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

#[derive(Serialize)]
struct SessionInfo {
    authenticated: bool,
    csrf_token: String,
    expires_at: i64,
}

async fn issue_session(
    store: &Store,
    config: &AdminConfig,
    subject: &str,
    old_token: Option<&str>,
) -> Result<(String, SessionInfo), Error> {
    let token = random_secret()?;
    let csrf = csrf_token(&token);
    let mut transaction = store.pool.begin_with("BEGIN IMMEDIATE").await?;
    // 같은 허용 이메일의 sub가 바뀌면 과거 세션이 폐기돼도 계정 재할당을 자동 승인하지 않는다.
    let other_subject: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM admin_sessions WHERE admin_email_hash = ? AND google_sub <> ?)"
    ).bind(token_hash(&config.email).as_slice()).bind(subject).fetch_one(&mut *transaction).await?;
    if other_subject {
        return Err(Error::AdminUnauthorized);
    }
    if let Some(old) = old_token {
        sqlx::query("UPDATE admin_sessions SET revoked_at = unixepoch() WHERE token_hash = ? AND revoked_at IS NULL")
            .bind(token_hash(old).as_slice()).execute(&mut *transaction).await?;
    }
    let expires_at: i64 = sqlx::query_scalar(
        "INSERT INTO admin_sessions (id, token_hash, csrf_token_hash, google_sub, admin_email_hash, \
         config_hash, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, unixepoch(), unixepoch() + ?) \
         RETURNING expires_at"
    ).bind(Uuid::new_v4().to_string()).bind(token_hash(&token).as_slice())
        .bind(token_hash(&csrf).as_slice()).bind(subject).bind(token_hash(&config.email).as_slice())
        .bind(config.fingerprint.as_slice()).bind(SESSION_SECONDS).fetch_one(&mut *transaction).await?;
    transaction.commit().await?;
    Ok((
        token,
        SessionInfo {
            authenticated: true,
            csrf_token: csrf,
            expires_at,
        },
    ))
}

async fn session(Extension(session): Extension<Session>, request: Request) -> Response {
    if request.uri().query().is_some() {
        return invalid();
    }
    Json(SessionInfo {
        authenticated: true,
        csrf_token: session.csrf,
        expires_at: session.expires_at,
    })
    .into_response()
}

async fn logout(
    State(state): State<AdminState>,
    Extension(session): Extension<Session>,
    request: Request,
) -> Response {
    if request.uri().query().is_some() {
        return invalid();
    }
    let result = sqlx::query(
        "UPDATE admin_sessions SET revoked_at = unixepoch() WHERE id = ? AND revoked_at IS NULL",
    )
    .bind(session.id.to_string())
    .execute(&state.store.pool)
    .await;
    if result.is_err() {
        return unavailable();
    }
    let Some(config) = state.config.as_ref() else {
        return unavailable();
    };
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        config.cookie(config.session_cookie(), "", 0),
    );
    response
}

async fn keys(State(state): State<AdminState>, request: Request) -> Response {
    if request.uri().query().is_some() {
        return invalid();
    }
    match state.store.list_keys().await {
        Ok(keys) => Json(keys).into_response(),
        Err(_) => unavailable(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateKey {
    name: Option<String>,
    requests_per_minute: Option<i64>,
}

async fn create_key(
    State(state): State<AdminState>,
    Extension(session): Extension<Session>,
    request: Request,
) -> Response {
    if request.uri().query().is_some() {
        return invalid();
    }
    let Ok(Json(body)) = Json::<CreateKey>::from_request(request, &state).await else {
        return invalid();
    };
    match state
        .store
        .create_key(
            body.requests_per_minute.unwrap_or(60),
            body.name.as_deref(),
            KeyActor::Admin {
                session_id: session.id,
            },
        )
        .await
    {
        Ok(key) => (StatusCode::CREATED, Json(key)).into_response(),
        Err(error) => store_error(error),
    }
}

async fn revoke_key(
    State(state): State<AdminState>,
    Extension(session): Extension<Session>,
    path: Result<Path<String>, axum::extract::rejection::PathRejection>,
    request: Request,
) -> Response {
    let Ok(Path(id)) = path else {
        return invalid();
    };
    if request.uri().query().is_some() || !canonical_uuid(&id) {
        return invalid();
    }
    match state
        .store
        .revoke_key(
            &id,
            KeyActor::Admin {
                session_id: session.id,
            },
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => store_error(error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageQuery {
    key_id: Option<String>,
    from: Option<i64>,
    until: Option<i64>,
    kind: Option<String>,
}

async fn usage(State(state): State<AdminState>, request: Request) -> Response {
    let Ok(Query(query)) = Query::<UsageQuery>::try_from_uri(request.uri()) else {
        return invalid();
    };
    if query
        .key_id
        .as_deref()
        .is_some_and(|id| !canonical_uuid(id))
    {
        return invalid();
    }
    let kind = match query.kind.as_deref() {
        None => None,
        Some("inference") => Some(RequestKind::Inference),
        Some("models") => Some(RequestKind::Models),
        Some("auth_check") => Some(RequestKind::AuthCheck),
        Some("admin") => Some(RequestKind::Admin),
        Some("unknown") => Some(RequestKind::Unknown),
        _ => return invalid(),
    };
    match state
        .store
        .usage(UsageFilter {
            key_id: query.key_id.as_deref(),
            from: query.from,
            until: query.until,
            kind,
        })
        .await
    {
        Ok(report) => Json(report).into_response(),
        Err(error) => store_error(error),
    }
}

fn canonical_uuid(id: &str) -> bool {
    Uuid::parse_str(id)
        .is_ok_and(|uuid| uuid.hyphenated().encode_lower(&mut Uuid::encode_buffer()) == id)
}

fn store_error(error: Error) -> Response {
    match error {
        Error::InvalidLimit | Error::InvalidKeyName | Error::InvalidUsageFilter => invalid(),
        Error::KeyNotFound => api_error(StatusCode::NOT_FOUND, "not_found"),
        Error::AdminUnauthorized => denied(),
        _ => unavailable(),
    }
}

#[cfg(test)]
#[path = "admin_tests.rs"]
mod tests;
