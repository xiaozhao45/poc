use thiserror::Error;

/// 错误的数据层：变体携带结构化载荷。载荷在构造点经消息目录本地化，
/// Display 只回显载荷（无英文前缀叠加）；下列文案仅作 Debug/兜底语义说明。
#[derive(Debug, Error)]
pub enum PocError {
    #[error("not in a P.O.C. project (no .poc/store found; run `poc proj` to initialize)")]
    NotAProject,
    #[error("{0}")]
    Dirty(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    Msg(String),
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Conflict(String),
    #[error("storage error: {0}")]
    Database(#[from] redb::DatabaseError),
    #[error("storage error: {0}")]
    Transaction(#[from] redb::TransactionError),
    #[error("storage error: {0}")]
    Table(#[from] redb::TableError),
    #[error("storage error: {0}")]
    Storage(#[from] redb::StorageError),
    #[error("storage error: {0}")]
    Commit(#[from] redb::CommitError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Res<T> = Result<T, PocError>;
