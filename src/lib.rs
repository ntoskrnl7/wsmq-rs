pub mod client;
pub mod server;
pub mod response;
pub mod protos;
pub mod error;
pub type Result<T> = std::result::Result<T, error::Error>;

mod shared;
mod message;
mod callbacks;
