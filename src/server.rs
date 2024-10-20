use std::{ops::Deref, sync::LazyLock, time::Duration};

use actix_web::{
    dev::{ConnectionInfo, ServiceRequest},
    get,
    middleware::Logger,
    web::{self, Data},
    App, HttpRequest, HttpResponse, HttpServer, Responder,
};
use actix_web_httpauth::{
    extractors::{basic::BasicAuth, AuthenticationError},
    headers::www_authenticate::basic::Basic,
    middleware::HttpAuthentication,
};
use actix_ws::Message;
use env_logger::Env;
use futures_util::StreamExt as _;
use hookhub::RequestMessage;
use log::{info, warn};
use tokio::sync::broadcast;

static SECRET: LazyLock<String> =
    LazyLock::new(|| std::env::var("HOOKHUB_SECRET").unwrap_or("abc123".to_string()));

static BIND_ADDR: LazyLock<String> =
    LazyLock::new(|| std::env::var("HOOKHUB_BIND_ADDR").unwrap_or("127.0.0.1:9873".to_string()));

const VERSION: &str = env!("CARGO_PKG_VERSION");

async fn basic_auth_validator(
    req: ServiceRequest,
    credentials: BasicAuth,
) -> Result<ServiceRequest, (actix_web::Error, ServiceRequest)> {
    if let Some(password) = credentials.password() {
        if password != *SECRET {
            return Err((
                actix_web::error::ErrorUnauthorized(AuthenticationError::new(Basic::new())),
                req,
            ));
        }
    } else {
        return Err((
            actix_web::error::ErrorUnauthorized(AuthenticationError::new(Basic::new())),
            req,
        ));
    }

    if credentials.user_id() != VERSION {
        return Err((
            actix_web::error::ErrorBadRequest(format!(
                "Server is running version {} but you are running {}",
                credentials.user_id(),
                VERSION
            )),
            req,
        ));
    }

    Ok(req)
}

#[derive(Clone)]
struct Broadcaster(broadcast::Sender<RequestMessage>);

impl Broadcaster {
    fn send(&self, msg: RequestMessage) {
        if let Ok(count) = self.0.send(msg) {
            info!("Forwarded request to {} client(s)", count);
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<RequestMessage> {
        self.0.subscribe()
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let (tx, _) = broadcast::channel::<RequestMessage>(50);
    let broadcaster = Broadcaster(tx);

    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .app_data(Data::new(broadcaster.clone()))
            .service(
                web::scope("/__hookhub__")
                    .wrap(HttpAuthentication::basic(basic_auth_validator))
                    .service(handle_websocket),
            )
            .default_service(web::to(handle_receive))
    })
    .keep_alive(Duration::from_secs(30))
    .shutdown_timeout(10)
    .bind(BIND_ADDR.deref())?
    .run()
    .await
}

#[get("/")]
async fn handle_websocket(
    req: HttpRequest,
    body: web::Payload,
    connection_info: ConnectionInfo,
    broadcaster: Data<Broadcaster>,
) -> actix_web::Result<impl Responder> {
    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, body)?;

    let remote_addr = connection_info.realip_remote_addr().unwrap().to_owned();

    info!("[{remote_addr}] Session started");

    let mut receiver = broadcaster.subscribe();

    actix_web::rt::spawn(async move {
        loop {
            tokio::select! {
                message = msg_stream.next() => {
                    match message {
                        Some(Ok(Message::Ping(bytes))) => {
                            if session.pong(&bytes).await.is_err() {
                                break;
                            }
                        },
                        Some(Ok(Message::Close(_))) => {
                            break;
                        },
                        Some(Ok(_)) => {},
                        Some(Err(err)) => {
                            warn!("[{remote_addr}] {err}");
                            break;
                        }
                        None => {
                            break;
                        }
                    }
                },
                Ok(msg) = receiver.recv() => {
                    if let Err(err) = session.binary(rmp_serde::to_vec(&msg).unwrap()).await {
                        warn!("[{remote_addr}] {err}");
                        break;
                    }
                }
            }
        }

        let _ = session.close(None).await;

        info!("[{remote_addr}] Session finished");
    });

    Ok(response)
}

async fn handle_receive(
    req: HttpRequest,
    payload: web::Bytes,
    broadcaster: Data<Broadcaster>,
) -> impl Responder {
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .filter(|(k, _)| k.as_str() != "host")
        .filter(|(k, _)| k.as_str() != "origin")
        .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap().to_owned()))
        .collect();

    let message = RequestMessage {
        method: req.head().method.to_string(),
        fullpath: req.head().uri.to_string(),
        version: req.head().version.into(),
        headers,
        body: payload.into(),
    };

    broadcaster.send(message);

    HttpResponse::Ok()
}
