// src-tauri/src/logger.rs
use tauri::{AppHandle, Emitter, Manager};
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
/// Фильтрует сырые технические строки прогресса FFmpeg и внутренние дампы метаданных llama-cli, чтобы не перегружать IPC и буфер консоли
pub fn emit_stream_lines(app: &AppHandle, stage: &str, level: &str, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    for line in text.split(['\n', '\r']) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Пропускаем частые технические тики прогресса FFmpeg, которые уже обрабатываются через *-progress события
        if (trimmed.starts_with("frame=") || trimmed.starts_with("size=")) && trimmed.contains("time=") && trimmed.contains("speed=") {
            continue;
        }
        if is_noisy_stream_line(stage, trimmed) {
            continue;
        }
        emit_log(app, stage, level, trimmed);
    }
}

fn is_noisy_stream_line(stage: &str, line: &str) -> bool {
    if stage == "LLM" {
        const LLAMA_NOISE_PREFIXES: &[&str] = &[
            "Log start",
            "Log end",
            "main: build =",
            "main: built with",
            "main: seed  =",
            "ggml_cuda_init:",
            "Device 0:",
            "Device 1:",
            "llama_model_loader:",
            "llm_load_print_meta:",
            "llm_load_vocab:",
            "llm_load_tensors:",
            "llama_new_context_with_model:",
            "llama_kv_cache_init:",
            "system_info:",
            "sampling:",
            "sampling order:",
            "CFG ->",
            "generate: n_ctx",
            "llama_print_timings:",
        ];
        if LLAMA_NOISE_PREFIXES.iter().any(|p| line.starts_with(p)) {
            return true;
        }
        if line.chars().all(|c| c == '.' || c.is_whitespace()) {
            return true;
        }
        if line.contains("GGGGGGGG") || line.contains('\u{FFFD}') {
            return true;
        }
        // Отсекаем поврежденный мультиязычный мусор токенизатора (CJK / арабская вязь при генерации числового JSON)
        let exotic_chars = line
            .chars()
            .filter(|&c| {
                ('\u{4E00}'..='\u{9FFF}').contains(&c)
                    || ('\u{0600}'..='\u{06FF}').contains(&c)
                    || ('\u{0900}'..='\u{097F}').contains(&c)
            })
            .count();
        if exotic_chars >= 2 {
            return true;
        }
    }

    if stage == "Render" || stage == "FFmpeg" {
        const FFMPEG_NOISE_PREFIXES: &[&str] = &[
            "ffmpeg version",
            "built with gcc",
            "configuration: --",
            "libavutil",
            "libavcodec",
            "libavformat",
            "libavdevice",
            "libavfilter",
            "libswscale",
            "libswresample",
            "libpostproc",
            "[Parsed_subtitles_",
            "[libx264 @",
            "[aac @",
            "Metadata:",
            "major_brand",
            "minor_version",
            "compatible_brands",
            "creation_time",
            "handler_name",
            "vendor_id",
            "encoder         :",
            "Side data:",
            "cpb: bitrate",
        ];
        if FFMPEG_NOISE_PREFIXES.iter().any(|p| line.starts_with(p)) {
            return true;
        }
    }

    false
}

/// Формирует расширенный PATH с папками ресурсов для корректной загрузки DLL (CUDA, whisper.dll, ggml.dll)
pub fn build_sidecar_env_path(app: &AppHandle) -> String {
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut dirs = Vec::new();

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            dirs.push(exe_dir.to_string_lossy().to_string());
            dirs.push(exe_dir.join("resources").to_string_lossy().to_string());
        }
    }

    if let Ok(res_dir) = app.path().resource_dir() {
        dirs.push(res_dir.to_string_lossy().to_string());
        dirs.push(res_dir.join("resources").to_string_lossy().to_string());
    }

    if let Ok(existing_path) = std::env::var("PATH") {
        dirs.push(existing_path);
    }

    dirs.join(sep)
}
