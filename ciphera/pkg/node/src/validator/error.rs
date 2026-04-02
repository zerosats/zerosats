use crate::errors::Error as NodeError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("node error")]
    Node(#[from] NodeError),

    #[error("contract error")]
    Contract(#[from] contracts::Error),
}

pub(super) type Result<T, E = Error> = std::result::Result<T, E>;
