use crate::{
    error::Error,
    protos::{self},
    response::{ResponseFuture, ResponseShared},
    Result,
};
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

pub async fn connect(url: &Url) -> Result<Client> {
    connect_with_config(url, None).await
}

pub async fn connect_with_config(url: &Url, config: Option<Config>) -> Result<Client> {
    match connect_async(url).await {
        Ok((stream, _)) => {
            let (write, read) = stream.split();
            let (tx, rx) = futures_channel::mpsc::unbounded();
            let rx_to_ws_write = rx.map(Ok).forward(write);
            let shared = Arc::new(ResponseShared {
                bandwidth: if let Some(ref config) = config {
                    config.bandwidth
                } else {
                    1024 * 1024 * 16
                },
                responses: Arc::new(Mutex::new(HashMap::new())),
                tx,
            });
            let shared2 = shared.clone();
            Ok(Client {
                thread: tokio::spawn(async move {
                    let ws_read = read.for_each(|message| async {
                        match message {
                            Ok(message) => {
                                match protos::message::Oneshot::parse_from_bytes(
                                    &message.into_data(),
                                ) {
                                    Ok(oneshot) => match Uuid::from_slice(oneshot.get_uuid()) {
                                        Ok(uuid) => {
                                            if let Ok(mut shared) = shared2.responses.lock() {
                                                if let Some(status) = shared.remove(&uuid) {
                                                    match status {
                                                        crate::response::Status::None => {}
                                                        crate::response::Status::Started(waker) => {
                                                            shared.insert(
                                                                uuid,
                                                                crate::response::Status::Completed(
                                                                    oneshot.message,
                                                                ),
                                                            );
                                                            waker.wake();
                                                        }
                                                        crate::response::Status::Completed(_) => {}
                                                    }
                                                }
                                            }
                                        }
                                        Err(_) => {}
                                    },
                                    Err(_) => {}
                                }
                            }
                            Err(_) => {}
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

#[derive(Debug)]
pub struct Config {
    pub bandwidth: usize,
}
