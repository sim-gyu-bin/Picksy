use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    body::to_bytes,
    extract::{Request, State},
    http::{
        HeaderValue, StatusCode,
        header::{CONTENT_TYPE, RETRY_AFTER},
    },
    response::{IntoResponse, Response},
};
use reqwest::{Client, Url};
use serde_json::{Map, Value, json};
use tokio::{sync::Semaphore, time::timeout};

use crate::{Error, db::TokenUsage, gateway::api_error};

pub const MODEL_ID: &str = "d1-2b-q4";
const INTERNAL_MODEL: &str = "/models/d1-2B-Q4_K_M.gguf";
const BODY_LIMIT: usize = 64 * 1024;
const RESPONSE_LIMIT: usize = 256 * 1024;

/// 한 프로세스의 추론·대기 한도다. 복제본 전체 한도가 아니다.
#[derive(Clone, Copy)]
pub struct Limits {
    pub concurrency: usize,
    pub queue: usize,
    pub queue_timeout: Duration,
    pub inference_timeout: Duration,
    pub body_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            concurrency: 1,
            queue: 8,
            queue_timeout: Duration::from_millis(500),
            inference_timeout: Duration::from_secs(8),
            body_timeout: Duration::from_secs(1),
        }
    }
}

#[derive(Clone)]
pub struct Inference {
    client: Client,
    endpoint: Url,
    health: Url,
    authorization: HeaderValue,
    active: Arc<Semaphore>,
    admitted: Arc<Semaphore>,
    limits: Limits,
}

impl Inference {
    /// origin과 내부 전용 키를 받는다. CLI는 키를 환경변수에서만 읽는다.
    pub fn new(origin: &str, key: &str, limits: Limits) -> Result<Self, Error> {
        let url = Url::parse(origin).map_err(|_| Error::InferenceConfig)?;
        let authority = origin.split_once("://").ok_or(Error::InferenceConfig)?.1;
        let suffix = authority.split_once('/').map(|(_, path)| path);
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || authority.contains('@')
            || origin.contains('\\')
            || suffix.is_some_and(|path| !path.is_empty())
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || key.is_empty()
            || key.trim() != key
            || limits.concurrency == 0
            || limits.queue == 0
            || limits.queue_timeout.is_zero()
            || limits.inference_timeout.is_zero()
            || limits.body_timeout.is_zero()
        {
            return Err(Error::InferenceConfig);
        }
        let total = limits
            .concurrency
            .checked_add(limits.queue)
            .filter(|total| *total <= Semaphore::MAX_PERMITS)
            .ok_or(Error::InferenceConfig)?;
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| Error::InferenceConfig)?;
        authorization.set_sensitive(true);
        // 환경 프록시·리다이렉트·자동 재시도는 내부 키의 경계와 단일 실행 의미를 깨뜨린다.
        let client = Client::builder()
            .use_rustls_tls()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| Error::InferenceConfig)?;
        Ok(Self {
            client,
            endpoint: url
                .join("v1/systemone")
                .map_err(|_| Error::InferenceConfig)?,
            health: url.join("health").map_err(|_| Error::InferenceConfig)?,
            authorization,
            active: Arc::new(Semaphore::new(limits.concurrency)),
            admitted: Arc::new(Semaphore::new(total)),
            limits,
        })
    }

    pub(crate) async fn ready(&self) -> bool {
        // 고정 llama.cpp의 /health는 인증 없이 준비 상태를 제공한다. 본문은 외부에 내보내지 않는다.
        matches!(timeout(Duration::from_millis(500), self.client.get(self.health.clone()).send()).await,
            Ok(Ok(response)) if response.status() == StatusCode::OK)
    }

    async fn infer(&self, request: &Value) -> Result<(Value, TokenUsage), ()> {
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .header(reqwest::header::AUTHORIZATION, self.authorization.clone())
            .json(request)
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success() {
            return Err(());
        }
        let mut bytes = Vec::new();
        // Content-Length는 신뢰하지 않는다. 청크 응답도 실제 읽은 바이트로 제한한다.
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if chunk.len() > RESPONSE_LIMIT.saturating_sub(bytes.len()) {
                return Err(());
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut result: Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
        let tokens = validate_response(&mut result, &request["questions"])?;
        Ok((result, tokens))
    }
}

pub(crate) async fn models() -> Json<Value> {
    Json(json!({"models": [{
        "name": MODEL_ID,
        "description": "d1 Q4: score는 누적확률 0.5의 첫 중앙값(median) 등급이며 confidence는 최대 확률(max-probability)입니다. 기대값·정답 확률로 보정하지 않습니다.",
        "release_date": ""
    }]}))
}

pub(crate) async fn systemone(State(inference): State<Inference>, request: Request) -> Response {
    // 본문을 읽기 전 즉시 접수 한도를 확보한다. 대기·느린 본문도 무한 축적하지 않는다.
    let Ok(_admission) = inference.admitted.try_acquire() else {
        return overloaded();
    };
    let mut types = request.headers().get_all(CONTENT_TYPE).iter();
    let valid_type = types
        .next()
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
        });
    if !valid_type || types.next().is_some() {
        return api_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type");
    }
    let bytes = match timeout(
        inference.limits.body_timeout,
        to_bytes(request.into_body(), BODY_LIMIT),
    )
    .await
    {
        Err(_) => return api_error(StatusCode::GATEWAY_TIMEOUT, "inference_timeout"),
        Ok(Err(error)) => {
            use std::error::Error as _;
            let oversized = error
                .source()
                .is_some_and(|source| source.is::<http_body_util::LengthLimitError>());
            return if oversized {
                api_error(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large")
            } else {
                api_error(StatusCode::BAD_REQUEST, "invalid_request")
            };
        }
        Ok(Ok(bytes)) => bytes,
    };
    let mut payload: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return api_error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if let Err(error) = validate_request(&payload) {
        return api_error(StatusCode::BAD_REQUEST, error);
    }
    // 단일 모델 서버의 실제 실험 계약은 state+questions다. 공개 라우팅 ID를 내부에 보내지 않는다.
    payload
        .as_object_mut()
        .expect("검증된 객체")
        .shift_remove("model");
    let _active = match timeout(inference.limits.queue_timeout, inference.active.acquire()).await {
        Ok(Ok(permit)) => permit,
        _ => return overloaded(),
    };
    // future를 별도 작업으로 분리하지 않아 서비스 future drop 시 HTTP 요청과 permit도 함께 해제한다.
    match timeout(
        inference.limits.inference_timeout,
        inference.infer(&payload),
    )
    .await
    {
        Err(_) => api_error(StatusCode::GATEWAY_TIMEOUT, "inference_timeout"),
        Ok(Err(())) => api_error(StatusCode::BAD_GATEWAY, "inference_unavailable"),
        Ok(Ok((body, tokens))) => {
            let mut response = Json(body).into_response();
            response.extensions_mut().insert(tokens);
            response
        }
    }
}

fn overloaded() -> Response {
    let mut response = api_error(StatusCode::SERVICE_UNAVAILABLE, "overloaded");
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

fn valid_text(value: &Value, maximum: usize) -> bool {
    value
        .as_str()
        .is_some_and(|text| !text.is_empty() && text.len() <= maximum)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128
}

fn only_fields(object: &Map<String, Value>, fields: &[&str]) -> bool {
    object.keys().all(|key| fields.contains(&key.as_str()))
}

fn validate_request(request: &Value) -> Result<(), &'static str> {
    let invalid = "invalid_request";
    let object = request.as_object().ok_or(invalid)?;
    if !only_fields(object, &["model", "state", "questions"]) {
        return Err(invalid);
    }
    let model = object.get("model").and_then(Value::as_str).ok_or(invalid)?;
    if model != MODEL_ID {
        return Err("unsupported_model");
    }
    // 빈 문자열도 의도적인 문맥 없는 판단 입력이므로 허용한다. JSON 내부 값은 그대로 보존한다.
    if !matches!(
        object.get("state"),
        Some(Value::String(_) | Value::Object(_) | Value::Array(_))
    ) {
        return Err(invalid);
    }
    let questions = object
        .get("questions")
        .and_then(Value::as_object)
        .ok_or(invalid)?;
    if !(1..=8).contains(&questions.len()) {
        return Err(invalid);
    }
    for (id, question) in questions {
        let q = question.as_object().ok_or(invalid)?;
        if !valid_id(id)
            || !only_fields(q, &["type", "instructions", "criteria"])
            || !q
                .get("instructions")
                .is_some_and(|value| valid_text(value, 4096))
        {
            return Err(invalid);
        }
        match q.get("type").and_then(Value::as_str) {
            Some("choice") => {
                let criteria = q
                    .get("criteria")
                    .and_then(Value::as_object)
                    .ok_or(invalid)?;
                if !(2..=16).contains(&criteria.len())
                    || !criteria
                        .iter()
                        .all(|(id, label)| valid_id(id) && (label.is_null() || label.is_string()))
                {
                    return Err(invalid);
                }
            }
            Some("score") => {
                let criteria = q.get("criteria").and_then(Value::as_array).ok_or(invalid)?;
                // 고정 llama.cpp의 d1 score parser는 2..10 등급만 받는다. choice의 상한과 다르다.
                if !(2..=10).contains(&criteria.len()) || !criteria.iter().all(Value::is_string) {
                    return Err(invalid);
                }
            }
            Some("noul") => {
                if let Some(criteria) = q.get("criteria") {
                    let criteria = criteria.as_object().ok_or(invalid)?;
                    if !only_fields(criteria, &["true", "false"])
                        || !criteria.values().all(Value::is_string)
                    {
                        return Err(invalid);
                    }
                }
            }
            _ => return Err(invalid),
        }
    }
    Ok(())
}

fn probability(value: &Value) -> Result<f64, ()> {
    value
        .as_f64()
        .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
        .ok_or(())
}

fn validate_response(response: &mut Value, questions: &Value) -> Result<TokenUsage, ()> {
    let object = response.as_object_mut().ok_or(())?;
    if object.get("model").and_then(Value::as_str) != Some(INTERNAL_MODEL) {
        return Err(());
    }
    let usage = object
        .get_mut("usage")
        .and_then(Value::as_object_mut)
        .ok_or(())?;
    // SQLite 정수 범위를 넘는 수치는 반올림·포화하지 않고 응답 전체를 거절한다.
    let token_count = |field: &str| {
        usage
            .get(field)
            .and_then(Value::as_i64)
            .filter(|count| *count >= 0)
            .ok_or(())
    };
    let tokens = TokenUsage {
        input_tokens: token_count("input_tokens")?,
        output_tokens: token_count("output_tokens")?,
    };
    validate_cost(usage.get("cost"))?;
    usage.retain(|key, _| ["input_tokens", "output_tokens", "cost"].contains(&key.as_str()));
    validate_cost(object.get("cost"))?;
    let answers = object
        .get_mut("answers")
        .and_then(Value::as_object_mut)
        .ok_or(())?;
    let questions = questions.as_object().ok_or(())?;
    if answers.len() != questions.len() {
        return Err(());
    }
    for (id, question) in questions {
        let answer = answers
            .get_mut(id)
            .and_then(Value::as_object_mut)
            .ok_or(())?;
        let kind = question["type"].as_str().ok_or(())?;
        if answer.get("type").and_then(Value::as_str) != Some(kind) {
            return Err(());
        }
        if kind == "noul" {
            probability(answer.get("noul").ok_or(())?)?;
            answer.retain(|key, _| ["type", "noul"].contains(&key.as_str()));
            continue;
        }
        let probabilities = answer
            .get("probabilities")
            .and_then(Value::as_object)
            .ok_or(())?;
        let mut sum = 0.0;
        let mut maximum = 0.0_f64;
        for value in probabilities.values() {
            let p = probability(value)?;
            sum += p;
            maximum = maximum.max(p);
        }
        if (sum - 1.0).abs() > 0.001
            || (probability(answer.get("confidence").ok_or(())?)? - maximum).abs() > 0.000001
        {
            return Err(());
        }
        if kind == "choice" {
            let criteria = question["criteria"].as_object().ok_or(())?;
            if probabilities.len() != criteria.len()
                || !criteria.keys().all(|key| probabilities.contains_key(key))
            {
                return Err(());
            }
            let choice = answer.get("choice").and_then(Value::as_str).ok_or(())?;
            if probability(probabilities.get(choice).ok_or(())?)? != maximum {
                return Err(());
            }
            answer.retain(|key, _| {
                ["type", "choice", "probabilities", "confidence"].contains(&key.as_str())
            });
        } else {
            let criteria = question["criteria"].as_array().ok_or(())?;
            let legend = answer.get("legend").and_then(Value::as_object).ok_or(())?;
            if probabilities.len() != criteria.len() || legend.len() != criteria.len() {
                return Err(());
            }
            let mut mass = 0.0;
            let mut median = criteria.len() - 1;
            for (index, label) in criteria.iter().enumerate() {
                let key = index.to_string();
                if legend.get(&key) != Some(label) {
                    return Err(());
                }
                let p = probability(probabilities.get(&key).ok_or(())?)?;
                if mass < 0.5 {
                    median = index;
                }
                mass += p;
            }
            if answer.get("score").and_then(Value::as_f64) != Some(median as f64) {
                return Err(());
            }
            answer.retain(|key, _| {
                ["type", "score", "legend", "probabilities", "confidence"].contains(&key.as_str())
            });
        }
    }
    // 알려지지 않은 서버 라우팅·진단 필드는 중첩 구조에서도 제거한다. 실제 수치는 바꾸지 않는다.
    object.retain(|key, _| ["model", "answers", "usage", "cost"].contains(&key.as_str()));
    object.insert("model".to_owned(), Value::String(MODEL_ID.to_owned()));
    Ok(tokens)
}

fn validate_cost(cost: Option<&Value>) -> Result<(), ()> {
    if let Some(cost) = cost {
        cost.as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .ok_or(())?;
    }
    Ok(())
}
