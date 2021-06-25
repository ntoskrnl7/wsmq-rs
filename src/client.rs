use crate::{
    define_set_callback, error::Error, message::MessageContext, message_dispatch,
    response::ResponseFuture, shared::SharedData, Result,
};
use futures::{
    future::{self, Either},
    pin_mut, StreamExt, TryStreamExt,
};
use protobuf::Message;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

type OnProgress = Box<dyn Fn(&crate::message::ProgressContext) + Send + Sync>;
type OnError = Box<dyn Fn(Box<dyn std::error::Error>) + Send + Sync>;

struct NoCertificateVerification {}
impl rustls::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _roots: &rustls::RootCertStore,
        _presented_certs: &[rustls::Certificate],
        _dns_name: tokio_rustls::webpki::DNSNameRef,
        _ocsp_response: &[u8],
    ) -> std::result::Result<rustls::ServerCertVerified, rustls::TLSError> {
        Ok(rustls::ServerCertVerified::assertion())
    }
}

pub struct Config {
    bandwidth: usize,
    verifier: Option<Arc<dyn rustls::ServerCertVerifier>>,
    on_error: Option<OnError>,
    on_progress: Option<OnProgress>,
}

pub static DEFAULT_BANDWIDTH: usize = 1024 * 1024 * 16;

impl Config {
    pub fn new() -> Self {
        Config {
            bandwidth: DEFAULT_BANDWIDTH,
            verifier: None,
            on_error: None,
            on_progress: None,
        }
    }

    pub fn set_bandwidth(mut self, bandwidth: usize) -> Self {
        self.bandwidth = bandwidth;
        self
    }

    pub fn no_certificate_verification(mut self) -> Self {
        self.verifier = Some(Arc::new(NoCertificateVerification {}));
        self
    }

    pub fn set_verifier(mut self, verifier: Arc<dyn rustls::ServerCertVerifier>) -> Self {
        self.verifier = Some(verifier);
        self
    }

    define_set_callback!(on_error, OnError);
    define_set_callback!(on_progress, OnProgress);
}

pub struct Client {
    thread: JoinHandle<()>,
    shared: Arc<SharedData>,
}

impl Drop for Client {
    fn drop(&mut self) {
        self.thread.abort();
    }
}

impl Client {
    pub fn close(&self) {
        self.thread.abort();
    }

    pub fn send(&self, message: Vec<u8>) -> Result<ResponseFuture> {
        self.shared.send(self.shared.generate_uuid(), message)
    }

    pub fn send_message(&self, message: &dyn protobuf::Message) -> Result<ResponseFuture> {
        self.shared
            .send_message(self.shared.generate_uuid(), message)
    }
}

pub async fn connect<R>(url: R) -> Result<Client>
where
    R: IntoClientRequest + Unpin,
{
    connect_with_config2(url, None).await
}

pub async fn connect_with_config<R>(url: R, config: Config) -> Result<Client>
where
    R: IntoClientRequest + Unpin,
{
    connect_with_config2(url, Some(config)).await
}

use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

fn domain(
    request: &tokio_tungstenite::tungstenite::handshake::client::Request,
) -> std::result::Result<String, tokio_tungstenite::tungstenite::error::Error> {
    match request.uri().host() {
        Some(d) => Ok(d.to_string()),
        None => Err(tokio_tungstenite::tungstenite::error::Error::Url(
            tokio_tungstenite::tungstenite::error::UrlError::NoHostName,
        )),
    }
}

async fn connect_async_with_config<R>(
    request: R,
    config: &Option<Config>,
    ws_config: Option<WebSocketConfig>,
) -> std::result::Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    tokio_tungstenite::tungstenite::error::Error,
>
where
    R: IntoClientRequest + Unpin,
{
    let request = request.into_client_request()?;
    let domain = domain(&request)?;
    let port = request
        .uri()
        .port_u16()
        .or_else(|| match request.uri().scheme_str() {
            Some("wss") => Some(443),
            Some("ws") => Some(80),
            _ => None,
        })
        .ok_or(tokio_tungstenite::tungstenite::error::Error::Url(
            tokio_tungstenite::tungstenite::error::UrlError::UnsupportedUrlScheme,
        ))?;

    let addr = format!("{}:{}", domain, port);
    let try_socket = tokio::net::TcpStream::connect(addr).await;
    let socket = try_socket.map_err(tokio_tungstenite::tungstenite::error::Error::Io)?;

    let connector = if let Some("wss") = request.uri().scheme_str() {
        let mut con_config = rustls::ClientConfig::new();
        if let Some(config) = config {
            if let Some(ref verifier) = config.verifier {
                let mut cfg = rustls::DangerousClientConfig {
                    cfg: &mut con_config,
                };
                cfg.set_certificate_verifier(verifier.clone());
            }
        }
        Some(tokio_tungstenite::Connector::Rustls(Arc::new(con_config)))
    } else {
        None
    };
    tokio_tungstenite::client_async_tls_with_config(request, socket, ws_config, connector).await
}

async fn connect_with_config2<R>(url: R, config: Option<Config>) -> Result<Client>
where
    R: IntoClientRequest + Unpin,
{
    match connect_async_with_config(url, &config, None).await {
        Ok((stream, _)) => {
            let (write, read) = stream.split();
            let (tx, rx) = futures_channel::mpsc::unbounded();
            let rx_to_ws_write = rx.map(Ok).forward(write);
            let config = Arc::new(config);
            let shared = Arc::new(SharedData::new(
                if let Some(config) = config.as_ref() {
                    config.bandwidth
                } else {
                    DEFAULT_BANDWIDTH
                },
                tx,
            ));
            let message_contexts = Arc::new(Mutex::new(HashMap::new()));
            let shared2 = shared.clone();
            Ok(Client {
                thread: tokio::spawn(async move {
                    let ws_read = read.try_for_each(|message| {
                        message_dispatch!(
                            message_contexts,
                            &message.into_data(),
                            |uuid, message| {
                                match shared2.get_responses() {
                                    Ok(mut shared) => {
                                        if let Some(status) = shared.remove(&uuid) {
                                            if let crate::response::Status::Started(waker) = status
                                            {
                                                shared.insert(
                                                    uuid,
                                                    crate::response::Status::Completed(message),
                                                );
                                                waker.wake();
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        if let Some(ref config) = config.as_ref() {
                                            if let Some(on_error) = &config.on_error {
                                                on_error(Box::new(err));
                                            }
                                        }
                                    }
                                }
                            },
                            |context| {
                                if let Some(ref config) = config.as_ref() {
                                    if let Some(on_progress) = &config.on_progress {
                                        on_progress(context);
                                    }
                                }
                            },
                            |err| {
                                if let Some(ref config) = config.as_ref() {
                                    if let Some(on_error) = &config.on_error {
                                        on_error(err);
                                    }
                                }
                            },
                            |_| {}
                        );
                        futures::future::ok(())
                    });
                    pin_mut!(rx_to_ws_write, ws_read);
                    if let Err(err) = match future::select(rx_to_ws_write, ws_read).await {
                        Either::Left((value1, _)) => value1,
                        Either::Right((value2, _)) => value2,
                    } {
                        if let Some(ref config) = config.as_ref() {
                            if let Some(on_error) = &config.on_error {
                                on_error(Box::new(err));
                            }
                        }
                    }
                    shared2.clear();
                }),
                shared,
            })
        }
        Err(err) => Err(Error::InternalError(Box::new(err))),
    }
}
