use std::collections::HashMap;
use std::io::Read;
use std::sync::Mutex;
use std::{net::SocketAddr, sync::Arc};

use futures::future;
use futures::StreamExt;
use futures::TryStreamExt;
use futures_channel::mpsc::unbounded;
use protobuf::Message;
use uuid::Uuid;

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

struct Buffer {
    compression: crate::protos::message::Compression,
    //total_length: u64,
    buffer: Vec<u8>,
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
                            let buffers = Arc::new(Mutex::new(HashMap::new()));
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
                                            message::Type::BEGIN => {
                                                if let Ok(begin) =
                                                    message::Begin::parse_from_bytes(data)
                                                {
                                                    println!("begin : {:?}", begin);
                                                    if let Ok(uuid) =
                                                        Uuid::from_slice(begin.get_uuid())
                                                    {
                                                        buffers.lock().unwrap().insert(
                                                            uuid,
                                                            Buffer {
                                                                compression: begin
                                                                    .get_compression(),
                                                                //total_length: begin.get_length(),
                                                                buffer: Vec::new(),
                                                            },
                                                        );
                                                    }
                                                }
                                            }
                                            message::Type::PROCESS => {
                                                if let Ok(process) =
                                                    message::Process::parse_from_bytes(data)
                                                {
                                                    if let Ok(uuid) =
                                                        Uuid::from_slice(process.get_uuid())
                                                    {
                                                        if let Some(ref mut buffer) =
                                                            buffers.lock().unwrap().get_mut(&uuid)
                                                        {
                                                            buffer.buffer.extend(process.get_message());
                                                        }
                                                    }
                                                }
                                            }
                                            message::Type::END => {
                                                if let Ok(end) =
                                                    message::End::parse_from_bytes(data)
                                                {
                                                    if let Ok(uuid) =
                                                        Uuid::from_slice(end.get_uuid())
                                                    {
                                                        if let Some(ref mut buffer) =
                                                            buffers.lock().unwrap().remove(&uuid)
                                                        {
                                                            if buffer.compression == crate::protos::message::Compression::SNAPPY {
                                                                let mut decomp = vec![];
                                                                match snap::read::FrameDecoder::new(&buffer.buffer[..]).read_to_end(&mut decomp) {
                                                                    Ok(_) => {
                                                                        (config.on_message)(
                                                                            addr,
                                                                            Response::new(uuid, decomp, &shared),
                                                                        );
                                                                    },
                                                                    Err(err) => {
                                                                        if let Some(ref on_error) = config.on_error {
                                                                            on_error(Box::new(err));
                                                                        }
                                                                    }
                                                                }                                                                
                                                            }
                                                        }
                                                    }
                                                }
                                            }
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
