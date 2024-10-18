use std::time::Duration;

use anyhow::Result;
use async_tungstenite::{
    tokio::connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use env_logger::Env;
use futures::prelude::*;
use hookhub::RequestMessage;
use reqwest::{Client, Method};
use tokio::{
    signal::unix::SignalKind,
    sync::broadcast,
    time::{self, interval_at, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use clap::Parser;
use log::{error, info, warn};
use url::Url;

/// Hookhub client - connect to a Hookhub server and receive webhooks
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Remote origin that will relay requests (e.g. wss://something.herokuapp.com)
    #[arg(long, env = "HOOKHUB_REMOTE")]
    remote: String,

    /// Remote server secret used to authenticate
    #[arg(long, env = "HOOKHUB_SECRET")]
    secret: String,

    /// Local origin to relay the requests to (e.g. https://dealers.carwow.local)
    #[arg(long, env = "HOOKHUB_LOCAL")]
    local: String,
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    let remote = validate_and_update_remote(args.remote)?;
    let local = validate_and_update_local(args.local)?;

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
            args.secret.clone(),
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

    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()?;

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
                        forward_request(rmp_serde::from_slice(&msg)?, local.clone(), http.clone());
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

fn validate_and_update_remote(remote: String) -> Result<Url> {
    let mut url = Url::parse(&remote)?;

    if url.scheme() != "ws" && url.scheme() != "wss" {
        return Err(anyhow::anyhow!("remote must use ws or wss scheme"));
    }

    url.set_path("/__hookhub__/");

    Ok(url)
}

fn validate_and_update_local(local: String) -> Result<Url> {
    let mut url = Url::parse(&local)?;

    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(anyhow::anyhow!("local must use http or https scheme"));
    }

    url.set_path("/");

    Ok(url)
}

fn forward_request(req: RequestMessage, mut local: Url, http: Client) {
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
    });
}
