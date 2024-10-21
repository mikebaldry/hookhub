use std::time::Duration;

use actix_web::{
    dev::{ConnectionInfo, ServiceRequest}, get, guard, middleware::Logger, web::{self, Data}, App, HttpRequest, HttpResponse, HttpServer, Responder
};
use actix_web_httpauth::{
    extractors::{basic::BasicAuth, AuthenticationError},
    headers::www_authenticate::basic::Basic,
    middleware::HttpAuthentication,
};
use actix_ws::Message;
use anyhow::Result;
use futures_util::StreamExt as _;
use hookhub::RequestMessage;
use log::{info, warn};
use tokio::sync::broadcast;

use crate::VERSION;

pub async fn handle(bind_addr: String, secret: String) -> Result<()> {
    let (tx, _) = broadcast::channel::<RequestMessage>(50);
    let broadcaster = Broadcaster(tx);

    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .app_data(Data::new(broadcaster.clone()))
            .service(
                web::scope("/__hookhub__")
                    .guard(AuthGuard::new(secret.clone()))
                    .service(handle_websocket),
            )
            .default_service(web::to(handle_receive))
    })
    .keep_alive(Duration::from_secs(30))
    .shutdown_timeout(10)
    .bind(bind_addr)?
    .run()
    .await
    .map_err(|e| e.into())
}

#[derive(Clone)]
struct Broadcaster(broadcast::Sender<RequestMessage>);

#[derive(Clone)]
struct Secret(String);

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

#[get("/")]
async fn handle_websocket(
    req: HttpRequest,
    body: web::Payload,
    connection_info: ConnectionInfo,
    broadcaster: Data<Broadcaster>,
    secret: Data<Secret>,
) -> actix_web::Result<impl Responder> {
    // req.

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
        .filter(|(k, _)| k.as_str() != "connection")
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

fn basic_auth_validator(
    req: ServiceRequest,
    credentials: BasicAuth,
    secret: String,
) -> Result<ServiceRequest, (actix_web::Error, ServiceRequest)> {
    // let secret = req.app_data::<Secret>().unwrap();

    if let Some(password) = credentials.password() {
        if password != secret.0 {
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


impl Guard for Secret {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        if let Some(val) = ctx.head().headers.get(&self.0) {
            return val == self.1;
        }

        false
    }
}
