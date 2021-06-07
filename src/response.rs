use crate::{
    error::Error,
    protos::{self, message::Type},
    Result,
};
use core::task::Waker;
use futures_channel::mpsc::UnboundedSender;
use protobuf::Message;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use std::{future::Future, task::Poll};
use uuid::Uuid;

pub type ResponseStatusMap = HashMap<Uuid, Status>;

pub enum Status {
    None,
    Started(Waker),
    Completed(Vec<u8>),
}

pub struct Response<'a> {
    uuid: Uuid,
    data: Vec<u8>,
    shared: &'a ResponseShared,
}

impl<'a> Response<'a> {
    pub fn new(uuid: Uuid, data: Vec<u8>, shared: &'a ResponseShared) -> Self {
        Response { uuid, data, shared }
    }
}

impl<'a> Response<'a> {
    pub fn to_vec(&self) -> &Vec<u8> {
        &self.data
    }

    pub fn to_message<R>(&self) -> Result<R>
    where
        R: protobuf::Message,
    {
        match R::parse_from_bytes(&self.data) {
            Ok(message) => Ok(message),
            Err(err) => Err(Error::InternalError(Box::new(err))),
        }
    }

    pub async fn send_message(
        &self,
        message: &dyn protobuf::Message,
    ) -> Result<ResponseFuture<'a>> {
        if let Ok(mut map) = self.shared.responses.lock() {
            if let None = map.get(&self.uuid) {
                map.insert(self.uuid, crate::response::Status::None);
            }
        }
        self.shared.send_message(self.uuid, message)
    }
}

pub struct ResponseFuture<'a> {
    uuid: Uuid,
    shared: &'a ResponseShared,
}

impl<'a> ResponseFuture<'a> {
    pub(crate) fn new(uuid: Uuid, shared: &'a ResponseShared) -> Self {
        ResponseFuture { uuid, shared }
    }
}

impl<'a> Drop for ResponseFuture<'a> {
    fn drop(&mut self) {
        if let Ok(mut responses) = self.shared.responses.lock() {
            responses.remove(&self.uuid);
        }
    }
}

impl<'a> Future for ResponseFuture<'a> {
    type Output = Result<Response<'a>>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if let Ok(mut responses) = self.shared.responses.lock() {
            if let Some(status) = responses.get_mut(&self.uuid) {
                match status {
                    Status::None => {
                        responses.insert(self.uuid, Status::Started(cx.waker().clone()));
                        return Poll::Pending;
                    }
                    Status::Started(_) => {
                        return Poll::Pending;
                    }
                    Status::Completed(data) => {
                        return Poll::Ready(Ok(Response::new(
                            self.uuid,
                            data.drain(..).collect(),
                            self.shared,
                        )));
                    }
                }
            }
            Poll::Ready(Err(Error::NotFound()))
        } else {
            Poll::Ready(Err(Error::MutexError()))
        }
    }
}

pub struct ResponseShared {
    pub bandwidth: usize,
    pub responses: Arc<Mutex<ResponseStatusMap>>,
    pub tx: UnboundedSender<tokio_tungstenite::tungstenite::Message>,
}

impl ResponseShared {
    pub(crate) fn generate_uuid(&self) -> Uuid {
        match self.responses.lock() {
            Ok(mut map) => loop {
                let uuid = Uuid::new_v4();
                if map.iter().find(|e| e.0 == &uuid).is_none() {
                    map.insert(uuid, crate::response::Status::None);
                    break uuid;
                }
            },
            Err(_) => Uuid::nil(),
        }
    }

    pub fn new_response<'a>(&'a self, uuid: Uuid, data: Vec<u8>) -> Response<'a> {
        Response::new(uuid, data, self)
    }
    pub fn send(&self, uuid: Uuid, message: Vec<u8>) -> Result<ResponseFuture> {
        let mut oneshot = protos::message::Oneshot::new();
        oneshot.set_field_type(Type::ONESHOT);
        oneshot.set_uuid(uuid.as_bytes().to_vec());
        oneshot.set_message(message);
        match oneshot.write_to_bytes() {
            Ok(message) => {
                match self
                    .tx
                    .unbounded_send(tokio_tungstenite::tungstenite::Message::binary(message))
                {
                    Ok(_) => Ok(ResponseFuture::new(uuid, self)),
                    Err(err) => return Err(Error::InternalError(Box::new(err))),
                }
            }
            Err(err) => return Err(Error::InternalError(Box::new(err))),
        }
    }

    pub fn send_large_message(
        &self,
        _uuid: Uuid,
        _message: &dyn protobuf::Message,
    ) -> Result<ResponseFuture> {
        Err(Error::InvalidParameters())
    }

    pub fn send_message(
        &self,
        uuid: Uuid,
        message: &dyn protobuf::Message,
    ) -> Result<ResponseFuture> {
        if message.compute_size() > self.bandwidth as u32 {
            return self.send_large_message(uuid, message);
        }
        match message.write_to_bytes() {
            Ok(vec) => self.send(uuid, vec),
            Err(err) => Err(Error::InternalError(Box::new(err))),
        }
    }
}
