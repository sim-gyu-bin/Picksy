//! 공개 화면 shell만 제공한다. 관리 데이터와 인증은 기존 관리 API의 책임이다.

use std::{
    ffi::CStr,
    fs::{File, OpenOptions},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};

// Radix 대화상자의 스크롤 잠금은 동적 style 태그·속성을 쓴다. 스크립트의 inline/eval은 금지한다.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self'; font-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; object-src 'none'";

/// 지정한 세 파일만 공개한다. 미빌드 상태도 서버 시작을 막지 않으며 해당 요청만 503이다.
/// 요청 경로를 파일 경로로 변환하지 않고 SPA 포괄 fallback도 등록하지 않는다.
pub fn router(root: PathBuf) -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::to("/console") }))
        .route("/console", get(index))
        .route("/console/keys", get(index))
        .route("/console/assets/app.js", get(javascript))
        .route("/console/assets/app.css", get(stylesheet))
        .with_state(Arc::new(root))
        .layer(middleware::from_fn(security_headers))
}

async fn index(State(root): State<Arc<PathBuf>>) -> Response {
    serve(root, None, c"index.html", "text/html; charset=utf-8").await
}

async fn javascript(State(root): State<Arc<PathBuf>>) -> Response {
    serve(
        root,
        Some(c"assets"),
        c"app.js",
        "text/javascript; charset=utf-8",
    )
    .await
}

async fn stylesheet(State(root): State<Arc<PathBuf>>) -> Response {
    serve(root, Some(c"assets"), c"app.css", "text/css; charset=utf-8").await
}

async fn serve(
    root: Arc<PathBuf>,
    directory: Option<&'static CStr>,
    name: &'static CStr,
    content_type: &'static str,
) -> Response {
    // watch 빌드 결과를 재시작 없이 반영한다. 디스크 I/O는 런타임 작업 스레드를 막지 않는다.
    match tokio::task::spawn_blocking(move || read_asset(&root, directory, name)).await {
        Ok(Ok(body)) => ([(header::CONTENT_TYPE, content_type)], body).into_response(),
        // 경로·OS 오류를 외부에 노출하거나 잘못된 asset 요청을 index로 대신하지 않는다.
        _ => crate::gateway::api_error(StatusCode::SERVICE_UNAVAILABLE, "ui_unavailable"),
    }
}

fn read_asset(root: &Path, directory: Option<&CStr>, name: &CStr) -> io::Result<Vec<u8>> {
    let root = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(root)?;
    let parent = match directory {
        Some(directory) => open_child(&root, directory, libc::O_DIRECTORY)?,
        None => root,
    };
    let mut file = open_child(&parent, name, libc::O_NONBLOCK)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    let capacity =
        usize::try_from(metadata.len()).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    let mut body = Vec::with_capacity(capacity);
    file.read_to_end(&mut body)?;
    Ok(body)
}

fn open_child(parent: &File, name: &CStr, flags: i32) -> io::Result<File> {
    // 각 구성요소를 열린 디렉터리 기준으로 연다. canonicalize 뒤 read하는 TOCTOU와
    // assets 디렉터리·최종 파일의 외부 symlink를 함께 차단한다. name은 위의 고정 단일 이름뿐이다.
    // SAFETY: parent는 호출 중 유효한 fd이고 name은 NUL 종결 CStr이다. 생성 플래그는 쓰지 않는다.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | flags,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat 성공으로 새로 소유한 fd를 File 한 곳에만 넘긴다.
    Ok(unsafe { File::from_raw_fd(fd) })
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    // 고정 파일명으로 개발 빌드를 갱신하므로 HTML과 asset 모두 브라우저 캐시를 남기지 않는다.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}
