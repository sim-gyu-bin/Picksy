use std::time::Instant;

use axum::{
    Json, Router,
    extract::{MatchedPath, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{AUTHORIZATION, CACHE_CONTROL, RETRY_AFTER, WWW_AUTHENTICATE},
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use uuid::Uuid;

use crate::db::{
    Admission, RequestKind, Store, TokenUsage, elapsed_ms, token_hash, valid_token_format,
};
use crate::inference::{self, Inference};

#[derive(Clone, Copy)]
struct RequestContext {
    id: Uuid,
    started: Instant,
}

#[derive(Clone)]
struct Readiness {
    store: Store,
    inference: Inference,
}

/// 목록·추론·인증 확인이 같은 인증·공유 쿼터·사용량 경계를 통과한다.
pub fn router(store: Store, inference: Inference) -> Router {
    router_with_admin(store, inference, None)
}

/// 관리자 설정은 명시적으로 주입하며 기존 추론 전용 호출은 환경변수를 읽지 않는다.
pub fn router_with_admin(
    store: Store,
    inference: Inference,
    config: Option<crate::admin::AdminConfig>,
) -> Router {
    let protected = Router::new()
        .route("/v1/auth/check", get(|| async { StatusCode::NO_CONTENT }))
        .route("/v1/models", get(inference::models))
        .route("/v1/systemone", post(inference::systemone))
        .with_state(inference.clone())
        .route_layer(middleware::from_fn_with_state(store.clone(), authenticate));
    let admin = crate::admin::router(store.clone(), config);
    Router::new()
        .route("/health/live", get(|| async { StatusCode::OK }))
        .route("/health/ready", get(ready))
        .with_state(Readiness { store, inference })
        .merge(protected)
        .merge(admin)
        .layer(middleware::from_fn(observe))
}

async fn ready(State(state): State<Readiness>) -> Response {
    let result = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        let (database, inference) = tokio::join!(state.store.ready(), state.inference.ready());
        database.is_ok() && inference
    })
    .await;
    if matches!(result, Ok(true)) {
        StatusCode::OK.into_response()
    } else {
        unavailable()
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
    // 라우터가 확정한 경로만 신뢰한다. 쿼리·헤더·본문의 종류 주장은 사용하지 않는다.
    let kind = match request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
    {
        Some("/v1/systemone") => RequestKind::Inference,
        Some("/v1/models") => RequestKind::Models,
        Some("/v1/auth/check") => RequestKind::AuthCheck,
        _ => return unavailable(),
    };
    // 인증 뒤 원문 Authorization이 후속 핸들러나 디버그 출력에 흘러가지 않게 제거한다.
    request.headers_mut().remove(AUTHORIZATION);
    let request_id = context.id.to_string();
    // 접수 커밋 중 HTTP future가 drop되어도 DB 결과를 회수한다. 추론은 이 작업에 넣지 않는다.
    // 수신 측이 사라지면 guard가 499를 기록하므로 커밋 성공 직후의 취소 틈도 막는다.
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let admission_store = store.clone();
    tokio::spawn(async move {
        let result = admission_store
            .admit(&hash, &request_id, context.started, kind)
            .await;
        let guard = matches!(&result, Ok(Admission::Allowed)).then(|| UsageGuard {
            store: admission_store,
            request_id,
            started: context.started,
            armed: true,
        });
        let _ = sender.send((result, guard));
    });
    let Ok((admission, mut guard)) = receiver.await else {
        return unavailable();
    };
    match admission {
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
            let guard = guard.as_mut().expect("허용한 접수에는 완료 guard가 있다");
            // 완료는 클라이언트 수신 확인이 아니라 서버 응답 생성 경계다.
            // UPDATE 중 drop되면 guard가 남은 pending만 499로 종료하며 이미 완료된 값은 덮지 않는다.
            // 검증된 추론 핸들러의 내부 확장만 전달하고 응답 본문은 복사·재해석하지 않는다.
            let tokens = if matches!(kind, RequestKind::Inference) && response.status().is_success()
            {
                response.extensions().get::<TokenUsage>().copied()
            } else {
                None
            };
            let completed = store
                .complete(
                    &guard.request_id,
                    response.status().as_u16(),
                    elapsed_ms(context.started),
                    tokens,
                )
                .await;
            guard.armed = false;
            if completed.is_err() {
                return unavailable();
            }
            response
        }
    }
}

struct UsageGuard {
    store: Store,
    request_id: String,
    started: Instant,
    armed: bool,
}

impl Drop for UsageGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let store = self.store.clone();
        let id = std::mem::take(&mut self.request_id);
        let duration = elapsed_ms(self.started);
        // HTTP 서비스 future의 drop만 관찰한다. TCP 단절 감지·상류 계산 즉시 중단은 별도 경계다.
        // 동기 Drop에서는 DB만 비동기 마무리한다. 런타임 강제 종료·DB 장애 시 pending은 보존된다.
        tokio::spawn(async move {
            if store.cancel(&id, duration).await.is_err() {
                tracing::error!(target: "picksy::request", request_id = %id, status = 499, duration_ms = duration);
            }
        });
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

pub(crate) fn api_error(status: StatusCode, error: &'static str) -> Response {
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
