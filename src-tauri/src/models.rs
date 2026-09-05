// src-tauri/src/models.rs
use std::path::PathBuf;
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
    pub exists: bool,
    pub local_path: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct DownloadProgressPayload {
    pub model_id: String,
    pub percent: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub status: String, // "downloading", "completed", "failed"
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

fn get_models_dir(app_handle: &AppHandle) -> PathBuf {
    app_handle
        .path()
        .resource_dir()
        .unwrap_or_default()
        .join("models")
}

fn get_all_models_config() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "whisper-tiny".to_string(),
            name: "Whisper Tiny".to_string(),
            filename: "ggml-tiny.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin".to_string(),
            size_mb: 75.0,
            description: "Сверхбыстрая компактная модель для распознавания речи (75 MB)".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "whisper-base".to_string(),
            name: "Whisper Base".to_string(),
            filename: "ggml-base.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin".to_string(),
            size_mb: 148.0,
            description: "Стандартная и быстрая модель для большинства диалогов (148 MB)".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "whisper-small".to_string(),
            name: "Whisper Small".to_string(),
            filename: "ggml-small.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin".to_string(),
            size_mb: 466.0,
            description: "Высокоточная модель для сложных условий записи (466 MB)".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "qwen-vl-2b".to_string(),
            name: "Qwen3-VL 2B".to_string(),
            filename: "qwen-vl-2b.gguf".to_string(),
            url: "https://huggingface.co/Qwen/Qwen2-VL-2B-Instruct-GGUF/resolve/main/qwen2-vl-2b-instruct-q4_k_m.gguf".to_string(),
            size_mb: 1700.0,
            description: "Интеллектуальный ИИ-анализатор кадров и поведения лиц (1.7 GB)".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
    ]
}

#[tauri::command]
pub async fn get_models_status(app_handle: AppHandle) -> AppResult<Vec<ModelInfo>> {
    let models_dir = get_models_dir(&app_handle);
    let mut configured_models = get_all_models_config();

    for model in &mut configured_models {
        let path = models_dir.join(&model.filename);
        if path.exists() {
            model.exists = true;
            model.local_path = path.to_string_lossy().to_string();
        } else {
            model.exists = false;
        }
    }

    Ok(configured_models)
}

#[tauri::command]
pub async fn download_model(
    app_handle: AppHandle,
    models_state: State<'_, ModelsState>,
    model_id: String,
) -> AppResult<String> {
    log::info!("Запуск скачивания модели: {}", model_id);

    // Находим описание модели
    let models = get_all_models_config();
    let model = models.iter().find(|m| m.id == model_id)
        .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("Неизвестный ID модели: {}", model_id)))?;

    // Проверяем, не скачивается ли уже
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

    // Запускаем загрузку в фоновой tokio задаче, чтобы не блокировать IPC
    tokio::spawn(async move {
        let res = perform_download(&app_clone, &model_id_clone, &model_filename, &model_url).await;
        
        // Убираем из списка активных в конце работы
        let mut active = active_downloads_clone.lock().await;
        active.remove(&model_id_clone);

        match res {
            Ok(path) => {
                log::info!("Модель {} успешно загружена и сохранена по адресу: {}", model_id_clone, path);
                let _ = app_clone.emit("model-download-progress", DownloadProgressPayload {
                    model_id: model_id_clone,
                    percent: 100.0,
                    downloaded_bytes: 0,
                    total_bytes: 0,
                    status: "completed".to_string(),
                });
            }
            Err(e) => {
                log::error!("Ошибка при скачивании модели {}: {}", model_id_clone, e);
                let _ = app_clone.emit("model-download-progress", DownloadProgressPayload {
                    model_id: model_id_clone,
                    percent: 0.0,
                    downloaded_bytes: 0,
                    total_bytes: 0,
                    status: "failed".to_string(),
                });
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
) -> Result<String, AppError> {
    let models_dir = get_models_dir(app_handle);
    tokio::fs::create_dir_all(&models_dir).await.map_err(AppError::Io)?;
    let dest_path = models_dir.join(filename);

    log::info!("Загрузка по ссылке: {} в {}", url, dest_path.display());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3600)) // 1 час таймаут для больших файлов
        .build()
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка создания HTTP клиента: {}", e)))?;

    let response = client.get(url)
        .send()
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка отправки HTTP запроса: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Сервер ответил ошибкой: {}", response.status())));
    }

    let total_bytes = response.content_length().unwrap_or(0);
    let mut file = File::create(&dest_path).await.map_err(AppError::Io)?;
    
    let mut downloaded_bytes = 0u64;
    let mut stream = response.bytes_stream();
    let mut last_emit_percent = -1.0;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка при чтении потока данных: {}", e)))?;
        file.write_all(&chunk).await.map_err(AppError::Io)?;
        
        downloaded_bytes += chunk.len() as u64;

        if total_bytes > 0 {
            let percent = (downloaded_bytes as f64 / total_bytes as f64) * 100.0;
            // Шлем события не слишком часто — только при изменении процента на 0.5%
            if percent - last_emit_percent >= 0.5 || percent >= 100.0 {
                last_emit_percent = percent;
                let _ = app_handle.emit("model-download-progress", DownloadProgressPayload {
                    model_id: model_id.to_string(),
                    percent,
                    downloaded_bytes,
                    total_bytes,
                    status: "downloading".to_string(),
                });
            }
        }
    }

    file.flush().await.map_err(AppError::Io)?;

    Ok(dest_path.to_string_lossy().to_string())
}
