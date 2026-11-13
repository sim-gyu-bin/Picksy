use std::{
    io::Write,
    net::SocketAddr,
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

use clap::{Parser, Subcommand, ValueEnum};
use picksy::{
    Error,
    admin::AdminConfig,
    db, gateway,
    inference::{Inference, Limits},
};
use serde::Serialize;
use tokio::signal::unix::{SignalKind, signal};
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(
    name = "picksy",
    version,
    about = "Picksy Server 인증·추론·키·사용량 관리"
)]
struct Cli {
    /// SQLite 파일 경로. 환경변수와 .env는 읽지 않는다.
    #[arg(
        long,
        global = true,
        default_value = ".local/picksy-dev/gateway.sqlite3"
    )]
    db: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// DB를 생성하고 내장 Migration을 명시적으로 실행한다.
    Migrate,
    /// 기존 DB의 스키마를 검증한 뒤 HTTP 서버를 실행한다.
    Serve(ServeArgs),
    /// 운영자 전용 키 관리. HTTP 관리 API는 제공하지 않는다.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    /// 기간·키·종류별 사용량과 UTC 일별 집계를 조회한다. pending도 합계에 포함한다.
    Usage {
        #[arg(long)]
        key_id: Option<String>,
        /// 시작 UTC Unix 초(포함). 생략하면 하한이 없다.
        #[arg(long, value_parser = clap::value_parser!(i64).range(0..))]
        from: Option<i64>,
        /// 종료 UTC Unix 초(미포함). from보다 커야 하며 생략하면 상한이 없다.
        #[arg(long, value_parser = clap::value_parser!(i64).range(0..))]
        until: Option<i64>,
        #[arg(long, value_enum, default_value_t = UsageKind::All)]
        kind: UsageKind,
    },
}

#[derive(Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
enum UsageKind {
    All,
    Inference,
    Models,
    AuthCheck,
    Admin,
    Unknown,
}

impl UsageKind {
    fn request_kind(self) -> Option<db::RequestKind> {
        match self {
            Self::All => None,
            Self::Inference => Some(db::RequestKind::Inference),
            Self::Models => Some(db::RequestKind::Models),
            Self::AuthCheck => Some(db::RequestKind::AuthCheck),
            Self::Admin => Some(db::RequestKind::Admin),
            Self::Unknown => Some(db::RequestKind::Unknown),
        }
    }
}

#[derive(clap::Args)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:18081")]
    bind: SocketAddr,
    /// 관리 화면 산출물 경로. 파일이 없어도 추론 서버는 시작하고 화면만 503을 반환한다.
    #[arg(long, default_value = "admin/dist")]
    admin_assets: PathBuf,
    /// 내부 추론 서버의 http/https origin만 허용한다. 경로·인증 정보는 넣지 않는다.
    #[arg(long)]
    inference_url: String,
    #[arg(long, default_value = "1")]
    inference_concurrency: NonZeroUsize,
    #[arg(long, default_value = "8")]
    inference_queue: NonZeroUsize,
    #[arg(long, default_value = "500")]
    queue_timeout_ms: NonZeroU64,
    #[arg(long, default_value = "8000")]
    inference_timeout_ms: NonZeroU64,
    #[arg(long, default_value = "1000")]
    body_timeout_ms: NonZeroU64,
}

#[derive(Subcommand)]
enum KeyCommand {
    /// 원문 키는 이번 JSON 출력에서만 확인할 수 있다.
    Create {
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(i64).range(1..))]
        requests_per_minute: i64,
        /// 선택 표시 이름. 생략하면 자동 생성한 UUID로 표시한다.
        #[arg(long)]
        name: Option<String>,
    },
    /// 키의 ID·이름·표시 이름·생성/폐기 시각·분당 한도를 조회한다.
    List,
    /// 이후 요청을 즉시 거부한다. 이미 허용한 요청은 완료할 수 있다.
    Revoke { id: String },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // 내부 예외나 CLI 전체를 출력하지 않는다. 오류는 민감정보 없는 고정 분류다.
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), Error> {
    if matches!(cli.command, Command::Migrate) {
        db::migrate(&cli.db).await?;
        return print_json(&serde_json::json!({ "migrated": true }));
    }
    let store = db::Store::open(&cli.db).await?;
    let result = run_existing(cli.command, &store).await;
    store.close().await;
    result
}

async fn run_existing(command: Command, store: &db::Store) -> Result<(), Error> {
    match command {
        Command::Migrate => unreachable!("Migration은 기존 DB 명령과 분리한다"),
        Command::Serve(args) => serve(store.clone(), args).await,
        Command::Key { command } => match command {
            KeyCommand::Create {
                requests_per_minute,
                name,
            } => print_json(
                &store
                    .create_key(requests_per_minute, name.as_deref(), db::KeyActor::Cli)
                    .await?,
            ),
            KeyCommand::List => print_json(&store.list_keys().await?),
            KeyCommand::Revoke { id } => {
                store.revoke_key(&id, db::KeyActor::Cli).await?;
                print_json(&serde_json::json!({ "id": id, "revoked": true }))
            }
        },
        Command::Usage {
            key_id,
            from,
            until,
            kind,
        } => {
            let report = store
                .usage(db::UsageFilter {
                    key_id: key_id.as_deref(),
                    from,
                    until,
                    kind: kind.request_kind(),
                })
                .await?;
            print_json(&serde_json::json!({
                "key_id": key_id,
                "from": from,
                "until": until,
                "kind": kind,
                "usage": report.summary,
                "by_day": report.by_day,
                "by_kind": report.by_kind,
            }))
        }
    }
}

fn print_json(value: &impl Serialize) -> Result<(), Error> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, value).map_err(|_| Error::Io)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

async fn serve(store: db::Store, args: ServeArgs) -> Result<(), Error> {
    let admin = AdminConfig::from_env()?;
    if let Some(config) = &admin {
        config.validate_bind(args.bind)?;
    }
    // 키는 환경변수에서만 읽고 .env 자동 로딩·CLI 인자·원문 오류 출력을 제공하지 않는다.
    let key = std::env::var("PICKSY_INFERENCE_API_KEY").map_err(|_| Error::InferenceConfig)?;
    let inference = Inference::new(
        &args.inference_url,
        &key,
        Limits {
            concurrency: args.inference_concurrency.get(),
            queue: args.inference_queue.get(),
            queue_timeout: Duration::from_millis(args.queue_timeout_ms.get()),
            inference_timeout: Duration::from_millis(args.inference_timeout_ms.get()),
            body_timeout: Duration::from_millis(args.body_timeout_ms.get()),
        },
    )?;
    // RUST_LOG를 받아 민감한 의존성 로그까지 켜는 경로를 만들지 않는다.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .without_time()
                .with_level(false)
                .with_target(false)
                .with_current_span(false)
                .with_span_list(false)
                .with_writer(std::io::stderr)
                .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                    metadata.target() == "picksy::request"
                })),
        )
        .try_init()
        .map_err(|_| Error::Io)?;
    // 신호 등록 실패를 숨기지 않고 서버 시작 전에 실패한다.
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    axum::serve(
        listener,
        gateway::router_with_admin(store, inference, admin)
            .merge(picksy::static_ui::router(args.admin_assets)),
    )
    .with_graceful_shutdown(async move {
        // 신규 연결을 중단하고 정상 처리 중인 요청의 응답·사용량 기록을 기다린다.
        // 취소 후 DB 정리는 런타임 강제 종료·DB 장애까지 완료를 보장하지 않는다.
        tokio::select! {
            _ = interrupt.recv() => {},
            _ = terminate.recv() => {},
        }
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_rejects_invalid_epoch_and_kind_arguments() {
        for argument in ["--from", "--until"] {
            for value in ["-1", "1.5", "9223372036854775808", "not-a-time"] {
                assert!(Cli::try_parse_from(["picksy", "usage", argument, value]).is_err());
            }
        }
        assert!(Cli::try_parse_from(["picksy", "usage", "--kind", "invented"]).is_err());
    }

    #[test]
    fn key_creation_rejects_user_supplied_identifiers_and_secrets() {
        for argument in ["--id", "--api-key"] {
            assert!(
                Cli::try_parse_from(["picksy", "key", "create", argument, "synthetic"]).is_err()
            );
        }
    }
}
