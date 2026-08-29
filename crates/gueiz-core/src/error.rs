use std::error::Error;
use std::fmt::Display;

#[derive(Debug)]
pub enum GueizCoreError {
    NotSupportedError(String),
}

impl Display for GueizCoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSupportedError(err) => {
                write!(f, "The requested operation is not supported. {}", err)
            }
        }
    }
}

impl Error for GueizCoreError {}