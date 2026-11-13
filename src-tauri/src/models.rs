// src-tauri/src/models.rs
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use serde::{Deserialize, Serialize};
use crate::error::{AppResult, AppError};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub filename: String,
    pub url: String,
    pub size_mb: f64,
    pub description: String,
    pub category: String, // "whisper" | "llm" | "vision"
    pub params: String,   // e.g. "74M", "0.5B", "1.5B", "3B"
    pub exists: bool,
    pub local_path: String,
    #[serde(default)]
    pub mmproj_filename: Option<String>,
    #[serde(default)]
    pub mmproj_url: Option<String>,
    #[serde(default)]
    pub mmproj_size_mb: Option<f64>,
}

impl ModelInfo {
    pub fn standard(
        id: &str,
        name: &str,
        filename: &str,
        url: &str,
        size_mb: f64,
        description: &str,
        category: &str,
        params: &str,
    ) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            filename: filename.to_string(),
            url: url.to_string(),
            size_mb,
            description: description.to_string(),
            category: category.to_string(),
            params: params.to_string(),
            exists: false,
            local_path: String::new(),
            mmproj_filename: None,
            mmproj_url: None,
            mmproj_size_mb: None,
        }
    }

    pub fn vision(
        id: &str,
        name: &str,
        filename: &str,
        url: &str,
        size_mb: f64,
        description: &str,
        params: &str,
        mmproj_filename: &str,
        mmproj_url: &str,
        mmproj_size_mb: f64,
    ) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            filename: filename.to_string(),
            url: url.to_string(),
            size_mb,
            description: description.to_string(),
            category: "vision".to_string(),
            params: params.to_string(),
            exists: false,
            local_path: String::new(),
            mmproj_filename: Some(mmproj_filename.to_string()),
            mmproj_url: Some(mmproj_url.to_string()),
            mmproj_size_mb: Some(mmproj_size_mb),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct DownloadProgressPayload {
    pub model_id: String,
    pub percent: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub status: String, // "downloading", "completed", "failed", "cancelled"
}

// Глобальное хранилище активных загрузок, чтобы можно было отменять или предотвращать дублирование
pub struct ModelsState {
    pub active_downloads: Arc<Mutex<std::collections::HashSet<String>>>,
}

pub fn init_models_state() -> ModelsState {
    ModelsState {
        active_downloads: Arc::new(Mutex::new(std::collections::HashSet::new())),
    }
}

fn is_dir_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let test_file = dir.join(".write_probe");
    match std::fs::write(&test_file, b"ok") {
        Ok(_) => {
            let _ = std::fs::remove_file(&test_file);
            true
        }
        Err(_) => false,
    }
}

/// Возвращает гарантированно доступную для записи папку хранения моделей
pub fn get_models_dir(app_handle: &AppHandle) -> PathBuf {
    // 1. Приоритет: системная пользовательская директория AppData (всегда доступна для записи и удаления на Windows/macOS/Linux)
    if let Ok(app_data) = app_handle.path().app_data_dir() {
        let data_models = app_data.join("models");
        if is_dir_writable(&data_models) {
            return data_models;
        }
    }

    // 2. Папка models рядом с исполняемым файлом (для Portable-режима, если есть права на запись)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let portable_models = exe_dir.join("models");
            if is_dir_writable(&portable_models) {
                return portable_models;
            }
        }
    }

    // 3. В рабочей директории (для dev-режима)
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_models = cwd.join("models");
        if is_dir_writable(&cwd_models) {
            return cwd_models;
        }
    }

    // 4. Fallback
    if let Ok(app_data) = app_handle.path().app_data_dir() {
        return app_data.join("models");
    }

    PathBuf::from("models")
}

/// Возвращает список всех потенциальных директорий, где могут лежать модели
fn get_candidate_model_dirs(app_handle: &AppHandle) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(app_data) = app_handle.path().app_data_dir() {
        dirs.push(app_data.join("models"));
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            dirs.push(exe_dir.join("models"));
            dirs.push(exe_dir.to_path_buf());
        }
    }
    if let Ok(res_dir) = app_handle.path().resource_dir() {
        dirs.push(res_dir.join("models"));
        dirs.push(res_dir);
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("models"));
        dirs.push(cwd.join("src-tauri").join("models"));
    }
    dirs
}

fn expand_model_aliases(filename: &str) -> Vec<String> {
    let mut search_filenames = vec![filename.to_string()];
    let lower = filename.to_lowercase();

    if lower.contains("llava-phi-3") || lower.contains("llava_phi_3") {
        search_filenames.push("llava-phi-3-mini-int4.gguf".to_string());
        search_filenames.push("llava-phi-3-mini-f16.gguf".to_string());
    } else if lower.contains("moondream") {
        search_filenames.push("moondream2-q4_k.gguf".to_string());
        search_filenames.push("moondream2-text-model-f16.gguf".to_string());
        search_filenames.push("moondream2-text-model.gguf".to_string());
        search_filenames.push("moondream2.gguf".to_string());
    } else if lower.contains("qwen-0.5b") || lower.contains("qwen2.5-0.5b") {
        search_filenames.push("qwen2.5-0.5b-instruct-q4_k_m.gguf".to_string());
        search_filenames.push("qwen-0.5b-chat.gguf".to_string());
    } else if lower.contains("qwen-1.5b") || lower.contains("qwen2.5-1.5b") {
        search_filenames.push("qwen-1.5b-chat.gguf".to_string());
        search_filenames.push("qwen2.5-1.5b-instruct-q4_k_m.gguf".to_string());
    } else if lower.contains("qwen-3b") || lower.contains("qwen2.5-3b") {
        search_filenames.push("qwen2.5-3b-instruct-q4_k_m.gguf".to_string());
        search_filenames.push("qwen-3b-chat.gguf".to_string());
    } else if lower.contains("vikhr-llama-3.2-1b") || lower.contains("vikhr-1b") {
        search_filenames.push("Vikhr-Llama-3.2-1B-Q4_K_M.gguf".to_string());
    } else if lower.contains("vikhr-qwen-2.5-1.5b") || lower.contains("vikhr-1.5b") {
        search_filenames.push("Vikhr-Qwen-2.5-1.5B-Instruct.Q4_K_M.gguf".to_string());
    } else if lower.contains("vikhr-gemma-2b") || lower.contains("vikhr-2b") {
        search_filenames.push("Vikhr-Gemma-2B-instruct-Q4_K_M.gguf".to_string());
    } else if lower.contains("llama-3.2-1b") {
        search_filenames.push("Llama-3.2-1B-Instruct-Q4_K_M.gguf".to_string());
    } else if lower.contains("llama-3.2-3b") {
        search_filenames.push("Llama-3.2-3B-Instruct-Q4_K_M.gguf".to_string());
    } else if lower.contains("gemma-2-2b") {
        search_filenames.push("gemma-2-2b-it-Q4_K_M.gguf".to_string());
    } else if lower.contains("small-q5") {
        search_filenames.push("ggml-small-q5_1.bin".to_string());
    } else if lower.contains("medium-q5") {
        search_filenames.push("ggml-medium-q5_0.bin".to_string());
    } else if lower.contains("large-v3-turbo") {
        search_filenames.push("ggml-large-v3-turbo-q5_0.bin".to_string());
    } else if lower.contains("base") {
        search_filenames.push("ggml-base.bin".to_string());
    } else if lower.contains("tiny") {
        search_filenames.push("ggml-tiny.bin".to_string());
    } else if lower.contains("small") {
        search_filenames.push("ggml-small.bin".to_string());
    }

    search_filenames.dedup();
    search_filenames
}

fn is_valid_model_file(path: &Path) -> bool {
    if let Ok(meta) = std::fs::metadata(path) {
        // Файл модели должен быть не менее 1 МБ (защита от пустых заглушек и битых обрывков)
        meta.is_file() && meta.len() > 1_000_000
    } else {
        false
    }
}

pub fn resolve_model_path(app_handle: &AppHandle, model_name_or_path: &str) -> Option<PathBuf> {
    let trimmed = model_name_or_path.trim();
    if trimmed.is_empty() {
        return None;
    }

    let p = PathBuf::from(trimmed);
    if is_valid_model_file(&p) {
        return Some(p);
    }
    let filename = p.file_name().and_then(|f| f.to_str()).unwrap_or(trimmed);
    let search_filenames = expand_model_aliases(filename);
    let candidate_dirs = get_candidate_model_dirs(app_handle);

    for name in &search_filenames {
        for dir in &candidate_dirs {
            let candidate = dir.join(name);
            if is_valid_model_file(&candidate) {
                log::info!("Успешно найдена модель: {}", candidate.display());
                return Some(candidate);
            }
        }
    }

    None
}

/// Поиск парного проектора mmproj для Vision-модели (Moondream2, LLaVA Phi-3 Mini)
pub fn resolve_mmproj_for_model(app_handle: &AppHandle, model_path: &Path) -> Option<PathBuf> {
    let parent = model_path.parent();
    let model_stem = model_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let lower_stem = model_stem.to_lowercase();

    let mut mmproj_names = Vec::new();
    if lower_stem.contains("llava-phi-3") || lower_stem.contains("llava") {
        mmproj_names.push("llava-phi-3-mini-mmproj-f16.gguf".to_string());
        mmproj_names.push("mmproj-model-f16.gguf".to_string());
    } else if lower_stem.contains("moondream") {
        mmproj_names.push("moondream2-mmproj-f16.gguf".to_string());
        mmproj_names.push("mmproj-moondream2.gguf".to_string());
    }
    mmproj_names.push("mmproj-model-f16.gguf".to_string());
    mmproj_names.push("mmproj.gguf".to_string());

    // 1. Проверяем в той же папке, где лежит сама модель
    if let Some(p) = parent {
        for name in &mmproj_names {
            let candidate = p.join(name);
            if is_valid_model_file(&candidate) {
                log::info!("Найден парный mmproj проектор: {}", candidate.display());
                return Some(candidate);
            }
        }
    }

    // 2. Проверяем во всех стандартных папках моделей
    for name in &mmproj_names {
        if let Some(found) = resolve_model_path(app_handle, name) {
            log::info!("Найден mmproj проектор в системных путях: {}", found.display());
            return Some(found);
        }
    }

    None
}

/// Находит полнофункциональную связку Vision-модели и её проектора mmproj: (model_path, mmproj_path),
/// совместимую с движком llama-vision (llava-cli: архитектуры llama / phi2 / phi3 + clip mmproj)
pub fn resolve_vision_model_and_projector(
    app_handle: &AppHandle,
    preferred: Option<&str>,
) -> Option<(PathBuf, PathBuf)> {
    if let Some(pref) = preferred {
        let trimmed = pref.trim();
        let lower_pref = trimmed.to_lowercase();
        // Игнорируем устаревшие qwen2-vl веса (архитектура qwen2vl не поддерживается бинарником llama-llava-cli)
        if !trimmed.is_empty() && !lower_pref.contains("qwen2-vl") && !lower_pref.contains("qwen-vl") {
            if let Some(m_path) = resolve_model_path(app_handle, trimmed) {
                if let Some(proj) = resolve_mmproj_for_model(app_handle, &m_path) {
                    return Some((m_path, proj));
                }
            }
        }
    }

    for m in get_all_models_config()
        .into_iter()
        .filter(|m| m.category == "vision")
    {
        if let Some(m_path) = resolve_model_path(app_handle, &m.filename) {
            if let Some(proj) = resolve_mmproj_for_model(app_handle, &m_path) {
                return Some((m_path, proj));
            }
        }
    }

    None
}

/// Ищет сначала указанную Whisper-модель, а если её нет — лучшую из установленных Whisper-моделей (от старших к младшим)
pub fn resolve_any_whisper_model(app_handle: &AppHandle, preferred: Option<&str>) -> Option<PathBuf> {
    if let Some(pref) = preferred {
        // Если передан дефолтный `models/ggml-base.bin`, проверяем сначала, не скачана ли более точная модель (Small / Medium / Large-v3-Turbo)
        let is_default_base = pref == "models/ggml-base.bin" || pref == "ggml-base.bin";
        if !is_default_base {
            if let Some(found) = resolve_model_path(app_handle, pref) {
                return Some(found);
            }
        }
    }
    for m in get_all_models_config()
        .into_iter()
        .filter(|m| m.category == "whisper")
        .rev()
    {
        if let Some(found) = resolve_model_path(app_handle, &m.filename) {
            return Some(found);
        }
    }
    None
}

/// Ищет сначала указанную текстовую GGUF-модель, а если передан `auto` или старый дефолт `models/qwen-1.5b-chat.gguf` —
/// проверяет сначала русскоязычные модели Vikhr и более мощные установленные модели (от 3B к 0.5B)
pub fn resolve_any_llm_model(app_handle: &AppHandle, preferred: Option<&str>) -> Option<PathBuf> {
    if let Some(pref) = preferred {
        let trimmed = pref.trim();
        let is_legacy_or_auto =
            trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") || trimmed == "models/qwen-1.5b-chat.gguf";
        if !is_legacy_or_auto {
            if let Some(found) = resolve_model_path(app_handle, trimmed) {
                return Some(found);
            }
        }
    }
    // Сначала проверяем специализированные русскоязычные модели Vikhr (2B -> 1.5B -> 1B)
    let russian_vikhr_priority = [
        "Vikhr-Gemma-2B-instruct-Q4_K_M.gguf",
        "Vikhr-Qwen-2.5-1.5B-Instruct.Q4_K_M.gguf",
        "Vikhr-Llama-3.2-1B-Q4_K_M.gguf",
    ];
    for vikhr_file in &russian_vikhr_priority {
        if let Some(found) = resolve_model_path(app_handle, vikhr_file) {
            return Some(found);
        }
    }
    for m in get_all_models_config()
        .into_iter()
        .filter(|m| m.category == "llm")
        .rev()
    {
        if let Some(found) = resolve_model_path(app_handle, &m.filename) {
            return Some(found);
        }
    }
    None
}

/// Находит выбранную пользователем GGUF-модель (или приоритетную русскоязычную модель серии Vikhr 2B / 1.5B / 1B при режиме `auto`)
/// для 2-й финальной проверки и финального SMM-копирайтинга.
pub fn resolve_russian_verifier_llm_model(
    app_handle: &AppHandle,
    preferred: Option<&str>,
) -> Option<PathBuf> {
    if let Some(pref) = preferred {
        let trimmed = pref.trim();
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("auto") {
            if let Some(found) = resolve_model_path(app_handle, trimmed) {
                return Some(found);
            }
        }
    }
    // Приоритет русскоязычных моделей Vikhr (от старшей 2B к быстрой 1B)
    let russian_vikhr_priority = [
        "Vikhr-Gemma-2B-instruct-Q4_K_M.gguf",
        "Vikhr-Qwen-2.5-1.5B-Instruct.Q4_K_M.gguf",
        "Vikhr-Llama-3.2-1B-Q4_K_M.gguf",
    ];
    for vikhr_file in &russian_vikhr_priority {
        if let Some(found) = resolve_model_path(app_handle, vikhr_file) {
            return Some(found);
        }
    }
    // Если ни одна модель Vikhr ещё не скачана, используем любую лучшую доступную GGUF-модель
    resolve_any_llm_model(app_handle, preferred)
}

pub fn get_all_models_config() -> Vec<ModelInfo> {
    vec![
        // ==================== WHISPER ASR (РАСПОЗНАВАНИЕ РЕЧИ И СУБТИТРЫ) ====================
        ModelInfo::standard(
            "whisper-tiny",
            "Whisper Tiny (Ультрабыстрая)",
            "ggml-tiny.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
            75.0,
            "Сверхбыстрая модель для слабых ПК и мгновенной черновой транскрипции (75 МБ)",
            "whisper",
            "39M",
        ),
        ModelInfo::standard(
            "whisper-base",
            "Whisper Base (Рекомендуется)",
            "ggml-base.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
            142.0,
            "Оптимальный баланс скорости и точности для русской и английской речи (142 МБ)",
            "whisper",
            "74M",
        ),
        ModelInfo::standard(
            "whisper-small-q5",
            "Whisper Small Q5 (Легкая Pro)",
            "ggml-small-q5_1.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin",
            181.0,
            "Квантованная Small-модель: высокая точность распознавания при малом весе (181 МБ)",
            "whisper",
            "244M Q5",
        ),
        ModelInfo::standard(
            "whisper-small",
            "Whisper Small (Высокая точность)",
            "ggml-small.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
            466.0,
            "Отличное качество для аниме, стримов, сложной дикции и фонового шума (466 МБ)",
            "whisper",
            "244M",
        ),
        ModelInfo::standard(
            "whisper-medium-q5",
            "Whisper Medium Q5 (Макс. точность RU)",
            "ggml-medium-q5_0.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium-q5_0.bin",
            514.0,
            "Квантованная Medium-модель: студийная точность русского языка без перегрузки ОЗУ (514 МБ)",
            "whisper",
            "769M Q5",
        ),
        ModelInfo::standard(
            "whisper-large-v3-turbo-q5",
            "Whisper Large-v3 Turbo Q5",
            "ggml-large-v3-turbo-q5_0.bin",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
            547.0,
            "Ускоренная флагманская модель OpenAI Large-v3 Turbo в компактном формате Q5 (547 МБ)",
            "whisper",
            "809M Q5",
        ),

        // ==================== LLM (АНАЛИЗ ДИАЛОГОВ И ОТБОР ХАЙЛАЙТОВ — ДО 3B) ====================
        ModelInfo::standard(
            "qwen-0.5b-chat",
            "Qwen 2.5 0.5B Instruct (Ультралегкая)",
            "qwen2.5-0.5b-instruct-q4_k_m.gguf",
            "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
            469.0,
            "Мгновенный анализ реплик на любых ПК и ноутбуках даже без дискретной видеокарты (469 МБ)",
            "llm",
            "0.5B",
        ),
        ModelInfo::standard(
            "llama-3.2-1b",
            "Llama 3.2 1B Instruct (Быстрая)",
            "Llama-3.2-1B-Instruct-Q4_K_M.gguf",
            "https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf",
            770.0,
            "Легкая открытая модель Meta Llama 3.2 для быстрой оценки динамики диалогов (770 МБ)",
            "llm",
            "1B",
        ),
        ModelInfo::standard(
            "vikhr-llama-3.2-1b",
            "Vikhr Llama 3.2 1B (RU Сленг и Юмор)",
            "Vikhr-Llama-3.2-1B-Q4_K_M.gguf",
            "https://huggingface.co/Vikhrmodels/Vikhr-Llama-3.2-1B-instruct-GGUF/resolve/main/Vikhr-Llama-3.2-1B-Q4_K_M.gguf",
            808.0,
            "Русскоязычная модель Vikhr 1B: специально обучена на живой русской речи, сленге, иронии и панчлайнах (808 МБ)",
            "llm",
            "1B RU",
        ),
        ModelInfo::standard(
            "vikhr-qwen-2.5-1.5b",
            "Vikhr Qwen 2.5 1.5B (RU Диалоги и Юмор)",
            "Vikhr-Qwen-2.5-1.5B-Instruct.Q4_K_M.gguf",
            "https://huggingface.co/QuantFactory/Vikhr-Qwen-2.5-1.5B-Instruct-GGUF/resolve/main/Vikhr-Qwen-2.5-1.5B-Instruct.Q4_K_M.gguf",
            986.0,
            "Русскоязычная дообученная Qwen 2.5 1.5B от проекта Vikhr: отличное чувство юмора, сарказма и сюжетного накала (986 МБ)",
            "llm",
            "1.5B RU",
        ),
        ModelInfo::standard(
            "qwen-1.5b-chat",
            "Qwen 2.5 1.5B Instruct (Рекомендуется)",
            "qwen-1.5b-chat.gguf",
            "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
            1065.0,
            "Оптимальный выбор: отличное понимание русского контекста, юмора и эмоций (1.06 ГБ)",
            "llm",
            "1.5B",
        ),
        ModelInfo::standard(
            "gemma-2-2b",
            "Google Gemma 2 2B Instruct",
            "gemma-2-2b-it-Q4_K_M.gguf",
            "https://huggingface.co/bartowski/gemma-2-2b-it-GGUF/resolve/main/gemma-2-2b-it-Q4_K_M.gguf",
            1629.0,
            "Модель от Google (2B): точное выделение смысловых панчлайнов и ярких реплик (1.63 ГБ)",
            "llm",
            "2B",
        ),
        ModelInfo::standard(
            "vikhr-gemma-2b",
            "Vikhr Gemma 2B Instruct (RU Профи)",
            "Vikhr-Gemma-2B-instruct-Q4_K_M.gguf",
            "https://huggingface.co/Vikhrmodels/Vikhr-Gemma-2B-instruct-GGUF/resolve/main/Vikhr-Gemma-2B-instruct-Q4_K_M.gguf",
            1630.0,
            "Усиленная русскоязычная модель Vikhr 2B (на базе Gemma 2): глубокое понимание русских идиом, шуток и кульминаций (1.63 ГБ)",
            "llm",
            "2B RU",
        ),
        ModelInfo::standard(
            "llama-3.2-3b",
            "Llama 3.2 3B Instruct (Продвинутая)",
            "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
            "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf",
            1926.0,
            "Продвинутая 3B модель с глубоким анализом драматургии и сюжетных поворотов (1.93 ГБ)",
            "llm",
            "3B",
        ),
        ModelInfo::standard(
            "qwen-3b-chat",
            "Qwen 2.5 3B Instruct (Максимум 3B)",
            "qwen2.5-3b-instruct-q4_k_m.gguf",
            "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
            2007.0,
            "Максимальное качество анализа русскоязычных диалогов и сленга в пределах 3B (2.0 ГБ)",
            "llm",
            "3B",
        ),

        // ==================== VISION AI (ВИЗУАЛЬНЫЙ АНАЛИЗ КАДРОВ — МОДЕЛЬ + ПРОЕКТОР MMPROJ) ====================
        ModelInfo::vision(
            "moondream2",
            "Moondream 2 Vision Q4_K (Быстрая + mmproj)",
            "moondream2-q4_k.gguf",
            "https://huggingface.co/salivosa/moondream2-gguf/resolve/main/moondream2-q4_k.gguf",
            877.0,
            "Быстрая и легкая Vision-нейросеть (архитектура Phi-2 + CLIP), нативно совместимая с llama-vision: сканирует кадры видео без перегрузки ПК (1.74 ГБ комплект с mmproj).",
            "1.8B + mmproj",
            "moondream2-mmproj-f16.gguf",
            "https://huggingface.co/salivosa/moondream2-gguf/resolve/main/moondream2-mmproj-f16.gguf",
            868.0,
        ),
        ModelInfo::vision(
            "llava-phi-3-mini",
            "LLaVA Phi-3 Mini Vision INT4 (Флагман Pro + mmproj)",
            "llava-phi-3-mini-int4.gguf",
            "https://huggingface.co/xtuner/llava-phi-3-mini-gguf/resolve/main/llava-phi-3-mini-int4.gguf",
            2211.0,
            "Флагманская мультимодальная модель LLaVA (XTuner Phi-3 Mini + CLIP): высокая точность распознавания сцен, эмоций персонажей и экшена (2.79 ГБ комплект с mmproj).",
            "3.8B INT4 + mmproj",
            "llava-phi-3-mini-mmproj-f16.gguf",
            "https://huggingface.co/xtuner/llava-phi-3-mini-gguf/resolve/main/llava-phi-3-mini-mmproj-f16.gguf",
            579.0,
        ),
    ]
}

#[tauri::command]
pub async fn get_models_status(app_handle: AppHandle) -> AppResult<Vec<ModelInfo>> {
    let mut configured_models = get_all_models_config();

    for model in &mut configured_models {
        if let Some(resolved_path) = resolve_model_path(&app_handle, &model.filename) {
            let mmproj_ready = if let Some(ref proj_name) = model.mmproj_filename {
                resolve_model_path(&app_handle, proj_name).is_some()
            } else {
                true
            };

            if mmproj_ready {
                model.exists = true;
                model.local_path = resolved_path.to_string_lossy().to_string();
            } else {
                model.exists = false;
                model.local_path = String::new();
            }
        } else {
            model.exists = false;
            model.local_path = String::new();
        }
    }

    Ok(configured_models)
}

#[tauri::command]
pub async fn delete_model(
    app_handle: AppHandle,
    models_state: State<'_, ModelsState>,
    model_id: String,
) -> AppResult<String> {
    log::info!("Запрос на удаление модели: {}", model_id);

    let models = get_all_models_config();
    let model = models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("Неизвестный ID модели: {}", model_id)))?;

    // Если модель сейчас скачивается — прерываем загрузку
    {
        let mut active = models_state.active_downloads.lock().await;
        active.remove(&model_id);
    }

    let mut aliases = expand_model_aliases(&model.filename);
    if let Some(ref proj) = model.mmproj_filename {
        aliases.extend(expand_model_aliases(proj));
        aliases.push(proj.clone());
    }
    aliases.dedup();

    let candidate_dirs = get_candidate_model_dirs(&app_handle);
    let mut deleted_count = 0usize;
    let mut last_err: Option<std::io::Error> = None;

    for dir in &candidate_dirs {
        for fname in &aliases {
            let file_path = dir.join(fname);
            if file_path.exists() {
                match tokio::fs::remove_file(&file_path).await {
                    Ok(_) => {
                        log::info!("Удален файл модели: {}", file_path.display());
                        deleted_count += 1;
                    }
                    Err(e) => {
                        log::warn!("Не удалось удалить файл {}: {}", file_path.display(), e);
                        last_err = Some(e);
                    }
                }
            }
            let part_path = dir.join(format!("{}.download", fname));
            if part_path.exists() {
                let _ = tokio::fs::remove_file(&part_path).await;
            }
        }
    }

    if deleted_count == 0 {
        if let Some(e) = last_err {
            return Err(AppError::Anyhow(anyhow::anyhow!(
                "Не удалось удалить файл модели (возможно, он занят процессом): {}",
                e
            )));
        }
    }

    Ok(format!("Модель {} успешно удалена с диска", model.name))
}

#[tauri::command]
pub async fn cancel_model_download(
    app_handle: AppHandle,
    models_state: State<'_, ModelsState>,
    model_id: String,
) -> AppResult<String> {
    log::info!("Отмена скачивания модели: {}", model_id);
    {
        let mut active = models_state.active_downloads.lock().await;
        active.remove(&model_id);
    }

    let _ = app_handle.emit(
        "model-download-progress",
        DownloadProgressPayload {
            model_id: model_id.clone(),
            percent: 0.0,
            downloaded_bytes: 0,
            total_bytes: 0,
            status: "cancelled".to_string(),
        },
    );

    Ok(format!("Загрузка модели {} отменена", model_id))
}

#[tauri::command]
pub async fn download_model(
    app_handle: AppHandle,
    models_state: State<'_, ModelsState>,
    model_id: String,
) -> AppResult<String> {
    log::info!("Запуск скачивания модели: {}", model_id);

    let models = get_all_models_config();
    let model = models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("Неизвестный ID модели: {}", model_id)))?;

    {
        let mut active = models_state.active_downloads.lock().await;
        if active.contains(&model_id) {
            return Err(AppError::Anyhow(anyhow::anyhow!("Загрузка этой модели уже запущена")));
        }
        active.insert(model_id.clone());
    }

    let active_downloads_clone = Arc::clone(&models_state.active_downloads);
    let model_id_clone = model_id.clone();
    let app_clone = app_handle.clone();
    let model_filename = model.filename.clone();
    let model_url = model.url.clone();
    let mmproj_filename = model.mmproj_filename.clone();
    let mmproj_url = model.mmproj_url.clone();

    tokio::spawn(async move {
        let res = perform_download_bundle(
            &app_clone,
            &model_id_clone,
            &model_filename,
            &model_url,
            mmproj_filename.as_deref(),
            mmproj_url.as_deref(),
            &active_downloads_clone,
        )
        .await;

        let was_cancelled = {
            let mut active = active_downloads_clone.lock().await;
            let still_active = active.remove(&model_id_clone);
            !still_active
        };

        match res {
            Ok(path) => {
                log::info!("Модель {} успешно сохранена: {}", model_id_clone, path);
                let _ = app_clone.emit(
                    "model-download-progress",
                    DownloadProgressPayload {
                        model_id: model_id_clone,
                        percent: 100.0,
                        downloaded_bytes: 0,
                        total_bytes: 0,
                        status: "completed".to_string(),
                    },
                );
            }
            Err(e) => {
                if was_cancelled || e.to_string().contains("Отменено") {
                    log::info!("Загрузка модели {} была отменена пользователем", model_id_clone);
                    let _ = app_clone.emit(
                        "model-download-progress",
                        DownloadProgressPayload {
                            model_id: model_id_clone,
                            percent: 0.0,
                            downloaded_bytes: 0,
                            total_bytes: 0,
                            status: "cancelled".to_string(),
                        },
                    );
                } else {
                    log::error!("Ошибка при скачивании модели {}: {}", model_id_clone, e);
                    let _ = app_clone.emit(
                        "model-download-progress",
                        DownloadProgressPayload {
                            model_id: model_id_clone,
                            percent: 0.0,
                            downloaded_bytes: 0,
                            total_bytes: 0,
                            status: "failed".to_string(),
                        },
                    );
                }
            }
        }
    });

    Ok(format!("Скачивание модели {} запущено в фоновом режиме", model.name))
}

async fn perform_download_bundle(
    app_handle: &AppHandle,
    model_id: &str,
    filename: &str,
    url: &str,
    mmproj_filename: Option<&str>,
    mmproj_url: Option<&str>,
    active_downloads: &Arc<Mutex<std::collections::HashSet<String>>>,
) -> Result<String, AppError> {
    if let (Some(proj_name), Some(proj_url)) = (mmproj_filename, mmproj_url) {
        crate::logger::emit_log(
            app_handle,
            "Models",
            "info",
            &format!("[Шаг 1/2] Загрузка весов модели: '{}'", filename),
        );
        let path1 = perform_single_file_download(
            app_handle,
            model_id,
            filename,
            url,
            active_downloads,
            0.0,
            65.0,
        )
        .await?;

        crate::logger::emit_log(
            app_handle,
            "Models",
            "info",
            &format!("[Шаг 2/2] Загрузка парного проектора mmproj: '{}'", proj_name),
        );
        let _ = perform_single_file_download(
            app_handle,
            model_id,
            proj_name,
            proj_url,
            active_downloads,
            65.0,
            100.0,
        )
        .await?;

        crate::logger::emit_log(
            app_handle,
            "Models",
            "info",
            &format!("Vision-комплект '{}' (веса + проектор mmproj) успешно сохранен и готов к работе!", model_id),
        );

        Ok(path1)
    } else {
        perform_single_file_download(
            app_handle,
            model_id,
            filename,
            url,
            active_downloads,
            0.0,
            100.0,
        )
        .await
    }
}

async fn perform_single_file_download(
    app_handle: &AppHandle,
    model_id: &str,
    filename: &str,
    url: &str,
    active_downloads: &Arc<Mutex<std::collections::HashSet<String>>>,
    start_percent: f64,
    end_percent: f64,
) -> Result<String, AppError> {
    let models_dir = get_models_dir(app_handle);
    tokio::fs::create_dir_all(&models_dir).await.map_err(AppError::Io)?;
    let dest_path = models_dir.join(filename);
    let part_path = models_dir.join(format!("{}.download", filename));

    log::info!(
        "Начало загрузки файла: {} ({}) в {}",
        filename,
        url,
        dest_path.display()
    );

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 VideoCutterPro/2.0")
        .redirect(reqwest::redirect::Policy::limited(20))
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(7200))
        .build()
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка создания HTTP клиента: {}", e)))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка соединения с сервером загрузки: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::Anyhow(anyhow::anyhow!(
            "Сервер ответил ошибкой HTTP {}",
            response.status()
        )));
    }

    let total_bytes = response.content_length().unwrap_or(0);
    let mut file = File::create(&part_path).await.map_err(AppError::Io)?;

    let mut downloaded_bytes = 0u64;
    let mut stream = response.bytes_stream();
    let mut last_emit_percent = -1.0;
    let span = (end_percent - start_percent).max(1.0);

    let download_res: Result<(), AppError> = async {
        while let Some(chunk_result) = stream.next().await {
            // Проверяем, не отменил ли пользователь загрузку
            {
                let active = active_downloads.lock().await;
                if !active.contains(model_id) {
                    return Err(AppError::Anyhow(anyhow::anyhow!("Отменено пользователем")));
                }
            }

            let chunk = chunk_result.map_err(|e| {
                AppError::Anyhow(anyhow::anyhow!("Ошибка при чтении потока данных: {}", e))
            })?;
            file.write_all(&chunk).await.map_err(AppError::Io)?;

            downloaded_bytes += chunk.len() as u64;

            if total_bytes > 0 {
                let part_fraction = (downloaded_bytes as f64 / total_bytes as f64).clamp(0.0, 1.0);
                let overall_percent = (start_percent + part_fraction * span).min(end_percent);
                if overall_percent - last_emit_percent >= 0.5 || overall_percent >= end_percent {
                    last_emit_percent = overall_percent;
                    let _ = app_handle.emit(
                        "model-download-progress",
                        DownloadProgressPayload {
                            model_id: model_id.to_string(),
                            percent: overall_percent,
                            downloaded_bytes,
                            total_bytes,
                            status: "downloading".to_string(),
                        },
                    );
                }
            }
        }
        file.flush().await.map_err(AppError::Io)?;
        Ok(())
    }
    .await;

    drop(file);

    if let Err(e) = download_res {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(e);
    }

    // Проверяем целостность скачанного файла перед переименованием
    if downloaded_bytes < 1_000_000 || (total_bytes > 0 && downloaded_bytes < total_bytes) {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(AppError::Anyhow(anyhow::anyhow!(
            "Загрузка прервалась преждевременно ({} из {} байт)",
            downloaded_bytes,
            total_bytes
        )));
    }

    if dest_path.exists() {
        let _ = tokio::fs::remove_file(&dest_path).await;
    }
    tokio::fs::rename(&part_path, &dest_path).await.map_err(AppError::Io)?;

    Ok(dest_path.to_string_lossy().to_string())
}


