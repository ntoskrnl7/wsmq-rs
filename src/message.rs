use std::sync::atomic::{AtomicBool, Ordering};

use uuid::Uuid;

use crate::protos::message;

pub struct MessageContext {
    pub compression: crate::protos::message::Compression,
    pub total_length: u64,
    pub buffer: Vec<u8>,
}

#[derive(Debug)]
pub struct ProgressContext {
    pub method: message::Type,
    uuid: Uuid,
    pub total_length: u64,
    pub current: u64,
    pub canceled: AtomicBool,
}

impl ProgressContext {
    pub fn new(method: message::Type, uuid: Uuid, total_length: u64, current: u64) -> Self {
        ProgressContext {
            method,
            uuid,
            total_length,
            current,
            canceled: AtomicBool::new(false),
        }
    }
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Relaxed);
    }
}

#[macro_export]
macro_rules! message_dispatch {
    ($contexts:expr, $data:expr, $on_message:expr, $on_callback:expr, $on_error:expr, $on_canceled:expr) => {
        use crate::message::ProgressContext;
        use crate::protos::message;
        use std::io::Read;
        use std::sync::atomic::Ordering;

        let data = $data;
        match message::Header::parse_from_bytes(data) {
            Ok(header) => match uuid::Uuid::from_slice(header.get_uuid()) {
                Ok(uuid) => match header.get_field_type() {
                    message::Type::ONESHOT => {
                        if let Ok(oneshot) = message::Oneshot::parse_from_bytes(data) {
                            $on_message(uuid, oneshot.message);
                        }
                    }
                    message::Type::BEGIN => {
                        if let Ok(begin) = message::Begin::parse_from_bytes(data) {
                            if let Ok(uuid) = Uuid::from_slice(begin.get_uuid()) {
                                $contexts.lock().unwrap().insert(
                                    uuid,
                                    MessageContext {
                                        compression: begin.get_compression(),
                                        total_length: begin.get_length(),
                                        buffer: Vec::new(),
                                    },
                                );
                                let pctx = ProgressContext::new(
                                    message::Type::BEGIN,
                                    uuid,
                                    begin.get_length(),
                                    0,
                                );
                                $on_callback(&pctx);
                                if pctx.canceled.load(Ordering::Relaxed) {
                                    $on_canceled(&pctx);
                                }
                            }
                        }
                    }
                    message::Type::PROCESS => {
                        if let Ok(process) = message::Process::parse_from_bytes(data) {
                            if let Ok(uuid) = Uuid::from_slice(process.get_uuid()) {
                                if let Some(ref mut context) =
                                    $contexts.lock().unwrap().get_mut(&uuid)
                                {
                                    context.buffer.extend(process.get_message());
                                    let pctx = ProgressContext::new(
                                        message::Type::PROCESS,
                                        uuid,
                                        context.total_length,
                                        context.buffer.len() as u64,
                                    );
                                    $on_callback(&pctx);
                                    if pctx.canceled.load(Ordering::Relaxed) {
                                        $on_canceled(&pctx);
                                    }
                                }
                            }
                        }
                    }
                    message::Type::END => {
                        if let Ok(end) = message::End::parse_from_bytes(data) {
                            if let Ok(uuid) = Uuid::from_slice(end.get_uuid()) {
                                if let Some(ref mut context) =
                                    $contexts.lock().unwrap().remove(&uuid)
                                {
                                    if context.compression
                                        == crate::protos::message::Compression::SNAPPY
                                    {
                                        let mut decomp = vec![];
                                        match snap::read::FrameDecoder::new(&context.buffer[..])
                                            .read_to_end(&mut decomp)
                                        {
                                            Ok(_) => {
                                                $on_callback(&ProgressContext::new(
                                                    message::Type::END,
                                                    uuid,
                                                    context.total_length,
                                                    context.total_length,
                                                ));
                                                $on_message(uuid, decomp);
                                            }
                                            Err(err) => {
                                                $on_error(Box::new(err));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                Err(err) => {
                    $on_error(Box::new(err));
                }
            },
            Err(err) => {
                $on_error(Box::new(err));
            }
        }
        false
    };
}
