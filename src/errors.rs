use thiserror::Error;

pub type Result<T> = std::result::Result<T, VenbindError>;

#[derive(Debug, Error)]
pub enum VenbindError {
    #[error("Something went wrong with libuiohook")] // TODO: better log
    LibUIOHookError,
    #[error("{0}")]
    Message(String)
}
