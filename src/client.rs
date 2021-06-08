use crate::{
    callbacks::OnError,
    define_method,
    error::Error,
    message::OnProgress,
    response::{ResponseFuture, ResponseShared},
    Result,
};
use crate::{message::MessageContext, message_dispatch};
use futures::{future, pin_mut, StreamExt};
use protobuf::Message;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async;
use url::Url;
use uuid::Uuid;

pub struct Config {
    bandwidth: usize,
    on_error: Option<OnError>,
    on_progress: Option<OnProgress>,
}

impl Config {
    pub fn new(bandwidth: usize) -> Self {
        Config {
            bandwidth,
            on_error: None,
            on_progress: None,
        }
    }
    define_method!(on_error, OnError);
    define_method!(on_progress, OnProgress);
}

pub struct Client {
    thread: JoinHandle<()>,
    shared: Arc<ResponseShared>,
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

pub async fn connect(url: &Url) -> Result<Client> {
    connect_with_config2(url, None).await
}

pub async fn connect_with_config(url: &Url, config: Config) -> Result<Client> {
    connect_with_config2(url, Some(config)).await
}

pub async fn connect_with_config2(url: &Url, config: Option<Config>) -> Result<Client> {
    match connect_async(url).await {
        Ok((stream, _)) => {
            let (write, read) = stream.split();
            let (tx, rx) = futures_channel::mpsc::unbounded();
            let rx_to_ws_write = rx.map(Ok).forward(write);
            let config1 = Arc::new(config);
            let shared = Arc::new(ResponseShared {
                bandwidth: if let Some(config) = config1.as_ref() {
                    config.bandwidth
                } else {
                    1024 * 1024 * 16
                },
                responses: Arc::new(Mutex::new(HashMap::new())),
                tx,
            });
            let message_contexts = Arc::new(Mutex::new(HashMap::new()));
            let shared2 = shared.clone();
            let config2 = config1.clone();
            Ok(Client {
                thread: tokio::spawn(async move {
                    let config3 = config2.clone();
                    let ws_read = read.for_each(|message| async {
                        match message {
                            Ok(message) => {
                                let config4 = config3.clone();
                                message_dispatch!(
                                    message_contexts,
                                    &message.into_data(),
                                    |uuid, message| {
                                        if let Ok(mut shared) = shared2.responses.lock() {
                                            if let Some(status) = shared.remove(&uuid) {
                                                match status {
                                                    crate::response::Status::None => {}
                                                    crate::response::Status::Started(waker) => {
                                                        shared.insert(
                                                            uuid,
                                                            crate::response::Status::Completed(
                                                                message,
                                                            ),
                                                        );
                                                        waker.wake();
                                                    }
                                                    crate::response::Status::Completed(_) => {}
                                                }
                                            }
                                        }
                                    },
                                    |context| {
                                        if let Some(ref config) = config4.as_ref() {
                                            if let Some(on_progress) = &config.on_progress {
                                                on_progress(context);
                                            }
                                        }
                                    },
                                    |err| {
                                        if let Some(ref config) = config4.as_ref() {
                                            if let Some(on_error) = &config.on_error {
                                                on_error(err);
                                            }
                                        }
                                    }
                                );
                            }
                            Err(err) => {
                                if let Some(ref config) = config3.as_ref() {
                                    if let Some(on_error) = &config.on_error {
                                        on_error(Box::new(err));
                                    }
                                }
                            }
                        }
                    });
                    pin_mut!(rx_to_ws_write, ws_read);
                    future::select(rx_to_ws_write, ws_read).await;
                }),
                shared: shared.clone(),
            })
        }
        Err(err) => Err(Error::InternalError(Box::new(err))),
    }
}
