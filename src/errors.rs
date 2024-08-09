use thiserror::Error;

pub type Result<T> = std::result::Result<T, VenkeybindError>;

#[derive(Debug, Error)]
pub enum VenkeybindError {
    #[error("Something went wrong with libuiohook")] // TODO: better log
    LibUIOHookError,
    #[error("{0}")]
    Message(String)
}