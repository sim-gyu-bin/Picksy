use std::{fs, os::unix::fs::symlink, path::PathBuf};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    response::Response,
};
use picksy::{
    admin::AdminConfig,
    db::{self, Store},
    gateway,
    inference::{Inference, Limits},
    static_ui,
};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

const HTML: &str = "<!doctype html><html><head><link rel=\"stylesheet\" href=\"/console/assets/app.css\"></head><body><div id=\"root\"></div><script type=\"module\" src=\"/console/assets/app.js\"></script></body></html>";
const JAVASCRIPT: &str = "document.title = 'Picksy';";
const CSS: &str = "body { color: black; }";

fn directory() -> PathBuf {
    // 기존 개발 DB·자산을 읽거나 삭제하지 않는 테스트별 격리 디렉터리다.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".local/picksy-dev/static-ui-tests")
        .join(Uuid::new_v4().to_string());
    fs::create_dir_all(&path).unwrap();
    path
}

fn assets(root: &std::path::Path) {
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("index.html"), HTML).unwrap();
    fs::write(root.join("assets/app.js"), JAVASCRIPT).unwrap();
    fs::write(root.join("assets/app.css"), CSS).unwrap();
}

fn fixture() -> (Router, PathBuf) {
    let root = directory().join("dist");
    assets(&root);
    (static_ui::router(root.clone()), root)
}

async fn request(app: &Router, method: &str, uri: &str) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn body(response: Response) -> Vec<u8> {
    to_bytes(response.into_body(), 1_048_576)
        .await
        .unwrap()
        .to_vec()
}

async fn json_error(response: Response, status: StatusCode, error: &str) {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let value: serde_json::Value = serde_json::from_slice(&body(response).await).unwrap();
    assert_eq!(value, json!({ "error": error }));
}

#[tokio::test]
async fn explicit_pages_and_assets_have_real_mime_and_security_headers() {
    let (app, _) = fixture();
    for (uri, mime, expected) in [
        ("/console", "text/html; charset=utf-8", HTML),
        ("/console/keys", "text/html; charset=utf-8", HTML),
        (
            "/console/assets/app.js",
            "text/javascript; charset=utf-8",
            JAVASCRIPT,
        ),
        ("/console/assets/app.css", "text/css; charset=utf-8", CSS),
    ] {
        let response = request(&app, "GET", uri).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
        let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap();
        for directive in [
            "default-src 'none'",
            "script-src 'self'",
            "style-src 'self' 'unsafe-inline'",
            "connect-src 'self'",
            "object-src 'none'",
            "base-uri 'none'",
            "frame-ancestors 'none'",
        ] {
            assert!(csp.split("; ").any(|value| value == directive));
        }
        assert!(!csp.contains("unsafe-eval"));
        assert_eq!(body(response).await, expected.as_bytes());
        let response = request(&app, "HEAD", uri).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
        assert!(body(response).await.is_empty());
    }
}

#[tokio::test]
async fn root_redirect_is_fixed_and_does_not_forward_parameters() {
    let (app, _) = fixture();
    let response = request(
        &app,
        "GET",
        "/?redirect=https://example.invalid&code=synthetic",
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/console");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(body(response).await.is_empty());
}

#[tokio::test]
async fn traversal_percent_encoding_and_unknown_paths_never_serve_files() {
    let (app, root) = fixture();
    fs::write(root.join("private.json"), "synthetic-private-file").unwrap();
    fs::write(root.join("assets/app.js.map"), "synthetic-source-map").unwrap();
    for uri in [
        "/console/",
        "/console/unknown",
        "/console/index.html",
        "/console/assets",
        "/console/assets/",
        "/console/assets/app.js.map",
        "/console/assets/private.json",
        "/console/assets/../private.json",
        "/console/assets/../../Cargo.toml",
        "/console/assets/%2e%2e/private.json",
        "/console/assets/%2E%2E%2Fprivate.json",
        "/console/assets/%252e%252e%252fprivate.json",
        "/console/assets/app%2ejs",
        "/console/assets/app.js%00",
        "/console/assets/%2fetc%2fpasswd",
        "/console/%6beys",
        "/%63onsole",
        "/console%2fkeys",
        "/console/assets/app.js/extra",
        "/console/assets//app.js",
        "/admin/missing",
        "/v1/missing",
    ] {
        let response = request(&app, "GET", uri).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        assert!(body(response).await.is_empty(), "{uri}");
    }
    // Query로 파일을 고르지 않으며 허용된 파일만 그대로 반환한다.
    let response = request(
        &app,
        "GET",
        "/console/assets/app.js?path=../../private.json",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(response).await, JAVASCRIPT.as_bytes());
}

#[tokio::test]
async fn static_routes_reject_mutation_methods_without_changing_files() {
    let (app, root) = fixture();
    for uri in ["/console", "/console/keys", "/console/assets/app.js"] {
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            assert_eq!(
                request(&app, method, uri).await.status(),
                StatusCode::METHOD_NOT_ALLOWED
            );
        }
    }
    assert_eq!(fs::read(root.join("index.html")).unwrap(), HTML.as_bytes());
    assert_eq!(
        fs::read(root.join("assets/app.js")).unwrap(),
        JAVASCRIPT.as_bytes()
    );
}

#[tokio::test]
async fn absent_assets_recover_after_build_and_watch_replacements_are_visible() {
    let root = directory().join("not-built-yet");
    let app = static_ui::router(root.clone());
    for uri in ["/console", "/console/keys", "/console/assets/app.js"] {
        json_error(
            request(&app, "GET", uri).await,
            StatusCode::SERVICE_UNAVAILABLE,
            "ui_unavailable",
        )
        .await;
    }
    assets(&root);
    assert_eq!(
        body(request(&app, "GET", "/console").await).await,
        HTML.as_bytes()
    );
    fs::write(root.join("assets/next.js"), "document.title = '갱신';").unwrap();
    fs::rename(root.join("assets/next.js"), root.join("assets/app.js")).unwrap();
    let response = request(&app, "GET", "/console/assets/app.js").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(body(response).await, "document.title = '갱신';".as_bytes());
}

#[tokio::test]
async fn symlink_roots_directories_and_files_cannot_expose_external_contents() {
    let base = directory();
    let outside = base.join("outside");
    assets(&outside);

    let root_link = base.join("root-link");
    symlink(&outside, &root_link).unwrap();
    let app = static_ui::router(root_link);
    json_error(
        request(&app, "GET", "/console").await,
        StatusCode::SERVICE_UNAVAILABLE,
        "ui_unavailable",
    )
    .await;

    let root = base.join("directory-link");
    fs::create_dir(&root).unwrap();
    symlink(outside.join("assets"), root.join("assets")).unwrap();
    let app = static_ui::router(root);
    json_error(
        request(&app, "GET", "/console/assets/app.js").await,
        StatusCode::SERVICE_UNAVAILABLE,
        "ui_unavailable",
    )
    .await;

    let root = base.join("file-link");
    fs::create_dir_all(root.join("assets")).unwrap();
    for name in ["index.html", "assets/app.js", "assets/app.css"] {
        symlink(outside.join(name), root.join(name)).unwrap();
    }
    let app = static_ui::router(root);
    for uri in [
        "/console",
        "/console/assets/app.js",
        "/console/assets/app.css",
    ] {
        json_error(
            request(&app, "GET", uri).await,
            StatusCode::SERVICE_UNAVAILABLE,
            "ui_unavailable",
        )
        .await;
    }
}

#[tokio::test]
async fn directories_are_not_served_as_assets() {
    let root = directory();
    fs::create_dir(root.join("index.html")).unwrap();
    let app = static_ui::router(root);
    json_error(
        request(&app, "GET", "/console").await,
        StatusCode::SERVICE_UNAVAILABLE,
        "ui_unavailable",
    )
    .await;
}

#[tokio::test]
async fn shell_does_not_replace_api_authentication_errors_or_readiness() {
    let base = directory();
    let path = base.join("gateway.sqlite3");
    db::migrate(&path).await.unwrap();
    let store = Store::open(&path).await.unwrap();
    let root = base.join("dist");
    assets(&root);
    for enabled in [false, true] {
        let config = enabled.then(|| {
            AdminConfig::new(
                "admin@example.invalid",
                "synthetic-client",
                "synthetic-secret",
                "https://console.example.invalid/auth/google/callback",
            )
            .unwrap()
        });
        let app = gateway::router_with_admin(
            store.clone(),
            Inference::new(
                "http://127.0.0.1:9",
                "synthetic-upstream-key",
                Limits::default(),
            )
            .unwrap(),
            config,
        )
        .merge(static_ui::router(root.clone()));
        assert_eq!(
            request(&app, "GET", "/console").await.status(),
            StatusCode::OK
        );
        assert_eq!(
            request(&app, "GET", "/health/live").await.status(),
            StatusCode::OK
        );
        for (method, uri) in [
            ("GET", "/admin/session"),
            ("GET", "/admin/keys"),
            ("GET", "/admin/usage"),
            ("POST", "/admin/keys"),
            ("POST", "/admin/logout"),
            ("GET", "/auth/google/callback?state=synthetic"),
        ] {
            json_error(
                request(&app, method, uri).await,
                if enabled {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                },
                if enabled {
                    "unauthorized"
                } else {
                    "unavailable"
                },
            )
            .await;
        }
        json_error(
            request(&app, "GET", "/v1/models").await,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
        json_error(
            request(&app, "GET", "/health/ready").await,
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
        )
        .await;
        for uri in ["/admin/not-found", "/v1/not-found"] {
            let response = request(&app, "GET", uri).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert!(body(response).await.is_empty());
        }
    }
}
