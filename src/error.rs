use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Connection error")]
    ConnectionError(),

    #[error("Invalid parameters")]
    InvalidParameters(),

    #[error("Internal error")]
    InternalError(Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    pub fn cause(&self) -> &dyn std::error::Error {
        if let Error::InternalError(err) = self {
            err.as_ref()
        } else {
            self
        }
    }
}
