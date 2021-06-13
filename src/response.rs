use crate::{error::Error, shared::SharedData, Result};
use core::task::Waker;
use std::{collections::HashMap, sync::Arc};
use std::{future::Future, task::Poll};
use uuid::Uuid;

pub type ResponseStatusMap = HashMap<Uuid, Status>;

pub enum Status {
    None,
    Started(Waker),
    Completed(Vec<u8>),
}

pub struct Response {
    uuid: Uuid,
    data: Vec<u8>,
    shared: Arc<SharedData>,
}

impl Response {
    pub fn new(uuid: Uuid, data: Vec<u8>, shared: Arc<SharedData>) -> Self {
        Response { uuid, data, shared }
    }
}

impl Response {
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

    pub async fn reply(&self, message: Vec<u8>) -> Result<ResponseFuture> {
        let mut map = self.shared.get_responses()?;
        if let None = map.get(&self.uuid) {
            map.insert(self.uuid, crate::response::Status::None);
        }
        self.shared.send(self.uuid, message)
    }

    pub async fn reply_message(&self, message: &dyn protobuf::Message) -> Result<ResponseFuture> {
        let mut map = self.shared.get_responses()?;
        if let None = map.get(&self.uuid) {
            map.insert(self.uuid, crate::response::Status::None);
        }
        self.shared.send_message(self.uuid, message)
    }
}

pub struct ResponseFuture {
    uuid: Uuid,
    shared: Arc<SharedData>,
}

impl ResponseFuture {
    pub(crate) fn new(uuid: Uuid, shared: Arc<SharedData>) -> Self {
        ResponseFuture { uuid, shared }
    }
}

impl Drop for ResponseFuture {
    fn drop(&mut self) {
        if let Ok(mut responses) = self.shared.get_responses() {
            responses.remove(&self.uuid);
        }
    }
}

impl Future for ResponseFuture {
    type Output = Result<Response>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if let Ok(mut responses) = self.shared.get_responses() {
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
                            self.shared.clone(),
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
