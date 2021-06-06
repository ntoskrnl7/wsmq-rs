use std::collections::HashMap;
use std::sync::Mutex;
use std::{net::SocketAddr, sync::Arc};

use futures::future;
use futures::StreamExt;
use futures::TryStreamExt;
use futures_channel::mpsc::unbounded;
use protobuf::Message;

use crate::protos::message;
use crate::response::{Response, ResponseShared};

type OnStarted = Box<dyn Fn() + Send + Sync>;
type OnConnect = Box<dyn Fn(SocketAddr) + Send + Sync>;
type OnDisconnect = Box<dyn Fn(SocketAddr) + Send + Sync>;
type OnError = Box<dyn Fn(Box<dyn std::error::Error>) + Send + Sync>;

pub struct Config<
    OnMessage: Fn(SocketAddr, crate::response::Response) -> () + Send + Sync + 'static,
> {
    pub bandwidth: usize,
    pub on_message: OnMessage,
    pub on_started: Option<OnStarted>,
    pub on_connect: Option<OnConnect>,
    pub on_disconnect: Option<OnDisconnect>,
    pub on_error: Option<OnError>,
}

macro_rules! define_method {
    ($prop:ident, $type:ty) => {
        pub fn $prop(mut self, callback: $type) -> Self {
            self.$prop = Some(callback);
            self
        }
    };
}

#[macro_export]
macro_rules! run_server {
    ($addr:expr, $config:expr) => {
        wsmq_rs::server::run_with_config($addr, $config)
    };
}

impl<OnMessage: Fn(SocketAddr, crate::response::Response) -> () + Send + Sync + 'static>
    Config<OnMessage>
{
    pub fn new(bandwidth: usize, on_message: OnMessage) -> Self {
        Config {
            bandwidth,
            on_message,
            on_started: None,
            on_connect: None,
            on_disconnect: None,
            on_error: None,
        }
    }
    define_method!(on_started, OnStarted);
    define_method!(on_disconnect, OnDisconnect);
    define_method!(on_error, OnError);
    define_method!(on_connect, OnConnect);
}

pub async fn run<
    OnMessage: Fn(SocketAddr, crate::response::Response) -> () + Send + Sync + 'static,
>(
    addr: &SocketAddr,
    config: Config<OnMessage>,
) -> crate::Result<()> {
    match tokio::net::TcpListener::bind(addr).await {
        Ok(socket) => {
            let config = Arc::new(config);
            if let Some(ref on_started) = config.on_started {
                on_started();
            }
            let config2 = config.clone();
            while let Ok((stream, addr)) = socket.accept().await {
                let config = config2.clone();
                tokio::spawn(async move {
                    let config2 = config.clone();
                    match tokio_tungstenite::accept_async(stream).await {
                        Ok(stream) => {
                            let config = config2.clone();
                            let (tx, rx) = unbounded();
                            let shared = ResponseShared {
                                bandwidth: config.bandwidth,
                                responses: Arc::new(Mutex::new(HashMap::new())),
                                tx,
                            };
                            if let Some(ref on_connect) = config.on_connect {
                                on_connect(addr);
                            }
                            let (write, read) = stream.split();
                            let read = read.try_for_each(|message| {
                                let data = &message.into_data();
                                match message::Header::parse_from_bytes(data) {
                                    Ok(header) => match uuid::Uuid::from_slice(header.get_uuid()) {
                                        Ok(uuid) => match header.get_field_type() {
                                            message::Type::ONESHOT => {
                                                if let Ok(oneshot) =
                                                    message::Oneshot::parse_from_bytes(data)
                                                {
                                                    (config.on_message)(
                                                        addr,
                                                        Response::new(
                                                            uuid,
                                                            oneshot.message,
                                                            &shared,
                                                        ),
                                                    );
                                                }
                                            }
                                            message::Type::BEGIN => {}
                                            message::Type::PROCESS => {}
                                            message::Type::END => {}
                                        },
                                        Err(err) => {
                                            if let Some(ref on_error) = config.on_error {
                                                on_error(Box::new(err));
                                            }
                                        }
                                    },
                                    Err(err) => {
                                        if let Some(ref on_error) = config.on_error {
                                            on_error(Box::new(err));
                                        }
                                    }
                                }
                                future::ok(())
                            });
                            let write = rx.map(Ok).forward(write);
                            future::select(read, write).await;
                            if let Some(ref on_disconnect) = config.on_disconnect {
                                on_disconnect(addr);
                            }
                        }
                        Err(err) => {
                            if let Some(ref on_error) = config.on_error {
                                on_error(Box::new(err));
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
