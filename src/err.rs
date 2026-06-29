use std::{fmt::Display, path::PathBuf};

#[derive(Debug)]
pub enum Init {
    ProjectAlreadyExists,
    Io(PathBuf, std::io::Error),
}

impl Display for Init {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Init::ProjectAlreadyExists => write!(f, "project already exists"),
            Init::Io(path, error) => {
                write!(f, "encountered error with {}: {}", path.display(), error)
            }
        }
    }
}

#[derive(Debug)]
pub enum Reason {
    Io(PathBuf, std::io::Error),
    IoString(String, std::io::Error),
    Json(PathBuf, serde_json::Error),
    JsonString(String, serde_json::Error),
    Other(String),
}

impl Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::Io(path, error) => {
                write!(f, "filesystem error with {}: {}", path.display(), error)
            }
            Reason::IoString(string, error) => {
                write!(f, "{}: {}", string, error)
            }
            Reason::Json(path, error) => {
                write!(f, "failed to parse JSON at {}: {}", path.display(), error)
            }
            Reason::JsonString(string, error) => {
                write!(f, "{}: {}", string, error)
            }
            Reason::Other(string) => {
                write!(f, "{}", string)
            }
        }
    }
}
