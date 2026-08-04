use serde::Serialize;

pub type Result<T> = std::result::Result<T, GitError>;

#[derive(Debug, Serialize, Clone)]
pub struct GitError {
    pub code: String,
    pub message: String,
}

impl GitError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

impl From<git2::Error> for GitError {
    fn from(err: git2::Error) -> Self {
        Self::new("git2", format!("{:?}: {}", err.code(), err.message()))
    }
}

impl From<std::io::Error> for GitError {
    fn from(err: std::io::Error) -> Self {
        Self::new("io", err.to_string())
    }
}
