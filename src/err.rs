use thiserror::Error;

#[derive(Debug, Error)]
pub enum PocError {
    #[error("不在 P.O.C. 项目内（未找到 .poc/store；用 `poc proj` 初始化）")]
    NotAProject,
    #[error("工作区不干净：{0}")]
    Dirty(String),
    #[error("未找到：{0}")]
    NotFound(String),
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    Msg(String),
    #[error("配置：{0}")]
    Config(String),
    #[error("存储错误：{0}")]
    Database(#[from] redb::DatabaseError),
    #[error("存储错误：{0}")]
    Transaction(#[from] redb::TransactionError),
    #[error("存储错误：{0}")]
    Table(#[from] redb::TableError),
    #[error("存储错误：{0}")]
    Storage(#[from] redb::StorageError),
    #[error("存储错误：{0}")]
    Commit(#[from] redb::CommitError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Res<T> = Result<T, PocError>;
