use std::{io, path::PathBuf};

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

pub(crate) enum ProcessingError {
    IoError((PathBuf, io::Error)),
    DataError((&'static str, String, Error)),
}
