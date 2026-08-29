use std::error::Error;
use std::fmt::{Display, Formatter};
use winit::error::{EventLoopError, RequestError};

use gueiz_core::error::GueizCoreError;

use crate::window::WindowId;

#[derive(Debug)]
pub enum GueizWindowError {
    ApplicationError(EventLoopError),
    WindowCreationError(RequestError),
    WindowRequestError(RequestError),
    NotSupportedError(GueizCoreError),
    UnknownWindowIdError(WindowId),
}

impl Display for GueizWindowError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApplicationError(event_loop_error) => {
                event_loop_error.fmt(f)
            }
            Self::WindowCreationError(request_error) => {
                request_error.fmt(f)
            }
            Self::WindowRequestError(request_error) => {
                request_error.fmt(f)
            }
            Self::NotSupportedError(gueiz_core_error) => {
                gueiz_core_error.fmt(f)
            }
            Self::UnknownWindowIdError(window_id) => {
                write!(f, "Unknown window id: {}", window_id)
            }
        }
    }
}

impl Error for GueizWindowError {}
