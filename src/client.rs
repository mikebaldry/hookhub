use std::{fs, io, path::PathBuf, sync::LazyLock, time::Duration};

use anyhow::Result;
use async_tungstenite::{
    tokio::connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use chrono::Utc;
use env_logger::Env;
use futures::prelude::*;
use history_db::ItemId;
use hookhub::RequestMessage;
use reqwest::{Client, Method};
use tokio::{
    signal::unix::SignalKind,
    sync::broadcast,
    task::JoinHandle,
    time::{self, interval_at, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use clap::{Parser, Subcommand};
use log::{error, info, warn};
use url::Url;

mod history;
mod history_db;

pub static ROOT_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let home = homedir::my_home().unwrap().unwrap();

    match fs::create_dir(&home) {
        Ok(_) => home.join(".hookhub"),
        Err(e) => {
            if e.kind() == io::ErrorKind::AlreadyExists {
                home.join(".hookhub")
            } else {
                panic!("{}", e);
            }
        }
    }
});

pub static HISTORY_DB: LazyLock<history_db::Db> =
    LazyLock::new(|| history_db::Db::new(&ROOT_PATH.join("history")).unwrap());

/// Hookhub client
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Connect to a remote server and relay requests to a local server
    Connect {
        /// Remote origin that will relay requests (e.g. wss://something.herokuapp.com)
        #[arg(long, env = "HOOKHUB_REMOTE")]
        remote: Url,

        /// Remote server secret used to authenticate
        #[arg(long, env = "HOOKHUB_SECRET")]
        secret: String,

        /// Local origin to relay requests to (e.g. https://localhost:3000/)
        #[arg(long, env = "HOOKHUB_LOCAL")]
        local: Url,
    },
    /// Manage and replay previously received requests
    History {
        #[command(subcommand)]
        command: HistoryCommands,
    },
}

#[derive(Subcommand)]
enum HistoryCommands {
    /// List previously received requests
    List,
    /// Delete a previously received request
    Delete {
        /// Identifier of the request
        id: ItemId,
    },
    /// Clear all previously received requests
    Clear,
    /// Replay a previously received request
    Replay {
        /// Identifier of the request
        id: ItemId,
        /// Local origin to relay requests to (e.g. https://localhost:3000/)
        #[arg(long, env = "HOOKHUB_LOCAL")]
        local: Url,
    },
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    match args.command {
        Commands::Connect {
            remote,
            secret,
            local,
        } => handle_connect(remote, secret, local).await,
        Commands::History { command } => history::handle(command).await,
    }
}

async fn handle_connect(mut remote: Url, secret: String, mut local: Url) -> Result<()> {
    prepare_remote_url(&mut remote)?;
    prepare_local_url(&mut local)?;

    info!("Local origin: {}", local);
    info!("Remote origin: {}", remote);

    let (shutdown, _) = broadcast::channel::<()>(1);

    let shutdown_tx = shutdown.clone();

    tokio::spawn(async move {
        let mut sigint = std::pin::pin!(interrupt_signal());
        tokio::select! {
            _ = sigint.as_mut() => {
                let _ = shutdown_tx.send(());
                warn!("SIGINT received, shutting down");
            }
        }
    });

    loop {
        let result = connect_and_run(
            local.clone(),
            remote.clone(),
            secret.clone(),
            shutdown.clone(),
        )
        .await;
        if let Err(e) = result {
            error!("Failed with error: {:?}", e);
            error!("Trying again in 5 seconds...");

            let mut shutdown = shutdown.clone().subscribe();

            tokio::select! {
                _ = time::sleep(Duration::from_secs(5)) => {
                },
                _ = shutdown.recv() => {
                    break;
                }
            }
        } else {
            break;
        }
    }

    Ok(())
}

async fn connect_and_run(
    local: Url,
    remote: Url,
    secret: String,
    shutdown: broadcast::Sender<()>,
) -> Result<()> {
    let mut request = remote.as_str().into_client_request()?;
    let auth = STANDARD.encode(format!("{}:{}", VERSION, secret));
    request
        .headers_mut()
        .insert("Authorization", format!("Basic {}", auth).parse()?);

    let http = http_client()?;

    let (mut stream, _) = connect_async(request).await?;

    info!("Connected successfully, waiting for events");

    let start = Instant::now() + Duration::from_secs(20);
    let mut interval = interval_at(start, Duration::from_secs(20));

    let mut shutdown = shutdown.subscribe();

    loop {
        tokio::select! {
            Some(message) = stream.next()  => {
                match message? {
                    Message::Binary(msg) => {
                        let req : RequestMessage = rmp_serde::from_slice(&msg)?;
                        HISTORY_DB.add(&history_db::Item::new(Utc::now(), req.clone())).await.unwrap();
                        forward_request(req, local.clone(), http.clone());
                    },
                    Message::Close(_) => {
                        info!("Server closed the connection");
                        break;
                    },
                    _ => { }
                }
            },
            _ = interval.tick() => {
                stream.send(Message::Ping(vec![5, 4, 3, 2, 1])).await?;
            },
            _ = shutdown.recv() => {
                break;
            }
        }
    }

    info!("Disconnected");
    let _ = stream.close(None).await;

    Ok(())
}

async fn interrupt_signal() {
    tokio::signal::unix::signal(SignalKind::interrupt())
        .expect("failed to install SIGINT handler")
        .recv()
        .await;
}

pub fn prepare_remote_url(remote: &mut Url) -> Result<()> {
    if remote.scheme() != "ws" && remote.scheme() != "wss" {
        return Err(anyhow::anyhow!("remote must use ws or wss scheme"));
    }

    if remote.path() != "/" {
        warn!("Remote path isn't supported and will always be /__hookhub__/");
        remote.set_path("/__hookhub__/");
    }

    Ok(())
}

pub fn prepare_local_url(local: &mut Url) -> Result<()> {
    if local.scheme() != "http" && local.scheme() != "https" {
        return Err(anyhow::anyhow!("local must use http or https scheme"));
    }

    if local.path() != "/" {
        warn!("Local path isn't supported and will be ignored");
        local.set_path("/");
    }

    Ok(())
}

fn forward_request(req: RequestMessage, mut local: Url, http: Client) -> JoinHandle<()> {
    tokio::spawn(async move {
        local.set_path(&req.fullpath);

        let start = Instant::now();

        let mut request_builder = http
            .request(Method::from_bytes(req.method.as_bytes()).unwrap(), local)
            .version(req.version.into());

        for (name, value) in req.headers.iter() {
            request_builder = request_builder.header(name, value);
        }

        if !req.body.is_empty() {
            request_builder = request_builder.body(req.body)
        }

        let request = request_builder.build().unwrap();

        match http.execute(request).await {
            Ok(resp) => {
                info!(
                    "Forwarded request: {} {} - {:?} {:?}",
                    req.method,
                    req.fullpath,
                    resp.status(),
                    start.elapsed(),
                );
            }
            Err(e) => {
                error!("Forwarded request error: {}", e);
            }
        }

        
    })
}

pub fn http_client() -> Result<reqwest::Client> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()?;

    Ok(client)
}
