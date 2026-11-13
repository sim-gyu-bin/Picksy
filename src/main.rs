use std::{io::Write, net::SocketAddr, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use picksy::{Error, db, gateway};
use serde::Serialize;
use tokio::signal::unix::{SignalKind, signal};
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "picksy", version, about = "Gateway 인증·키·사용량 관리")]
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
    Serve {
        #[arg(long, default_value = "127.0.0.1:18081")]
        bind: SocketAddr,
    },
    /// 운영자 전용 키 관리. HTTP 관리 API는 제공하지 않는다.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    /// 전체 또는 한 키의 누적 사용량을 조회한다. pending도 합계에 포함한다.
    Usage {
        #[arg(long)]
        key_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum KeyCommand {
    /// 원문 키는 이번 JSON 출력에서만 확인할 수 있다.
    Create {
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(i64).range(1..))]
        requests_per_minute: i64,
    },
    /// 키의 ID·생성/폐기 시각·분당 한도만 조회한다.
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
        Command::Serve { bind } => serve(store.clone(), bind).await,
        Command::Key { command } => match command {
            KeyCommand::Create {
                requests_per_minute,
            } => print_json(&store.create_key(requests_per_minute).await?),
            KeyCommand::List => print_json(&store.list_keys().await?),
            KeyCommand::Revoke { id } => {
                store.revoke_key(&id).await?;
                print_json(&serde_json::json!({ "id": id, "revoked": true }))
            }
        },
        Command::Usage { key_id } => {
            let summary = store.usage(key_id.as_deref()).await?;
            print_json(&serde_json::json!({ "key_id": key_id, "usage": summary }))
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

async fn serve(store: db::Store, bind: SocketAddr) -> Result<(), Error> {
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
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, gateway::router(store))
        .with_graceful_shutdown(async move {
            // 신규 연결을 중단하고 접수한 요청의 사용량 완료 기록까지 기다린다.
            tokio::select! {
                _ = interrupt.recv() => {},
                _ = terminate.recv() => {},
            }
        })
        .await?;
    Ok(())
}
