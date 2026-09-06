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

pub fn get_models_dir(app_handle: &AppHandle) -> PathBuf {
    // 1. Проверяем папку models рядом с исполняемым файлом (для Portable)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let portable_models = exe_dir.join("models");
            if portable_models.exists() {
                return portable_models;
            }
        }
    }

    // 2. В папке ресурсов Tauri
    if let Ok(resource_dir) = app_handle.path().resource_dir() {
        let res_models = resource_dir.join("models");
        if res_models.exists() {
            return res_models;
        }
    }

    // 3. В рабочей директории
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_models = cwd.join("models");
        if cwd_models.exists() {
            return cwd_models;
        }
    }

    // 4. Папка AppData / Ресурсы по умолчанию
    if let Ok(app_data) = app_handle.path().app_data_dir() {
        let data_models = app_data.join("models");
        return data_models;
    }

    PathBuf::from("models")
}

pub fn resolve_model_path(app_handle: &AppHandle, model_name_or_path: &str) -> Option<PathBuf> {
    let p = PathBuf::from(model_name_or_path);
    if p.exists() {
        return Some(p);
    }
    let filename = p.file_name().and_then(|f| f.to_str()).unwrap_or(model_name_or_path);

    // Список возможных алиасов для разных моделей
    let mut search_filenames = vec![filename.to_string()];
    if filename.contains("qwen-vl") || filename.contains("qwen2-vl") {
        search_filenames.push("qwen-vl-2b.gguf".to_string());
        search_filenames.push("qwen2-vl-2b-instruct-q4_k_m.gguf".to_string());
    } else if filename.contains("qwen-7b") || filename.contains("qwen2.5-7b") {
        search_filenames.push("qwen-7b-chat.gguf".to_string());
        search_filenames.push("qwen2.5-7b-instruct-q4_k_m.gguf".to_string());
    } else if filename.contains("qwen-1.5b") || filename.contains("qwen2.5-1.5b") {
        search_filenames.push("qwen-1.5b-chat.gguf".to_string());
        search_filenames.push("qwen2.5-1.5b-instruct-q4_k_m.gguf".to_string());
    } else if filename.contains("base") {
        search_filenames.push("ggml-base.bin".to_string());
    } else if filename.contains("tiny") {
        search_filenames.push("ggml-tiny.bin".to_string());
    } else if filename.contains("small") {
        search_filenames.push("ggml-small.bin".to_string());
    }

    for name in &search_filenames {
        let candidates = vec![
            // 1. App Data dir / models / filename
            app_handle.path().app_data_dir().map(|d| d.join("models").join(name)).ok(),
            // 2. Executable dir / models / filename (для Portable версий)
            std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("models").join(name))),
            // 3. Executable dir / filename
            std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name))),
            // 4. Resource dir / models / filename
            app_handle.path().resource_dir().map(|d| d.join("models").join(name)).ok(),
            // 5. Resource dir / filename
            app_handle.path().resource_dir().map(|d| d.join(name)).ok(),
            // 6. Current working directory / models / filename
            std::env::current_dir().map(|d| d.join("models").join(name)).ok(),
            // 7. Current working directory / src-tauri / models / filename (dev режим)
            std::env::current_dir().map(|d| d.join("src-tauri").join("models").join(name)).ok(),
        ];

        for candidate in candidates.into_iter().flatten() {
            if candidate.exists() {
                log::info!("Успешно найдена модель: {}", candidate.display());
                return Some(candidate);
            }
        }
    }

    None
}

pub fn get_all_models_config() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "whisper-tiny".to_string(),
            name: "Whisper Tiny".to_string(),
            filename: "ggml-tiny.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin".to_string(),
            size_mb: 75.0,
            description: "Сверхбыстрая компактная модель распознавания речи (75 MB)".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "whisper-base".to_string(),
            name: "Whisper Base (Рекомендуется)".to_string(),
            filename: "ggml-base.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin".to_string(),
            size_mb: 148.0,
            description: "Стандартная мультиязычная модель для русской и мировой речи (148 MB)".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "whisper-small".to_string(),
            name: "Whisper Small (Высокая точность)".to_string(),
            filename: "ggml-small.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin".to_string(),
            size_mb: 466.0,
            description: "Точная модель для сложных условий записи и шумных стримов (466 MB)".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "qwen-vl-2b".to_string(),
            name: "Qwen2-VL 2B (Vision AI)".to_string(),
            filename: "qwen-vl-2b.gguf".to_string(),
            url: "https://huggingface.co/Qwen/Qwen2-VL-2B-Instruct-GGUF/resolve/main/qwen2-vl-2b-instruct-q4_k_m.gguf".to_string(),
            size_mb: 1700.0,
            description: "Локальный визуальный анализатор кадров и лиц (1.7 GB)".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "qwen-1.5b-chat".to_string(),
            name: "Qwen2.5 1.5B Chat (Быстрый текст)".to_string(),
            filename: "qwen-1.5b-chat.gguf".to_string(),
            url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf".to_string(),
            size_mb: 986.0,
            description: "Легковесная автономная модель анализа диалогов и хайлайтов (986 MB)".to_string(),
            exists: false,
            local_path: "".to_string(),
        },
        ModelInfo {
            id: "qwen-7b-chat".to_string(),
            name: "Qwen2.5 7B Chat (Продвинутый ИИ)".to_string(),
            filename: "qwen-7b-chat.gguf".to_string(),
            url: "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m.gguf".to_string(),
            size_mb: 4680.0,
            description: "Мощная автономная языковая модель глубокого анализа юмора и сюжета (4.6 GB)".to_string(),
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

    let models = get_all_models_config();
    let model = models.iter().find(|m| m.id == model_id)
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
        let res = perform_download(&app_clone, &model_id_clone, &model_filename, &model_url).await;
        
        let mut active = active_downloads_clone.lock().await;
        active.remove(&model_id_clone);

        match res {
            Ok(path) => {
                log::info!("Модель {} успешно сохранена: {}", model_id_clone, path);
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
    let part_path = models_dir.join(format!("{}.download", filename));

    log::info!("Начало загрузки модели: {} ({}) в {}", model_id, url, dest_path.display());

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 VideoCutterPro/2.0")
        .redirect(reqwest::redirect::Policy::limited(15))
        .timeout(std::time::Duration::from_secs(7200)) // 2 часа таймаут для больших GGUF файлов
        .build()
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка создания HTTP клиента: {}", e)))?;

    let response = client.get(url)
        .send()
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка соединения с сервером загрузки: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Сервер ответил ошибкой: {}", response.status())));
    }

    let total_bytes = response.content_length().unwrap_or(0);
    let mut file = File::create(&part_path).await.map_err(AppError::Io)?;
    
    let mut downloaded_bytes = 0u64;
    let mut stream = response.bytes_stream();
    let mut last_emit_percent = -1.0;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка при чтении потока данных: {}", e)))?;
        file.write_all(&chunk).await.map_err(AppError::Io)?;
        
        downloaded_bytes += chunk.len() as u64;

        if total_bytes > 0 {
            let percent = (downloaded_bytes as f64 / total_bytes as f64) * 100.0;
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
    drop(file);

    // Переименовываем .download во временное имя
    if dest_path.exists() {
        let _ = tokio::fs::remove_file(&dest_path).await;
    }
    tokio::fs::rename(&part_path, &dest_path).await.map_err(AppError::Io)?;

    Ok(dest_path.to_string_lossy().to_string())
}

