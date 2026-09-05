// src-tauri/src/error.rs
use serde::{Serialize, Serializer};

/// Единый тип архитектурных ошибок бэкенда.
/// Автоматически преобразует любые anyhow::Error и std::io::Error.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),

    #[error("Системная ошибка ввода-вывода: {0}")]
    Io(#[from] std::io::Error),

    #[error("Ошибка при работе с FFmpeg/Whisper Sidecar: {0}")]
    Pipeline(String),
}

/// Реализация: Tauri требует, чтобы любые типы ошибок сериализовывались.
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
