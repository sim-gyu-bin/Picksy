//! 인증·공유 제한·사용량과 고정 d1 추론의 외부 경계를 제공한다.

pub mod admin;
pub mod db;
pub mod gateway;
pub mod inference;
pub mod static_ui;

use std::fmt;

/// 외부로 전달해도 경로·키·SQL·요청 본문이 노출되지 않는 오류 분류다.
#[derive(Debug)]
pub enum Error {
    Database,
    Migration,
    Schema,
    InvalidLimit,
    InvalidKeyName,
    InvalidUsageFilter,
    InvalidPath,
    KeyNotFound,
    Random,
    Io,
    InferenceConfig,
    AdminConfig,
    AdminUnauthorized,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Database => "DB 작업을 완료하지 못했습니다.",
            Self::Migration => "Migration을 완료하지 못했습니다.",
            Self::Schema => "DB 스키마가 일치하지 않습니다. 명시적 migrate 실행 여부를 확인하세요.",
            Self::InvalidLimit => "분당 요청 제한은 1 이상의 정수여야 합니다.",
            Self::InvalidKeyName => "키 이름은 제어문자 없이 128자 이하여야 합니다.",
            Self::InvalidUsageFilter => {
                "조회 기간은 0 이상의 UTC 초이며 시작은 끝보다 작아야 합니다."
            }
            Self::InvalidPath => "영구 DB 파일 경로를 지정해야 합니다.",
            Self::KeyNotFound => "해당 키 ID가 없습니다.",
            Self::Random => "안전한 난수를 생성하지 못했습니다.",
            Self::Io => "입출력 작업을 완료하지 못했습니다.",
            Self::InferenceConfig => "추론 origin·인증 환경변수·양수 상한 설정을 확인하세요.",
            Self::AdminConfig => "관리자 Google 인증 환경변수·콜백·바인딩 설정을 확인하세요.",
            Self::AdminUnauthorized => "유효한 관리자 세션이 필요합니다.",
        })
    }
}

impl std::error::Error for Error {}

impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self::Database
    }
}

impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}
