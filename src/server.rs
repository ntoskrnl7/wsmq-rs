use crate::{
    callbacks::{OnConnect, OnDisconnect, OnError, OnStarted},
    define_method,
    message::OnProgress,
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::{net::SocketAddr, sync::Arc};

use futures::future;
use futures::StreamExt;
use futures::TryStreamExt;
use futures_channel::mpsc::unbounded;
use protobuf::Message;
use uuid::Uuid;

use crate::response::{Response, ResponseShared};
use crate::{message::MessageContext, message_dispatch};

pub struct Config {
    pub bandwidth: usize,
    pub on_started: Option<OnStarted>,
    pub on_connect: Option<OnConnect>,
    pub on_disconnect: Option<OnDisconnect>,
    pub on_progress: Option<OnProgress>,
    pub on_error: Option<OnError>,
}

impl Config {
    pub fn new(bandwidth: usize) -> Self {
        Config {
            bandwidth,
            on_started: None,
            on_connect: None,
            on_disconnect: None,
            on_progress: None,
            on_error: None,
        }
    }
    define_method!(on_started, OnStarted);
    define_method!(on_disconnect, OnDisconnect);
    define_method!(on_error, OnError);
    define_method!(on_progress, OnProgress);
    define_method!(on_connect, OnConnect);
}

pub async fn run<
    OnMessage: Fn(SocketAddr, crate::response::Response) -> () + Send + Sync + 'static,
>(
    addr: &SocketAddr,
    on_message: OnMessage,
) -> crate::Result<()> {
    run_with_config2(addr, on_message, None).await
}

pub async fn run_with_config<
    OnMessage: Fn(SocketAddr, crate::response::Response) -> () + Send + Sync + 'static,
>(
    addr: &SocketAddr,
    on_message: OnMessage,
    config: Config,
) -> crate::Result<()> {
    run_with_config2(addr, on_message, Some(config)).await
}

pub async fn run_with_config2<
    OnMessage: Fn(SocketAddr, crate::response::Response) -> () + Send + Sync + 'static,
>(
    addr: &SocketAddr,
    on_message: OnMessage,
    config: Option<Config>,
) -> crate::Result<()> {
    match tokio::net::TcpListener::bind(addr).await {
        Ok(socket) => {
            let config = Arc::new(config);
            let on_message = Arc::new(on_message);
            if let Some(config) = config.as_ref() {
                if let Some(ref on_started) = config.on_started {
                    on_started();
                }
            }
            let config2 = config.clone();
            let on_message2 = on_message.clone();
            while let Ok((stream, addr)) = socket.accept().await {
                let config = config2.clone();
                let on_message = on_message2.clone();
                tokio::spawn(async move {
                    let config2 = config.clone();
                    match tokio_tungstenite::accept_async(stream).await {
                        Ok(stream) => {
                            let config = config2.clone();
                            let (tx, rx) = unbounded();
                            let shared = ResponseShared {
                                bandwidth: if let Some(config) = config.as_ref() {
                                    config.bandwidth
                                } else {
                                    1024 * 1024 * 16
                                },
                                responses: Arc::new(Mutex::new(HashMap::new())),
                                tx,
                            };
                            let message_contexts = Arc::new(Mutex::new(HashMap::new()));
                            if let Some(config) = config.as_ref() {
                                if let Some(ref on_connect) = config.on_connect {
                                    on_connect(addr);
                                }
                            }

                            let (write, read) = stream.split();
                            let read = read.try_for_each(|message| {
                                message_dispatch!(
                                    message_contexts,
                                    &message.into_data(),
                                    |uuid, message| {
                                        (on_message)(addr, Response::new(uuid, message, &shared));
                                    },
                                    |context| {
                                        if let Some(config) = config.as_ref() {
                                            if let Some(ref on_progress) = config.on_progress {
                                                on_progress(context);
                                            }
                                        }
                                    },
                                    |err| {
                                        if let Some(config) = config.as_ref() {
                                            if let Some(ref on_error) = config.on_error {
                                                on_error(Box::new(err));
                                            }
                                        }
                                    }
                                );
                                future::ok(())
                            });
                            let write = rx.map(Ok).forward(write);
                            future::select(read, write).await;
                            if let Some(config) = config.as_ref() {
                                if let Some(ref on_disconnect) = config.on_disconnect {
                                    on_disconnect(addr);
                                }
                            }
                        }
                        Err(err) => {
                            if let Some(config) = config.as_ref() {
                                if let Some(ref on_error) = config.on_error {
                                    on_error(Box::new(err));
                                }
                            }
                        }
                    }
                });
            }
            Ok(())
        }
        Err(err) => Err(crate::error::Error::InternalError(Box::new(err))),
    }
}
