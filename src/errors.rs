use ashpd::Error;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, VenbindError>;

#[derive(Debug, Error)]
pub enum VenbindError {
    #[error("Something went wrong with libuiohook")] // TODO: better log
    LibUIOHookError,
    #[error("{0}")]
    Message(String),
    #[cfg(all(target_os = "linux"))]
    #[error("ashpd error: {0}")]
    AshPdError(ashpd::Error),
}

#[cfg(all(target_os = "linux"))]
impl From<ashpd::Error> for VenbindError {
    fn from(value: Error) -> Self {
        VenbindError::AshPdError(value)
    }
}
