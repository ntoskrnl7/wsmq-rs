use crate::{
    error::Error,
    protos::{self, message::Type},
    response::{self, ResponseFuture, ResponseStatusMap},
    Result,
};
use futures_channel::mpsc::UnboundedSender;
use protobuf::Message;
use std::{
    collections::HashMap,
    io::Write,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

pub struct SharedData {
    bandwidth: usize,
    responses: Arc<Mutex<ResponseStatusMap>>,
    tx: UnboundedSender<tokio_tungstenite::tungstenite::Message>,
}

impl SharedData {
    pub(crate) fn new(
        bandwidth: usize,
        tx: UnboundedSender<tokio_tungstenite::tungstenite::Message>,
    ) -> Self {
        SharedData {
            bandwidth,
            responses: Arc::new(Mutex::new(HashMap::new())),
            tx,
        }
    }
}

impl SharedData {
    pub(crate) fn clear(&self) {
        for (_, status) in self.responses.lock().unwrap().iter() {
            if let response::Status::Started(waker) = status {
                waker.wake_by_ref();
            }
        }
        self.responses.lock().unwrap().clear();
    }

    pub(crate) fn get_responses(
        &self,
    ) -> Result<std::sync::MutexGuard<HashMap<Uuid, crate::response::Status>>> {
        match self.responses.lock() {
            Ok(res) => Ok(res),
            Err(_) => Err(Error::MutexError()),
        }
    }

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

    pub(crate) fn send(self: &Arc<Self>, uuid: Uuid, message: Vec<u8>) -> Result<ResponseFuture> {
        if message.len() > self.bandwidth {
            let mut wtr = snap::write::FrameEncoder::new(vec![]);
            return match wtr.write_all(&message) {
                Ok(_) => match wtr.into_inner() {
                    Ok(message) => self.send_large_bytes(uuid, message),
                    Err(err) => return Err(Error::InternalError(Box::new(err))),
                },
                Err(err) => return Err(Error::InternalError(Box::new(err))),
            };
        }
        self.send_bytes(uuid, message)
    }

    pub(crate) fn send_message(
        self: &Arc<Self>,
        uuid: Uuid,
        message: &dyn protobuf::Message,
    ) -> Result<ResponseFuture> {
        if message.compute_size() > self.bandwidth as u32 {
            let mut wtr = snap::write::FrameEncoder::new(vec![]);
            return match message.write_to_writer(&mut wtr) {
                Ok(_) => match wtr.into_inner() {
                    Ok(message) => self.send_large_bytes(uuid, message),
                    Err(err) => return Err(Error::InternalError(Box::new(err))),
                },
                Err(err) => return Err(Error::InternalError(Box::new(err))),
            };
        }
        match message.write_to_bytes() {
            Ok(vec) => self.send_bytes(uuid, vec),
            Err(err) => Err(Error::InternalError(Box::new(err))),
        }
    }

    fn send_bytes(self: &Arc<Self>, uuid: Uuid, message: Vec<u8>) -> Result<ResponseFuture> {
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
                    Ok(_) => Ok(ResponseFuture::new(uuid, self.clone())),
                    Err(err) => return Err(Error::InternalError(Box::new(err))),
                }
            }
            Err(err) => return Err(Error::InternalError(Box::new(err))),
        }
    }

    fn send_large_bytes(
        self: &Arc<Self>,
        uuid: Uuid,
        mut message: Vec<u8>,
    ) -> Result<ResponseFuture> {
        let mut begin = protos::message::Begin::new();
        begin.set_field_type(Type::BEGIN);
        begin.set_uuid(uuid.as_bytes().to_vec());
        begin.set_compression(protos::message::Compression::SNAPPY);
        begin.set_length(message.len() as u64);
        match begin.write_to_bytes() {
            Ok(msg) => {
                match self
                    .tx
                    .unbounded_send(tokio_tungstenite::tungstenite::Message::binary(msg))
                {
                    Ok(_) => {
                        let mut seq = 0;
                        let mut process = protos::message::Process::new();
                        process.set_field_type(Type::PROCESS);
                        process.set_uuid(begin.get_uuid().to_vec());
                        for chunk in message.chunks_mut(self.bandwidth) {
                            process.set_seq(seq);
                            process.set_message(chunk.to_vec());
                            match process.write_to_bytes() {
                                Ok(msg) => match self.tx.unbounded_send(
                                    tokio_tungstenite::tungstenite::Message::binary(msg),
                                ) {
                                    Ok(_) => {
                                        seq += 1;
                                    }
                                    Err(err) => return Err(Error::InternalError(Box::new(err))),
                                },
                                Err(err) => return Err(Error::InternalError(Box::new(err))),
                            }
                        }
                        let mut end = protos::message::End::new();
                        end.set_field_type(Type::END);
                        end.set_uuid(begin.get_uuid().to_vec());
                        match end.write_to_bytes() {
                            Ok(msg) => match self.tx.unbounded_send(
                                tokio_tungstenite::tungstenite::Message::binary(msg),
                            ) {
                                Ok(_) => {}
                                Err(err) => return Err(Error::InternalError(Box::new(err))),
                            },
                            Err(err) => return Err(Error::InternalError(Box::new(err))),
                        }
                        Ok(ResponseFuture::new(uuid, self.clone()))
                    }
                    Err(err) => return Err(Error::InternalError(Box::new(err))),
                }
            }
            Err(err) => return Err(Error::InternalError(Box::new(err))),
        }
    }
}
