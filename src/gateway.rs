use std::time::Instant;

use axum::{
    Json, Router,
    extract::{Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{AUTHORIZATION, CACHE_CONTROL, RETRY_AFTER, WWW_AUTHENTICATE},
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use uuid::Uuid;

use crate::db::{Admission, Store, elapsed_ms, token_hash, valid_token_format};

#[derive(Clone, Copy)]
struct RequestContext {
    id: Uuid,
    started: Instant,
}

/// 추론 경로는 등록하지 않는다. 인증 검사는 실제 키의 현재 허용 여부만 반환한다.
pub fn router(store: Store) -> Router {
    let protected = Router::new()
        .route("/v1/auth/check", get(|| async { StatusCode::NO_CONTENT }))
        .route_layer(middleware::from_fn_with_state(store.clone(), authenticate));
    Router::new()
        .route("/health/live", get(|| async { StatusCode::OK }))
        .route("/health/ready", get(ready))
        .merge(protected)
        .with_state(store)
        .layer(middleware::from_fn(observe))
}

async fn ready(State(store): State<Store>) -> Response {
    match store.ready().await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(_) => unavailable(),
    }
}

async fn observe(mut request: Request, next: Next) -> Response {
    let context = RequestContext {
        id: Uuid::new_v4(),
        started: Instant::now(),
    };
    // 클라이언트가 지정한 ID는 상관관계 식별자로 신뢰하지 않고 항상 교체한다.
    let mut encoded = Uuid::encode_buffer();
    let request_id = HeaderValue::from_str(context.id.hyphenated().encode_lower(&mut encoded))
        .expect("UUID는 유효한 ASCII 헤더다");
    request
        .headers_mut()
        .insert("x-request-id", request_id.clone());
    request.extensions_mut().insert(context);
    let mut response = next.run(request).await;
    response.headers_mut().insert("x-request-id", request_id);
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // URI·헤더·키·본문·DB 오류는 로깅하지 않는다. 이 전용 target만 구독한다.
    tracing::info!(
        target: "picksy::request",
        request_id = %context.id,
        status = response.status().as_u16(),
        duration_ms = elapsed_ms(context.started),
    );
    response
}

async fn authenticate(State(store): State<Store>, mut request: Request, next: Next) -> Response {
    let Some(context) = request.extensions().get::<RequestContext>().copied() else {
        return unavailable();
    };
    let Some(hash) = bearer(request.headers()).map(token_hash) else {
        return unauthorized();
    };
    // 인증 뒤 원문 Authorization이 후속 핸들러나 디버그 출력에 흘러가지 않게 제거한다.
    request.headers_mut().remove(AUTHORIZATION);
    let request_id = context.id.to_string();
    match store.admit(&hash, &request_id, context.started).await {
        Ok(Admission::Unauthorized) => unauthorized(),
        Err(_) => unavailable(),
        Ok(Admission::Limited { retry_after }) => {
            let mut response = api_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
            response.headers_mut().insert(
                RETRY_AFTER,
                HeaderValue::from_str(&retry_after.to_string()).expect("재시도 시간은 양의 정수다"),
            );
            response
        }
        Ok(Admission::Allowed) => {
            let response = next.run(request).await;
            // 이번 단계는 본문 없는 204만 제공한다. 완료는 응답 전 기록이며 전달 보장은 아니다.
            // 완료 UPDATE 실패 시 pending을 보존하고 HTTP 응답은 503으로 대체한다.
            if store
                .complete(
                    &request_id,
                    response.status().as_u16(),
                    elapsed_ms(context.started),
                )
                .await
                .is_err()
            {
                return unavailable();
            }
            response
        }
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let value = values.next()?;
    // 중복 필드와 쉼표로 합쳐진 자격 증명은 모두 단일한 401로 처리한다.
    if values.next().is_some() {
        return None;
    }
    let (scheme, token) = value.to_str().ok()?.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    let token = token.trim_start_matches(' ');
    valid_token_format(token).then_some(token)
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

fn api_error(status: StatusCode, error: &'static str) -> Response {
    (status, Json(ErrorBody { error })).into_response()
}

fn unauthorized() -> Response {
    let mut response = api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    response
        .headers_mut()
        .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

fn unavailable() -> Response {
    api_error(StatusCode::SERVICE_UNAVAILABLE, "unavailable")
}
