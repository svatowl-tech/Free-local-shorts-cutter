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

    if lower.contains("qwen-vl") || lower.contains("qwen2-vl") {
        search_filenames.push("qwen-vl-2b.gguf".to_string());
        search_filenames.push("Qwen2-VL-2B-Instruct-Q4_K_M.gguf".to_string());
        search_filenames.push("qwen2-vl-2b-instruct-q4_k_m.gguf".to_string());
    } else if lower.contains("qwen-0.5b") || lower.contains("qwen2.5-0.5b") {
        search_filenames.push("qwen2.5-0.5b-instruct-q4_k_m.gguf".to_string());
        search_filenames.push("qwen-0.5b-chat.gguf".to_string());
    } else if lower.contains("qwen-1.5b") || lower.contains("qwen2.5-1.5b") {
        search_filenames.push("qwen-1.5b-chat.gguf".to_string());
        search_filenames.push("qwen2.5-1.5b-instruct-q4_k_m.gguf".to_string());
    } else if lower.contains("qwen-3b") || lower.contains("qwen2.5-3b") {
        search_filenames.push("qwen2.5-3b-instruct-q4_k_m.gguf".to_string());
        search_filenames.push("qwen-3b-chat.gguf".to_string());
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

/// Ищет сначала указанную текстовую GGUF-модель, а если передан старый дефолт `models/qwen-1.5b-chat.gguf` —
/// проверяет сначала более мощные установленные модели (от 3B к 0.5B), чтобы автоматически задействовать `qwen2.5-3b-instruct`!
pub fn resolve_any_llm_model(app_handle: &AppHandle, preferred: Option<&str>) -> Option<PathBuf> {
    if let Some(pref) = preferred {
        let is_legacy_default =
            pref == "models/qwen-1.5b-chat.gguf" || pref == "qwen-1.5b-chat.gguf";
        if !is_legacy_default {
            if let Some(found) = resolve_model_path(app_handle, pref) {
                return Some(found);
            }
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

pub fn get_all_models_config() -> Vec<ModelInfo> {
    vec![
        // ==================== WHISPER ASR (РАСПОЗНАВАНИЕ РЕЧИ И СУБТИТРЫ) ====================
        ModelInfo {
            id: "whisper-tiny".to_string(),
            name: "Whisper Tiny (Ультрабыстрая)".to_string(),
            filename: "ggml-tiny.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin".to_string(),
            size_mb: 75.0,
            description: "Сверхбыстрая модель для слабых ПК и мгновенной черновой транскрипции (75 МБ)".to_string(),
            category: "whisper".to_string(),
            params: "39M".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "whisper-base".to_string(),
            name: "Whisper Base (Рекомендуется)".to_string(),
            filename: "ggml-base.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin".to_string(),
            size_mb: 142.0,
            description: "Оптимальный баланс скорости и точности для русской и английской речи (142 МБ)".to_string(),
            category: "whisper".to_string(),
            params: "74M".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "whisper-small-q5".to_string(),
            name: "Whisper Small Q5 (Легкая Pro)".to_string(),
            filename: "ggml-small-q5_1.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin".to_string(),
            size_mb: 181.0,
            description: "Квантованная Small-модель: высокая точность распознавания при малом весе (181 МБ)".to_string(),
            category: "whisper".to_string(),
            params: "244M Q5".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "whisper-small".to_string(),
            name: "Whisper Small (Высокая точность)".to_string(),
            filename: "ggml-small.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin".to_string(),
            size_mb: 466.0,
            description: "Отличное качество для аниме, стримов, сложной дикции и фонового шума (466 МБ)".to_string(),
            category: "whisper".to_string(),
            params: "244M".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "whisper-medium-q5".to_string(),
            name: "Whisper Medium Q5 (Макс. точность RU)".to_string(),
            filename: "ggml-medium-q5_0.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium-q5_0.bin".to_string(),
            size_mb: 514.0,
            description: "Квантованная Medium-модель: студийная точность русского языка без перегрузки ОЗУ (514 МБ)".to_string(),
            category: "whisper".to_string(),
            params: "769M Q5".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "whisper-large-v3-turbo-q5".to_string(),
            name: "Whisper Large-v3 Turbo Q5".to_string(),
            filename: "ggml-large-v3-turbo-q5_0.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin".to_string(),
            size_mb: 547.0,
            description: "Ускоренная флагманская модель OpenAI Large-v3 Turbo в компактном формате Q5 (547 МБ)".to_string(),
            category: "whisper".to_string(),
            params: "809M Q5".to_string(),
            exists: false,
            local_path: "".to_string(),
        },

        // ==================== LLM (АНАЛИЗ ДИАЛОГОВ И ОТБОР ХАЙЛАЙТОВ — ДО 3B) ====================
        ModelInfo {
            id: "qwen-0.5b-chat".to_string(),
            name: "Qwen 2.5 0.5B Instruct (Ультралегкая)".to_string(),
            filename: "qwen2.5-0.5b-instruct-q4_k_m.gguf".to_string(),
            url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf".to_string(),
            size_mb: 469.0,
            description: "Мгновенный анализ реплик на любых ПК и ноутбуках даже без дискретной видеокарты (469 МБ)".to_string(),
            category: "llm".to_string(),
            params: "0.5B".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "llama-3.2-1b".to_string(),
            name: "Llama 3.2 1B Instruct (Быстрая)".to_string(),
            filename: "Llama-3.2-1B-Instruct-Q4_K_M.gguf".to_string(),
            url: "https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf".to_string(),
            size_mb: 770.0,
            description: "Легкая открытая модель Meta Llama 3.2 для быстрой оценки динамики диалогов (770 МБ)".to_string(),
            category: "llm".to_string(),
            params: "1B".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "qwen-1.5b-chat".to_string(),
            name: "Qwen 2.5 1.5B Instruct (Рекомендуется)".to_string(),
            filename: "qwen-1.5b-chat.gguf".to_string(),
            url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf".to_string(),
            size_mb: 1065.0,
            description: "Оптимальный выбор: отличное понимание русского контекста, юмора и эмоций (1.06 ГБ)".to_string(),
            category: "llm".to_string(),
            params: "1.5B".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "gemma-2-2b".to_string(),
            name: "Google Gemma 2 2B Instruct".to_string(),
            filename: "gemma-2-2b-it-Q4_K_M.gguf".to_string(),
            url: "https://huggingface.co/bartowski/gemma-2-2b-it-GGUF/resolve/main/gemma-2-2b-it-Q4_K_M.gguf".to_string(),
            size_mb: 1629.0,
            description: "Модель от Google (2B): точное выделение смысловых панчлайнов и ярких реплик (1.63 ГБ)".to_string(),
            category: "llm".to_string(),
            params: "2B".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "llama-3.2-3b".to_string(),
            name: "Llama 3.2 3B Instruct (Продвинутая)".to_string(),
            filename: "Llama-3.2-3B-Instruct-Q4_K_M.gguf".to_string(),
            url: "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf".to_string(),
            size_mb: 1926.0,
            description: "Продвинутая 3B модель с глубоким анализом драматургии и сюжетных поворотов (1.93 ГБ)".to_string(),
            category: "llm".to_string(),
            params: "3B".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "qwen-3b-chat".to_string(),
            name: "Qwen 2.5 3B Instruct (Максимум 3B)".to_string(),
            filename: "qwen2.5-3b-instruct-q4_k_m.gguf".to_string(),
            url: "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf".to_string(),
            size_mb: 2007.0,
            description: "Максимальное качество анализа русскоязычных диалогов и сленга в пределах 3B (2.0 ГБ)".to_string(),
            category: "llm".to_string(),
            params: "3B".to_string(),
            exists: false,
            local_path: "".to_string(),
        },

        // ==================== VISION AI (ВИЗУАЛЬНЫЙ АНАЛИЗ КАДРОВ — ДО 2B) ====================
        ModelInfo {
            id: "qwen-vl-2b".to_string(),
            name: "Qwen2-VL 2B Vision (Анализ кадров)".to_string(),
            filename: "qwen-vl-2b.gguf".to_string(),
            url: "https://huggingface.co/bartowski/Qwen2-VL-2B-Instruct-GGUF/resolve/main/Qwen2-VL-2B-Instruct-Q4_K_M.gguf".to_string(),
            size_mb: 940.0,
            description: "Компактный локальный визуальный анализатор динамики сцен и ключевых кадров (940 МБ)".to_string(),
            category: "vision".to_string(),
            params: "2B Vision".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
    ]
}

#[tauri::command]
pub async fn get_models_status(app_handle: AppHandle) -> AppResult<Vec<ModelInfo>> {
    let mut configured_models = get_all_models_config();

    for model in &mut configured_models {
        if let Some(resolved_path) = resolve_model_path(&app_handle, &model.filename) {
            model.exists = true;
            model.local_path = resolved_path.to_string_lossy().to_string();
        } else {
            model.exists = false;
            model.local_path = "".to_string();
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

    let aliases = expand_model_aliases(&model.filename);
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

    tokio::spawn(async move {
        let res = perform_download(
            &app_clone,
            &model_id_clone,
            &model_filename,
            &model_url,
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

async fn perform_download(
    app_handle: &AppHandle,
    model_id: &str,
    filename: &str,
    url: &str,
    active_downloads: &Arc<Mutex<std::collections::HashSet<String>>>,
) -> Result<String, AppError> {
    let models_dir = get_models_dir(app_handle);
    tokio::fs::create_dir_all(&models_dir).await.map_err(AppError::Io)?;
    let dest_path = models_dir.join(filename);
    let part_path = models_dir.join(format!("{}.download", filename));

    log::info!(
        "Начало загрузки модели: {} ({}) в {}",
        model_id,
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
                let percent = (downloaded_bytes as f64 / total_bytes as f64) * 100.0;
                if percent - last_emit_percent >= 0.5 || percent >= 100.0 {
                    last_emit_percent = percent;
                    let _ = app_handle.emit(
                        "model-download-progress",
                        DownloadProgressPayload {
                            model_id: model_id.to_string(),
                            percent,
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


