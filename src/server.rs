use crate::{
    define_set_callback,
    message::{MessageContext, ProgressContext},
    message_dispatch,
    response::Response,
    shared::SharedData,
    Result,
};
use futures::{future, StreamExt, TryStreamExt};
use futures_channel::mpsc::unbounded;
use protobuf::Message;
use std::{
    collections::HashMap,
    net::SocketAddr,
    str::FromStr,
    sync::{Arc, Mutex},
    vec,
};
use uuid::Uuid;

pub type OnStarted = Box<dyn Fn() + Send + Sync>;
pub type OnConnect<Context> = Box<dyn Fn(SocketAddr) -> Context + Send + Sync>;
pub type OnDisconnect<Context> = Box<dyn Fn(SocketAddr, &mut Context) + Send + Sync>;
pub type OnProgress<Context> = Box<dyn Fn(&ProgressContext, &mut Context) + Send + Sync>;
pub type OnError<Context> =
    Box<dyn Fn(Box<dyn std::error::Error>, Option<&mut Context>) + Send + Sync>;

pub struct Pem {
    pub cert: std::path::PathBuf,
    pub key: std::path::PathBuf,
}

pub static DEFAULT_BANDWIDTH: usize = 1024 * 1024 * 16;

pub struct Config<Context> {
    bandwidth: usize,
    pem: Option<Pem>,
    on_started: Option<OnStarted>,
    on_connect: Option<OnConnect<Context>>,
    on_disconnect: Option<OnDisconnect<Context>>,
    on_progress: Option<OnProgress<Context>>,
    on_error: Option<OnError<Context>>,
}

impl<Context> Config<Context> {
    pub fn new() -> Self {
        Config {
            bandwidth: DEFAULT_BANDWIDTH,
            pem: None,
            on_started: None,
            on_connect: None,
            on_disconnect: None,
            on_progress: None,
            on_error: None,
        }
    }

    pub fn set_bandwidth(mut self, bandwidth: usize) -> Self {
        self.bandwidth = bandwidth;
        self
    }

    pub fn set_pem(mut self, pem: Pem) -> Self {
        self.pem = Some(pem);
        self
    }

    define_set_callback!(on_started, OnStarted);
    define_set_callback!(on_disconnect, OnDisconnect<Context>);
    define_set_callback!(on_error, OnError<Context>);
    define_set_callback!(on_progress, OnProgress<Context>);
    define_set_callback!(on_connect, OnConnect<Context>);
}

pub trait Address {
    fn into(self) -> Result<SocketAddr>;
}

impl<'a> Address for &'a str {
    fn into(self) -> Result<SocketAddr> {
        match SocketAddr::from_str(self) {
            Ok(addr) => Ok(addr),
            Err(err) => Err(crate::error::Error::InternalError(Box::new(err))),
        }
    }
}

impl<'a> Address for &'a SocketAddr {
    fn into(self) -> Result<SocketAddr> {
        Ok(*self)
    }
}

pub async fn run<Addr, OnMessage>(addr: Addr, on_message: OnMessage) -> crate::Result<()>
where
    Addr: Address + Unpin,
    OnMessage: Fn(SocketAddr, crate::response::Response, &mut ()) -> () + Send + Sync + 'static,
{
    run_with_config2::<Addr, OnMessage, ()>(addr, on_message, None).await
}

pub async fn run_with_config<Addr, OnMessage, Context>(
    addr: Addr,
    on_message: OnMessage,
    config: Config<Context>,
) -> crate::Result<()>
where
    Addr: Address + Unpin,
    Context: 'static + Send,
    OnMessage:
        Fn(SocketAddr, crate::response::Response, &mut Context) -> () + Send + Sync + 'static,
{
    run_with_config2(addr, on_message, Some(config)).await
}

async fn run_with_config2<Addr, OnMessage, Context>(
    addr: Addr,
    on_message: OnMessage,
    config: Option<Config<Context>>,
) -> crate::Result<()>
where
    Addr: Address + Unpin,
    Context: 'static + Send,
    OnMessage:
        Fn(SocketAddr, crate::response::Response, &mut Context) -> () + Send + Sync + 'static,
{
    let addr = match addr.into() {
        Ok(addr) => addr,
        Err(err) => {
            return Err(err);
        }
    };

    let acceptor = if let Some(ref config) = config {
        if let Some(ref pem) = config.pem {
            let mut tls_config = ServerConfig::new(NoClientAuth::new());
            let cert_file = &mut BufReader::new(File::open(&pem.cert).unwrap());
            let key_file = &mut BufReader::new(File::open(&pem.key).unwrap());
            let cert_chain = certs(cert_file).unwrap();
            let mut keys = rsa_private_keys(key_file).unwrap();
            tls_config
                .set_single_cert(cert_chain, keys.remove(0))
                .unwrap();
            Some(tokio_rustls::TlsAcceptor::from(Arc::new(tls_config)))
        } else {
            None
        }
    } else {
        None
    };

    use std::fs::File;
    use std::io::BufReader;
    use tokio_rustls::rustls::{
        internal::pemfile::{certs, rsa_private_keys},
        NoClientAuth, ServerConfig,
    };

    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            let config = Arc::new(config);
            let on_message = Arc::new(on_message);
            if let Some(config) = config.as_ref() {
                if let Some(ref on_started) = config.on_started {
                    on_started();
                }
            }
            while let Ok((stream, addr)) = listener.accept().await {
                let config = config.clone();
                let on_message = on_message.clone();
                let acceptor = acceptor.clone();

                macro_rules! process_stream {
                    ($stream:ident) => {
                        match tokio_tungstenite::accept_async($stream).await {
                            Ok(stream) => {
                                let (tx, rx) = unbounded();
                                let shared = Arc::new(SharedData::new(
                                    if let Some(config) = config.as_ref() {
                                        config.bandwidth
                                    } else {
                                        1024 * 1024 * 16
                                    },
                                    tx,
                                ));
                                let message_contexts = Arc::new(Mutex::new(HashMap::new()));
                                let mut context: Context = if let Some(config) = config.as_ref() {
                                    if let Some(ref on_connect) = config.on_connect {
                                        on_connect(addr)
                                    } else {
                                        unsafe { std::mem::zeroed() }
                                    }
                                } else {
                                    unsafe { std::mem::zeroed() }
                                };

                                let (write, read) = stream.split();
                                let read = read.try_for_each(|message| {
                                    let shared = shared.clone();
                                    let mut canceled = false;
                                    message_dispatch!(
                                        message_contexts,
                                        &message.into_data(),
                                        |uuid, message| {
                                            (on_message)(
                                                addr,
                                                Response::new(uuid, message, shared.clone()),
                                                &mut context,
                                            );
                                        },
                                        |prg_ctx| {
                                            if let Some(config) = config.as_ref() {
                                                if let Some(ref on_progress) = config.on_progress {
                                                    on_progress(prg_ctx, &mut context);
                                                }
                                            }
                                        },
                                        |err| {
                                            if let Some(config) = config.as_ref() {
                                                if let Some(ref on_error) = config.on_error {
                                                    on_error(Box::new(err), Some(&mut context));
                                                }
                                            }
                                        },
                                        |_| {
                                            canceled = true;
                                        }
                                    );
                                    if canceled {
                                        return future::err::<
                                            (),
                                            tokio_tungstenite::tungstenite::Error,
                                        >(
                                            tokio_tungstenite::tungstenite::Error::ConnectionClosed,
                                        );
                                    }
                                    future::ok(())
                                });
                                let write = rx.map(Ok).forward(write);
                                future::select(read, write).await;
                                if let Some(config) = config.as_ref() {
                                    if let Some(ref on_disconnect) = config.on_disconnect {
                                        on_disconnect(addr, &mut context);
                                    }
                                }
                            }
                            Err(err) => {
                                if let Some(config) = config.as_ref() {
                                    if let Some(ref on_error) = config.on_error {
                                        on_error(Box::new(err), None);
                                    }
                                }
                            }
                        }
                    };
                }

                tokio::spawn(async move {
                    if let Some(acceptor) = acceptor {
                        match acceptor.accept(stream).await {
                            Ok(stream) => {
                                process_stream!(stream);
                            }
                            Err(_) => {}
                        }
                    } else {
                        process_stream!(stream);
                    }
                });
            }
            Ok(())
        }
        Err(err) => Err(crate::error::Error::InternalError(Box::new(err))),
    }
}
