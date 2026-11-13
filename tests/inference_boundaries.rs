use std::{
    convert::Infallible,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::stream;
use picksy::{
    db, gateway,
    inference::{Inference, Limits, MODEL_ID},
};
use serde_json::{Value, json};
use sqlx::{
    ConnectOptions, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::timeout,
};
use tower::ServiceExt;
use uuid::Uuid;

// 제어 상류는 오류 경계 회귀 검사용이다. 실제 d1 품질·성능·호환성 증거가 아니다.
struct Captured {
    headers: HeaderMap,
    body: Value,
    reply: oneshot::Sender<Response>,
}

#[derive(Clone)]
struct UpstreamState {
    calls: mpsc::Sender<Captured>,
    health: Arc<AtomicU16>,
}

struct Fixture {
    store: db::Store,
    key: String,
    admin: SqlitePool,
    app: Router,
    calls: mpsc::Receiver<Captured>,
    health: Arc<AtomicU16>,
    server: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn fixture(limits: Limits) -> Fixture {
    let (calls, receiver) = mpsc::channel(32);
    let health = Arc::new(AtomicU16::new(200));
    let state = UpstreamState {
        calls,
        health: health.clone(),
    };
    let upstream = Router::new()
        .route("/v1/systemone", post(capture))
        .route(
            "/health",
            get(
                |State(state): State<UpstreamState>, headers: HeaderMap| async move {
                    assert!(!headers.contains_key("authorization"));
                    StatusCode::from_u16(state.health.load(Ordering::SeqCst)).unwrap()
                },
            ),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    // 이전 인증 테스트와 같이 실행별 합성 DB를 보존하며 자동 삭제하지 않는다.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".local/picksy-dev/gateway-tests")
        .join(Uuid::new_v4().to_string())
        .join("gateway.sqlite3");
    db::migrate(&path).await.unwrap();
    let store = db::Store::open(&path).await.unwrap();
    let admin = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .disable_statement_logging(),
        )
        .await
        .unwrap();
    let key = store
        .create_key(10000, None, db::KeyActor::Cli)
        .await
        .unwrap()
        .api_key;
    let inference = Inference::new(&origin, "synthetic-upstream-key", limits).unwrap();
    let app = gateway::router(store.clone(), inference);
    Fixture {
        store,
        key,
        app,
        calls: receiver,
        health,
        server,
        admin,
    }
}

async fn capture(State(state): State<UpstreamState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = serde_json::from_slice(&to_bytes(body, 65536).await.unwrap()).unwrap();
    let (reply, response) = oneshot::channel();
    state
        .calls
        .send(Captured {
            headers: parts.headers,
            body,
            reply,
        })
        .await
        .unwrap();
    response
        .await
        .unwrap_or_else(|_| StatusCode::SERVICE_UNAVAILABLE.into_response())
}

fn payload() -> Value {
    json!({"model": MODEL_ID, "state": {"상태": ["텍스트", 3, true]}, "questions": {
        "z-choice": {"type":"choice", "instructions":"선택", "criteria":{"z":null,"a":"다른 선택"}},
        "a-noul": {"type":"noul", "instructions":"판단", "criteria":{"true":"예","false":"아니오"}},
        "m-score": {"type":"score", "instructions":"점수", "criteria":["낮음","중간","높음"]}
    }})
}

fn answer() -> Value {
    json!({"model":"/models/d1-2B-Q4_K_M.gguf", "answers": {
        "z-choice":{"type":"choice","choice":"z","probabilities":{"z":0.8,"a":0.2},"confidence":0.8},
        "a-noul":{"type":"noul","noul":0.73},
        "m-score":{"type":"score","score":1.0,"legend":{"0":"낮음","1":"중간","2":"높음"},"probabilities":{"0":0.2,"1":0.5,"2":0.3},"confidence":0.5}
    }, "usage":{"input_tokens":140,"output_tokens":0}})
}

fn request(key: &str, body: Body, content_type: Option<&str>) -> Request {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/systemone")
        .header("authorization", format!("Bearer {key}"));
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    builder.body(body).unwrap()
}

fn submit(fixture: &Fixture, payload: &Value) -> JoinHandle<Response> {
    let app = fixture.app.clone();
    let request = request(
        &fixture.key,
        Body::from(serde_json::to_vec(payload).unwrap()),
        Some("application/json"),
    );
    tokio::spawn(async move { app.oneshot(request).await.unwrap() })
}

async fn captured(fixture: &mut Fixture) -> Captured {
    timeout(Duration::from_secs(3), fixture.calls.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn response_json(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 300000).await.unwrap()).unwrap()
}

async fn assert_error(response: Response, status: StatusCode, error: &str) {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(Uuid::parse_str(response.headers()["x-request-id"].to_str().unwrap()).is_ok());
    assert_eq!(response_json(response).await, json!({"error":error}));
}

#[tokio::test]
async fn real_http_preserves_input_order_values_and_validated_output_without_internal_fields() {
    let mut fixture = fixture(Limits::default()).await;
    let task = submit(&fixture, &payload());
    let captured = captured(&mut fixture).await;
    assert_eq!(
        captured.headers["authorization"],
        format!("Bearer {}", "synthetic-upstream-key")
    );
    assert!(!captured.headers.contains_key("x-request-id"));
    let mut expected = payload();
    expected.as_object_mut().unwrap().shift_remove("model");
    assert_eq!(captured.body, expected);
    assert_eq!(
        captured.body["questions"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["z-choice", "a-noul", "m-score"]
    );
    assert_eq!(
        captured.body["questions"]["z-choice"]["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
    let mut result = answer();
    result["routing"] = json!("private-origin");
    result["usage"]["internal"] = json!("private-origin");
    result["answers"]["z-choice"]["debug"] = json!("private-origin");
    result["answers"]["a-noul"]["debug"] = json!("private-origin");
    captured.reply.send(Json(result).into_response()).unwrap();
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let output = response_json(response).await;
    let mut expected = answer();
    expected["model"] = json!(MODEL_ID);
    assert_eq!(output, expected);
    assert!(output.get("cost").is_none());
    let usage = fixture
        .store
        .usage(db::UsageFilter::default())
        .await
        .unwrap()
        .summary;
    assert_eq!(
        (usage.input_tokens, usage.output_tokens),
        (Some(140), Some(0))
    );
    assert_eq!((usage.token_recorded, usage.token_unknown), (1, 0));
    let stored: (String, i64, i64) =
        sqlx::query_as("SELECT request_kind, input_tokens, output_tokens FROM request_usage")
            .fetch_one(&fixture.admin)
            .await
            .unwrap();
    assert_eq!(stored, ("inference".to_owned(), 140, 0));
    assert_eq!((usage.total, usage.succeeded, usage.pending), (1, 1, 0));
}

#[tokio::test]
async fn invalid_requests_never_reach_upstream_and_are_counted() {
    let mut fixture = fixture(Limits::default()).await;
    let mut invalid = Vec::new();
    for state in [Value::Null, json!(true), json!(42)] {
        let mut value = payload();
        value["state"] = state;
        invalid.push(value);
    }
    for field in ["images", "assets", "unknown"] {
        let mut value = payload();
        value[field] = json!([]);
        invalid.push(value);
    }
    for question in [
        json!({"type":"box","instructions":"상자"}),
        json!({"type":"noul","instructions":""}),
        json!({"type":"noul","instructions":42}),
        json!({"type":"noul","instructions":"가".repeat(1366)}),
        json!({"type":"noul","instructions":"판단","typo":true}),
        json!({"type":"noul","instructions":"판단","criteria":{"maybe":"가능"}}),
        json!({"type":"noul","instructions":"판단","criteria":{"true":null}}),
        json!({"type":"choice","instructions":"선택","criteria":{"one":"하나"}}),
        json!({"type":"choice","instructions":"선택","criteria":{"":"하나","b":"둘"}}),
        json!({"type":"choice","instructions":"선택","criteria":{"a":1,"b":"둘"}}),
        json!({"type":"score","instructions":"점수","criteria":["하나"]}),
        json!({"type":"score","instructions":"점수","criteria":["하나",null]}),
        json!({"type":"score","instructions":"점수","criteria":vec!["등급";11]}),
    ] {
        let mut value = payload();
        value["questions"] = json!({"q":question});
        invalid.push(value);
    }
    let mut value = payload();
    value["questions"] = json!({});
    invalid.push(value);
    let mut value = payload();
    value.as_object_mut().unwrap().shift_remove("model");
    invalid.push(value);
    let mut value = payload();
    value["model"] = Value::Null;
    invalid.push(value);
    for key in [String::new(), "x".repeat(129)] {
        let mut value = payload();
        value["questions"] = Value::Object(
            [(key, json!({"type":"noul","instructions":"판단"}))]
                .into_iter()
                .collect(),
        );
        invalid.push(value);
    }
    let mut value = payload();
    value["questions"] = Value::Object(
        (0..9)
            .map(|i| (i.to_string(), json!({"type":"noul","instructions":"판단"})))
            .collect(),
    );
    invalid.push(value);
    let count = invalid.len() as i64;
    for value in invalid {
        assert_error(
            submit(&fixture, &value).await.unwrap(),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        )
        .await;
    }
    let mut unknown = payload();
    unknown["model"] = json!("other");
    assert_error(
        submit(&fixture, &unknown).await.unwrap(),
        StatusCode::BAD_REQUEST,
        "unsupported_model",
    )
    .await;
    assert!(fixture.calls.try_recv().is_err());
    let usage = fixture
        .store
        .usage(db::UsageFilter::default())
        .await
        .unwrap()
        .summary;
    assert_eq!((usage.input_tokens, usage.output_tokens), (None, None));
    assert_eq!((usage.token_recorded, usage.token_unknown), (0, count + 1));
    assert_eq!(
        (usage.total, usage.rejected, usage.pending),
        (count + 1, count + 1, 0)
    );
}

#[tokio::test]
async fn body_size_syntax_and_media_type_have_safe_errors() {
    let fixture = fixture(Limits::default()).await;
    for (body, content_type, status, error) in [
        (
            vec![b'x'; 65537],
            Some("application/json"),
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
        ),
        (
            b"{".to_vec(),
            Some("application/json"),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        ),
        (
            b"{}".to_vec(),
            None,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
        ),
        (
            b"{}".to_vec(),
            Some("text/plain"),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
        ),
    ] {
        let response = fixture
            .app
            .clone()
            .oneshot(request(&fixture.key, Body::from(body), content_type))
            .await
            .unwrap();
        assert_error(response, status, error).await;
    }
    let chunks = stream::iter([
        Ok::<_, Infallible>(Bytes::from(vec![b'x'; 40000])),
        Ok(Bytes::from(vec![b'x'; 40000])),
    ]);
    let response = fixture
        .app
        .clone()
        .oneshot(request(
            &fixture.key,
            Body::from_stream(chunks),
            Some("application/json"),
        ))
        .await
        .unwrap();
    assert_error(response, StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large").await;
}

#[tokio::test]
async fn invalid_upstream_results_are_not_repaired_or_reflected() {
    let mut fixture = fixture(Limits::default()).await;
    let mut invalid = Vec::new();
    for (pointer, replacement) in [
        ("/model", json!("private-origin")),
        ("/usage/input_tokens", json!(-1)),
        ("/usage/output_tokens", json!(1.5)),
        ("/usage/input_tokens", Value::Null),
        ("/usage/input_tokens", json!(i64::MAX as u64 + 1)),
        ("/usage/output_tokens", json!(u64::MAX)),
        ("/answers/z-choice/type", json!("score")),
        ("/answers/z-choice/choice", json!("absent")),
        ("/answers/z-choice/choice", json!("a")),
        ("/answers/z-choice/confidence", json!(0.6)),
        ("/answers/z-choice/probabilities/z", json!(1.2)),
        ("/answers/z-choice/probabilities/a", json!(-0.1)),
        ("/answers/z-choice/probabilities/z", json!(0.6)),
        (
            "/answers/z-choice/probabilities",
            json!({"z":0.8,"other":0.2}),
        ),
        ("/answers/m-score/score", json!(1.1)),
        ("/answers/m-score/score", json!(2)),
        ("/answers/m-score/legend/1", json!("조작")),
        (
            "/answers/m-score/probabilities",
            json!({"0":0.2,"1":0.5,"3":0.3}),
        ),
        ("/answers/a-noul/noul", json!(1.01)),
        ("/answers/a-noul/noul", json!("NaN")),
    ] {
        let mut value = answer();
        *value.pointer_mut(pointer).unwrap() = replacement;
        invalid.push(value);
    }
    for field in ["model", "usage", "answers"] {
        let mut value = answer();
        value.as_object_mut().unwrap().shift_remove(field);
        invalid.push(value);
    }
    let mut value = answer();
    value["usage"]
        .as_object_mut()
        .unwrap()
        .shift_remove("output_tokens");
    invalid.push(value);
    let mut value = answer();
    value["answers"]
        .as_object_mut()
        .unwrap()
        .shift_remove("a-noul");
    invalid.push(value);
    let mut value = answer();
    value["answers"]["extra"] = json!({"type":"noul","noul":0.5});
    invalid.push(value);
    for value in invalid {
        let task = submit(&fixture, &payload());
        captured(&mut fixture)
            .await
            .reply
            .send(Json(value).into_response())
            .unwrap();
        assert_error(
            task.await.unwrap(),
            StatusCode::BAD_GATEWAY,
            "inference_unavailable",
        )
        .await;
    }
    let usage = fixture
        .store
        .usage(db::UsageFilter::default())
        .await
        .unwrap()
        .summary;
    assert_eq!((usage.input_tokens, usage.output_tokens), (None, None));
    assert_eq!(
        (usage.token_recorded, usage.token_unknown),
        (0, usage.total)
    );
    assert_eq!(usage.total, usage.failed);
    assert_eq!(usage.pending, 0);
}

#[tokio::test]
async fn upstream_errors_redirects_and_chunked_overflow_are_safe_and_not_retried() {
    let mut fixture = fixture(Limits::default()).await;
    for status in [
        StatusCode::UNAUTHORIZED,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::TEMPORARY_REDIRECT,
    ] {
        let task = submit(&fixture, &payload());
        let response = Response::builder()
            .status(status)
            .header("location", "http://127.0.0.1:9/private")
            .body(Body::from(
                "private-origin private-body synthetic-upstream-key",
            ))
            .unwrap();
        captured(&mut fixture).await.reply.send(response).unwrap();
        assert_error(
            task.await.unwrap(),
            StatusCode::BAD_GATEWAY,
            "inference_unavailable",
        )
        .await;
        assert!(fixture.calls.try_recv().is_err());
    }
    let task = submit(&fixture, &payload());
    let chunks = stream::iter((0..5).map(|_| Ok::<_, Infallible>(Bytes::from(vec![b'x'; 65536]))));
    captured(&mut fixture)
        .await
        .reply
        .send(Response::new(Body::from_stream(chunks)))
        .unwrap();
    assert_error(
        task.await.unwrap(),
        StatusCode::BAD_GATEWAY,
        "inference_unavailable",
    )
    .await;
    let task = submit(&fixture, &payload());
    captured(&mut fixture)
        .await
        .reply
        .send(Response::new(Body::from("{invalid private-body")))
        .unwrap();
    assert_error(
        task.await.unwrap(),
        StatusCode::BAD_GATEWAY,
        "inference_unavailable",
    )
    .await;
    let usage = fixture
        .store
        .usage(db::UsageFilter::default())
        .await
        .unwrap()
        .summary;
    assert_eq!((usage.input_tokens, usage.output_tokens), (None, None));
    assert_eq!((usage.token_recorded, usage.token_unknown), (0, 6));
}

#[tokio::test]
async fn timeouts_include_upstream_body_and_release_slots() {
    let limits = Limits {
        inference_timeout: Duration::from_millis(100),
        ..Limits::default()
    };
    let mut fixture = fixture(limits).await;
    let first = submit(&fixture, &payload());
    let held = captured(&mut fixture).await;
    assert_error(
        first.await.unwrap(),
        StatusCode::GATEWAY_TIMEOUT,
        "inference_timeout",
    )
    .await;
    drop(held);
    let second = submit(&fixture, &payload());
    let stalled = Body::from_stream(stream::pending::<Result<Bytes, Infallible>>());
    captured(&mut fixture)
        .await
        .reply
        .send(Response::new(stalled))
        .unwrap();
    assert_error(
        second.await.unwrap(),
        StatusCode::GATEWAY_TIMEOUT,
        "inference_timeout",
    )
    .await;
    let third = submit(&fixture, &payload());
    captured(&mut fixture)
        .await
        .reply
        .send(Json(answer()).into_response())
        .unwrap();
    assert_eq!(third.await.unwrap().status(), StatusCode::OK);
    let usage = fixture
        .store
        .usage(db::UsageFilter::default())
        .await
        .unwrap()
        .summary;
    assert_eq!(
        (usage.input_tokens, usage.output_tokens),
        (Some(140), Some(0))
    );
    assert_eq!((usage.token_recorded, usage.token_unknown), (1, 2));
    let incomplete_tokens: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM request_usage WHERE status = 504 \
         AND input_tokens IS NULL AND output_tokens IS NULL",
    )
    .fetch_one(&fixture.admin)
    .await
    .unwrap();
    assert_eq!(incomplete_tokens, 2);
    assert_eq!((usage.failed, usage.succeeded, usage.pending), (2, 1, 0));
}

#[tokio::test]
async fn waiting_deadline_returns_overload_without_starting_another_inference() {
    let mut fixture = fixture(Limits {
        queue_timeout: Duration::from_millis(50),
        ..Limits::default()
    })
    .await;
    let first = submit(&fixture, &payload());
    let held = captured(&mut fixture).await;
    let response = submit(&fixture, &payload()).await.unwrap();
    assert_eq!(response.headers()["retry-after"], "1");
    assert_error(response, StatusCode::SERVICE_UNAVAILABLE, "overloaded").await;
    assert!(fixture.calls.try_recv().is_err());
    held.reply.send(Json(answer()).into_response()).unwrap();
    assert_eq!(first.await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn admission_is_bounded_before_body_read_and_slow_body_times_out() {
    let limits = Limits {
        queue: 1,
        body_timeout: Duration::from_millis(200),
        ..Limits::default()
    };
    let mut fixture = fixture(limits).await;
    let first = submit(&fixture, &payload());
    let held = captured(&mut fixture).await;
    let (reading, started) = oneshot::channel();
    let slow = stream::once(async move {
        reading.send(()).unwrap();
        std::future::pending::<Result<Bytes, Infallible>>().await
    });
    let app = fixture.app.clone();
    let request = request(
        &fixture.key,
        Body::from_stream(slow),
        Some("application/json"),
    );
    let second = tokio::spawn(async move { app.oneshot(request).await.unwrap() });
    started.await.unwrap();
    let response = submit(&fixture, &payload()).await.unwrap();
    assert_eq!(response.headers()["retry-after"], "1");
    assert_error(response, StatusCode::SERVICE_UNAVAILABLE, "overloaded").await;
    assert_error(
        second.await.unwrap(),
        StatusCode::GATEWAY_TIMEOUT,
        "inference_timeout",
    )
    .await;
    held.reply.send(Json(answer()).into_response()).unwrap();
    assert_eq!(first.await.unwrap().status(), StatusCode::OK);
    let fourth = submit(&fixture, &payload());
    captured(&mut fixture)
        .await
        .reply
        .send(Json(answer()).into_response())
        .unwrap();
    assert_eq!(fourth.await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn service_future_cancellation_releases_inference_and_finishes_usage_as_499() {
    let mut fixture = fixture(Limits::default()).await;
    let first = submit(&fixture, &payload());
    let held = captured(&mut fixture).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    await_cancelled(&fixture, 1).await;
    // 취소는 서비스 future 경계 검사다. 실제 TCP 단절 감지 보장은 별도의 실행 검증 대상이다.
    timeout(Duration::from_secs(3), async {
        loop {
            let usage = fixture
                .store
                .usage(db::UsageFilter::default())
                .await
                .unwrap()
                .summary;
            if usage.pending == 0 {
                assert_eq!((usage.total, usage.rejected, usage.succeeded), (1, 1, 0));
                assert_eq!((usage.input_tokens, usage.output_tokens), (None, None));
                assert_eq!((usage.token_recorded, usage.token_unknown), (0, 1));
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let next = submit(&fixture, &payload());
    captured(&mut fixture)
        .await
        .reply
        .send(Json(answer()).into_response())
        .unwrap();
    assert_eq!(next.await.unwrap().status(), StatusCode::OK);
    drop(held);
}

#[tokio::test]
async fn readiness_requires_database_and_upstream_while_liveness_is_independent() {
    let fixture = fixture(Limits::default()).await;
    for (health, expected) in [
        (200, StatusCode::OK),
        (503, StatusCode::SERVICE_UNAVAILABLE),
    ] {
        fixture.health.store(health, Ordering::SeqCst);
        for (path, status) in [
            ("/health/ready", expected),
            ("/health/live", StatusCode::OK),
        ] {
            let response = fixture
                .app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), status);
        }
    }
    fixture.health.store(200, Ordering::SeqCst);
    fixture.store.close().await;
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_error(response, StatusCode::SERVICE_UNAVAILABLE, "unavailable").await;
}

#[test]
fn configuration_rejects_non_origins_and_invalid_limits() {
    for origin in [
        "ftp://localhost",
        "http://user:secret@localhost",
        "http://@localhost",
        "http://localhost/v1",
        "http://localhost//",
        "http://localhost/./",
        "http://localhost?x=1",
        "http://localhost/#part",
    ] {
        assert!(Inference::new(origin, "synthetic", Limits::default()).is_err());
    }
    for key in ["", " spaced", "bad\nheader"] {
        assert!(Inference::new("http://localhost", key, Limits::default()).is_err());
    }
    for limits in [
        Limits {
            concurrency: 0,
            ..Limits::default()
        },
        Limits {
            queue: 0,
            ..Limits::default()
        },
        Limits {
            queue_timeout: Duration::ZERO,
            ..Limits::default()
        },
        Limits {
            inference_timeout: Duration::ZERO,
            ..Limits::default()
        },
        Limits {
            body_timeout: Duration::ZERO,
            ..Limits::default()
        },
        Limits {
            concurrency: usize::MAX,
            ..Limits::default()
        },
    ] {
        assert!(Inference::new("http://localhost", "synthetic", limits).is_err());
    }
}

async fn await_cancelled(fixture: &Fixture, count: i64) {
    timeout(Duration::from_secs(3), async {
        loop {
            let cancelled: i64 =
                sqlx::query_scalar(
                    "SELECT COUNT(*) FROM request_usage WHERE status = 499 \
                     AND request_kind = 'inference' AND input_tokens IS NULL AND output_tokens IS NULL",
                )
                    .fetch_one(&fixture.admin)
                    .await
                    .unwrap();
            if cancelled == count {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_during_database_admission_does_not_leave_pending() {
    let mut fixture = fixture(Limits::default()).await;
    let lock = fixture.admin.begin_with("BEGIN IMMEDIATE").await.unwrap();
    {
        let request = request(
            &fixture.key,
            Body::from(serde_json::to_vec(&payload()).unwrap()),
            Some("application/json"),
        );
        let mut future = Box::pin(fixture.app.clone().oneshot(request));
        // 접수 커밋을 DB 쓰기 잠금으로 막은 상태에서 HTTP future를 실제 poll한 뒤 drop한다.
        assert!(futures_util::poll!(future.as_mut()).is_pending());
    }
    lock.rollback().await.unwrap();
    await_cancelled(&fixture, 1).await;
    let usage = fixture
        .store
        .usage(db::UsageFilter::default())
        .await
        .unwrap()
        .summary;
    assert_eq!((usage.total, usage.pending, usage.rejected), (1, 0, 1));
    assert!(fixture.calls.try_recv().is_err());
}

#[tokio::test]
async fn cancellation_while_waiting_releases_admission_for_the_next_request() {
    let mut fixture = fixture(Limits {
        queue: 1,
        queue_timeout: Duration::from_secs(2),
        ..Limits::default()
    })
    .await;
    let first = submit(&fixture, &payload());
    let held = captured(&mut fixture).await;
    let (reading, started) = oneshot::channel();
    let bytes = Bytes::from(serde_json::to_vec(&payload()).unwrap());
    let body = Body::from_stream(stream::once(async move {
        reading.send(()).unwrap();
        Ok::<_, Infallible>(bytes)
    }));
    let app = fixture.app.clone();
    let request = request(&fixture.key, body, Some("application/json"));
    let second = tokio::spawn(async move { app.oneshot(request).await.unwrap() });
    started.await.unwrap();
    second.abort();
    assert!(second.await.unwrap_err().is_cancelled());
    await_cancelled(&fixture, 1).await;
    let third = submit(&fixture, &payload());
    held.reply.send(Json(answer()).into_response()).unwrap();
    assert_eq!(first.await.unwrap().status(), StatusCode::OK);
    captured(&mut fixture)
        .await
        .reply
        .send(Json(answer()).into_response())
        .unwrap();
    assert_eq!(third.await.unwrap().status(), StatusCode::OK);
    let usage = fixture
        .store
        .usage(db::UsageFilter::default())
        .await
        .unwrap()
        .summary;
    assert_eq!(
        (usage.total, usage.succeeded, usage.rejected, usage.pending),
        (3, 2, 1, 0)
    );
}

#[tokio::test]
async fn unavailable_connection_fails_closed_and_does_not_break_liveness() {
    let mut fixture = fixture(Limits::default()).await;
    fixture.server.abort();
    assert!((&mut fixture.server).await.unwrap_err().is_cancelled());
    assert_error(
        submit(&fixture, &payload()).await.unwrap(),
        StatusCode::BAD_GATEWAY,
        "inference_unavailable",
    )
    .await;
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn accepted_limits_include_empty_state_eight_questions_and_ten_score_levels() {
    let mut fixture = fixture(Limits::default()).await;
    let mut input = payload();
    input["state"] = json!("");
    input["questions"] = Value::Object(
        (0..8)
            .map(|index| {
                (
                    format!("{index:0>128}"),
                    json!({"type":"noul","instructions":"x".repeat(4096)}),
                )
            })
            .collect(),
    );
    let mut result = answer();
    result["answers"] = Value::Object(
        input["questions"]
            .as_object()
            .unwrap()
            .keys()
            .map(|key| (key.clone(), json!({"type":"noul","noul":0.5})))
            .collect(),
    );
    let task = submit(&fixture, &input);
    let first_call = captured(&mut fixture).await;
    assert_eq!(first_call.body["state"], "");
    assert_eq!(first_call.body["questions"], input["questions"]);
    first_call.reply.send(Json(result).into_response()).unwrap();
    assert_eq!(task.await.unwrap().status(), StatusCode::OK);

    let criteria: Vec<_> = (0..10).map(|index| index.to_string()).collect();
    input["state"] = json!([]);
    input["questions"] =
        json!({"score":{"type":"score","instructions":"등급","criteria":criteria}});
    let legend: serde_json::Map<_, _> = (0..10)
        .map(|index| (index.to_string(), json!(index.to_string())))
        .collect();
    let probabilities: serde_json::Map<_, _> = (0..10)
        .map(|index| (index.to_string(), json!(if index == 9 { 1.0 } else { 0.0 })))
        .collect();
    let mut result = answer();
    result["answers"] = json!({"score":{"type":"score","score":9,"legend":legend,"probabilities":probabilities,"confidence":1.0}});
    let task = submit(&fixture, &input);
    captured(&mut fixture)
        .await
        .reply
        .send(Json(result).into_response())
        .unwrap();
    assert_eq!(task.await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn exact_body_limit_is_accepted_without_synthesizing_usage_or_cost() {
    let mut fixture = fixture(Limits::default()).await;
    let mut input = payload();
    input["state"] = json!("");
    let base = serde_json::to_vec(&input).unwrap().len();
    input["state"] = json!("x".repeat(65536 - base));
    assert_eq!(serde_json::to_vec(&input).unwrap().len(), 65536);
    let task = submit(&fixture, &input);
    let mut result = answer();
    result["usage"]["input_tokens"] = json!(123);
    result["usage"]["output_tokens"] = json!(7);
    result["cost"] = json!(0.125);
    captured(&mut fixture)
        .await
        .reply
        .send(Json(result).into_response())
        .unwrap();
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["usage"], json!({"input_tokens":123,"output_tokens":7}));
    assert_eq!(body["cost"], 0.125);
    let usage = fixture
        .store
        .usage(db::UsageFilter::default())
        .await
        .unwrap()
        .summary;
    assert_eq!(
        (usage.input_tokens, usage.output_tokens),
        (Some(123), Some(7))
    );
    assert_eq!((usage.token_recorded, usage.token_unknown), (1, 0));
}

#[tokio::test]
async fn choice_count_and_identifier_byte_limits_are_enforced() {
    let mut fixture = fixture(Limits::default()).await;
    for criteria in [
        Value::Object(
            (0..17)
                .map(|index| (index.to_string(), Value::Null))
                .collect(),
        ),
        Value::Object(
            [
                ("x".repeat(129), Value::Null),
                ("b".to_owned(), Value::Null),
            ]
            .into_iter()
            .collect(),
        ),
    ] {
        let mut input = payload();
        input["questions"] =
            json!({"q":{"type":"choice","instructions":"선택","criteria":criteria}});
        assert_error(
            submit(&fixture, &input).await.unwrap(),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        )
        .await;
    }
    assert!(fixture.calls.try_recv().is_err());
    let criteria: serde_json::Map<_, _> = (0..16)
        .map(|index| (format!("{index:0>128}"), Value::Null))
        .collect();
    let probabilities: serde_json::Map<_, _> = (0..16)
        .map(|index| {
            (
                format!("{index:0>128}"),
                json!(if index == 0 { 1.0 } else { 0.0 }),
            )
        })
        .collect();
    let mut input = payload();
    input["questions"] = json!({"q":{"type":"choice","instructions":"선택","criteria":criteria}});
    let mut output = answer();
    output["answers"] = json!({"q":{"type":"choice","choice":format!("{:0>128}",0),"probabilities":probabilities,"confidence":1.0}});
    let task = submit(&fixture, &input);
    captured(&mut fixture)
        .await
        .reply
        .send(Json(output).into_response())
        .unwrap();
    assert_eq!(task.await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn broken_request_body_is_not_misclassified_as_oversized() {
    let fixture = fixture(Limits::default()).await;
    let broken = stream::iter([Err::<Bytes, _>(std::io::Error::other(
        "합성 본문 수신 실패",
    ))]);
    let response = fixture
        .app
        .clone()
        .oneshot(request(
            &fixture.key,
            Body::from_stream(broken),
            Some("application/json"),
        ))
        .await
        .unwrap();
    assert_error(response, StatusCode::BAD_REQUEST, "invalid_request").await;
}

#[tokio::test]
async fn actual_zero_and_maximum_sqlite_token_counts_are_preserved_exactly() {
    let mut fixture = fixture(Limits::default()).await;
    for tokens in [0, i64::MAX] {
        let task = submit(&fixture, &payload());
        let mut result = answer();
        result["usage"] = json!({"input_tokens": tokens, "output_tokens": tokens});
        // 상류 헤더의 주장도 무시하고 검증된 JSON 정수만 사용한다.
        let mut upstream = Json(result).into_response();
        upstream
            .headers_mut()
            .insert("x-input-tokens", "7".parse().unwrap());
        captured(&mut fixture).await.reply.send(upstream).unwrap();
        let response = task.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let output = response_json(response).await;
        assert_eq!(output["usage"]["input_tokens"].as_i64(), Some(tokens));
        assert_eq!(output["usage"]["output_tokens"].as_i64(), Some(tokens));
        let stored: (i64, i64) =
            sqlx::query_as("SELECT input_tokens, output_tokens FROM request_usage WHERE id = ?")
                .bind(&id)
                .fetch_one(&fixture.admin)
                .await
                .unwrap();
        assert_eq!(stored, (tokens, tokens));
        let report = fixture
            .store
            .usage(db::UsageFilter::default())
            .await
            .unwrap();
        assert_eq!(report.summary.input_tokens, Some(tokens));
        assert_eq!(report.summary.output_tokens, Some(tokens));
        assert_eq!(report.summary.token_unknown, 0);
    }
}

#[tokio::test]
async fn inference_completion_failure_keeps_status_and_tokens_pending_atomically() {
    let mut fixture = fixture(Limits::default()).await;
    sqlx::query(
        "CREATE TRIGGER reject_inference_completion BEFORE UPDATE ON request_usage \
         BEGIN SELECT RAISE(ABORT, '합성 추론 완료 기록 실패'); END",
    )
    .execute(&fixture.admin)
    .await
    .unwrap();
    let task = submit(&fixture, &payload());
    captured(&mut fixture)
        .await
        .reply
        .send(Json(answer()).into_response())
        .unwrap();
    assert_error(
        task.await.unwrap(),
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
    )
    .await;
    let pending: (String, Option<i64>, Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT request_kind, status, duration_ms, input_tokens, output_tokens FROM request_usage",
    )
    .fetch_one(&fixture.admin)
    .await
    .unwrap();
    assert_eq!(pending, ("inference".to_owned(), None, None, None, None));
    let report = fixture
        .store
        .usage(db::UsageFilter::default())
        .await
        .unwrap();
    assert_eq!(
        (
            report.summary.total,
            report.summary.pending,
            report.summary.succeeded
        ),
        (1, 1, 0)
    );
    assert_eq!(
        (report.summary.input_tokens, report.summary.output_tokens),
        (None, None)
    );
    assert_eq!(
        (report.summary.token_recorded, report.summary.token_unknown),
        (0, 1)
    );
}
