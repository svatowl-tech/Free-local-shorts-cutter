// src-tauri/src/logger.rs
use tauri::{AppHandle, Emitter};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PipelineLogPayload {
    pub level: String,      // "info", "warn", "error", "debug"
    pub stage: String,      // "FFmpeg", "Whisper", "LLM", "Analyzer", "Render", "Pipeline", "System"
    pub message: String,
    pub timestamp: String,
}

/// Получение отформатированной временной метки HH:MM:SS.mmm
pub fn current_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = now.as_secs();
    let millis = now.subsec_millis();
    let hours = (total_secs / 3600) % 24;
    let minutes = (total_secs / 60) % 60;
    let seconds = total_secs % 60;
    format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
}

/// Отправка лога в системный Rust логгер (env_logger) и через Tauri событие во фронтенд
pub fn emit_log(app: &AppHandle, stage: &str, level: &str, message: &str) {
    let ts = current_timestamp();
    
    // Дублируем в env_logger
    match level {
        "error" => log::error!("[{}][{}] {}", ts, stage, message),
        "warn" => log::warn!("[{}][{}] {}", ts, stage, message),
        "debug" => log::debug!("[{}][{}] {}", ts, stage, message),
        _ => log::info!("[{}][{}] {}", ts, stage, message),
    }

    let payload = PipelineLogPayload {
        level: level.to_string(),
        stage: stage.to_string(),
        message: message.to_string(),
        timestamp: ts,
    };

    let _ = app.emit("pipeline-log", payload);
}

/// Утилита для безопасной очистки и разбиения stdout/stderr потоков на строки
pub fn emit_stream_lines(app: &AppHandle, stage: &str, level: &str, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            emit_log(app, stage, level, trimmed);
        }
    }
}
