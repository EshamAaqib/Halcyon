//! One error type for the whole app; each kind maps to the HTTP status the UI expects.

use std::fmt;

#[derive(Debug, Clone)]
pub enum Error {
    /// Input the app or the keyboard cannot take (400).
    Bad(String),
    /// Unknown keyboard, page or file (404).
    NotFound(String),
    /// Something is already running, e.g. a display upload (409).
    Busy(String),
    /// The keyboard is missing, on the wrong connection, or lacks the feature (503).
    Device(String),
    /// A HID read or write failed; the registry retries once on a fresh handle (503).
    Io(String),
    /// A bug or an unreadable file; never a dropped connection (500).
    Internal(String),
}

impl Error {
    pub fn status(&self) -> u16 {
        match self {
            Error::Bad(_) => 400,
            Error::NotFound(_) => 404,
            Error::Busy(_) => 409,
            Error::Device(_) | Error::Io(_) => 503,
            Error::Internal(_) => 500,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Error::Bad(m)
            | Error::NotFound(m)
            | Error::Busy(m)
            | Error::Device(m)
            | Error::Io(m)
            | Error::Internal(m) => m,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Internal(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn bad<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Bad(msg.into()))
}
