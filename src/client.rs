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
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};
use uuid::Uuid;

type OnProgress = Box<dyn Fn(&crate::message::ProgressContext) + Send + Sync>;
type OnError = Box<dyn Fn(Box<dyn std::error::Error>) + Send + Sync>;

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

async fn connect_with_config2<R>(url: R, config: Option<Config>) -> Result<Client>
where
    R: IntoClientRequest + Unpin,
{
    match connect_async(url).await {
        Ok((stream, _)) => {
            let (write, read) = stream.split();
            let (tx, rx) = futures_channel::mpsc::unbounded();
            let rx_to_ws_write = rx.map(Ok).forward(write);
            let config = Arc::new(config);
            let shared = Arc::new(SharedData::new(
                if let Some(config) = config.as_ref() {
                    config.bandwidth
                } else {
                    1024 * 1024 * 16
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
