use crate::{
    history::{self, History},
    profiles::{Profile, Profiles},
    VERSION,
};

use std::time::Duration;

use anyhow::Result;
use async_tungstenite::{
    tokio::connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use chrono::Utc;
use futures::prelude::*;
use hookhub::RequestMessage;
use reqwest::{Client, Method};
use tokio::{
    signal::unix::SignalKind,
    sync::broadcast,
    task::JoinHandle,
    time::{self, interval_at, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use log::{error, info, warn};
use url::Url;

pub async fn handle_connect(profile: String) -> Result<()> {
    let profile = Profiles::new()?
        .get(&profile)
        .expect("Profile doesn't exist")
        .prepare()?;

    info!("Local origin: {}", profile.local);
    info!("Remote origin: {}", profile.remote);

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
        let result = connect_and_run(profile.clone(), shutdown.clone()).await;
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

async fn connect_and_run(profile: Profile, shutdown: broadcast::Sender<()>) -> Result<()> {
    let mut request = profile.remote.as_str().into_client_request()?;
    let auth = STANDARD.encode(format!("{}:{}", VERSION, profile.secret));
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
                History::new()?.add(&history::Item::new(Utc::now(), profile.local.clone(), req.clone())).await.unwrap();
                forward_request(req, profile.local.clone(), http.clone());
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

pub fn forward_request(req: RequestMessage, mut local: Url, http: Client) -> JoinHandle<()> {
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
