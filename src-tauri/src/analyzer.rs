// src-tauri/src/analyzer.rs
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandEvent;
use serde::{Deserialize, Serialize};
use crate::error::{AppResult, AppError};
use tokio::sync::broadcast::Receiver;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SceneCandidate {
    pub timestamp_sec: f64,
    pub description: String,
    pub intensity: f64,
    pub actions: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct AnalyzerProgressPayload {
    pub percent: f64,
    pub stage: String,
    pub current_frame: u32,
    pub total_frames: u32,
}

#[derive(Deserialize, Debug)]
struct LlamaOutput {
    #[serde(default)]
    description: String,
    #[serde(default)]
    intensity: f64,
    #[serde(default)]
    actions: Vec<String>,
}

/// Выполняет анализ видео с помощью модели Qwen3-VL
pub async fn analyze_video(
    video_path: PathBuf,
    model_path: Option<PathBuf>,
    app_handle: AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<Vec<SceneCandidate>> {
    log::info!("Запуск ASR анализа видео через Qwen3-VL: {}", video_path.display());
    
    let shell = app_handle.shell();
    
    // Получаем путь к модели llama-vision через многоуровневый резолвер
    let model = match model_path {
        Some(p) => crate::models::resolve_model_path(&app_handle, &p.to_string_lossy()),
        None => None,
    }
    .or_else(|| crate::models::resolve_model_path(&app_handle, "qwen-vl-2b.gguf"))
    .or_else(|| crate::models::resolve_model_path(&app_handle, "qwen2-vl-2b-instruct-q4_k_m.gguf"))
    .or_else(|| crate::models::resolve_model_path(&app_handle, "models/qwen-vl-2b.gguf"));

    let model = match model {
        Some(m) if m.exists() => m,
        _ => {
            return Err(AppError::Pipeline(
                "Модель визуального анализа (qwen-vl-2b.gguf) не найдена. Убедитесь, что модель установлена или используйте текстовый/аудио анализ.".to_string(),
            ));
        }
    };

    // Создаем временную директорию для хранения кадров
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros();
    let pid = std::process::id();
    let temp_dir = std::env::temp_dir().join(format!("video_cutter_frames_{}_{}", pid, ts));
    tokio::fs::create_dir_all(&temp_dir).await.map_err(AppError::Io)?;
    
    let frames_pattern = temp_dir.join("frame_%d.jpg");

    let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
        percent: 0.0,
        stage: "extracting_frames".to_string(),
        current_frame: 0,
        total_frames: 0,
    });

    // 1. Извлечение кадров через FFmpeg (1 кадр каждые 5 секунд)
    let ffmpeg_cmd = shell.sidecar("ffmpeg")
        .map_err(|e| {
            let _ = std::fs::remove_dir_all(&temp_dir);
            AppError::Pipeline(format!("Бинарник ffmpeg не найден в sidecar ресурсах: {}", e))
        })?
        .args([
            "-y",
            "-i", &video_path.to_string_lossy(),
            "-vf", "fps=1/5",
            "-frame_pts", "1",
            "-vsync", "0",
            &frames_pattern.to_string_lossy()
        ]);

    let (mut rx, child) = ffmpeg_cmd.spawn()
        .map_err(|e| {
            let _ = std::fs::remove_dir_all(&temp_dir);
            AppError::Pipeline(format!("Не удалось запустить FFmpeg для извлечения кадров: {}", e))
        })?;

    let extract_future = async {
        while let Some(event) = rx.recv().await {
            if let CommandEvent::Terminated(payload) = event {
                if payload.code == Some(0) {
                    return Ok(());
                } else {
                    return Err(AppError::Pipeline(format!("FFmpeg (извлечение кадров) завершился с ошибкой: {:?}", payload.code)));
                }
            }
        }
        Err(AppError::Pipeline("Процесс FFmpeg неожиданно прервался.".to_string()))
    };

    tokio::select! {
        res = extract_future => {
            if let Err(e) = res {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Err(e);
            }
        },
        _ = cancel_rx.recv() => {
            let _ = child.kill();
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(AppError::Pipeline("Анализ отменен пользователем.".to_string()));
        }
    }

    // 2. Сбор извлечённых кадров
    let mut frames_with_ts = Vec::new();
    let mut entries = tokio::fs::read_dir(&temp_dir).await.map_err(AppError::Io)?;
    while let Some(entry) = entries.next_entry().await.map_err(AppError::Io)? {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("jpg") {
            if let Some(file_name) = path.file_stem().and_then(|s| s.to_str()) {
                if let Some(num_str) = file_name.strip_prefix("frame_") {
                    if let Ok(num) = num_str.parse::<u32>() {
                        // Время кадра = номер кадра * 5 секунд (по условию задачи)
                        let timestamp_sec = num as f64 * 5.0;
                        frames_with_ts.push((timestamp_sec, path, num));
                    }
                }
            }
        }
    }

    // Сортируем кадры в правильном хронологическом порядке
    frames_with_ts.sort_by_key(|k| k.2);

    let total_frames = frames_with_ts.len() as u32;
    if total_frames == 0 {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return Err(AppError::Pipeline("Не удалось извлечь ни одного кадра из видео.".to_string()));
    }

    let mut candidates = Vec::new();
    let prompt = "Опиши, что происходит на этом кадре. Оцени эмоциональную напряженность от 0 до 1 и определи наличие ключевых действий из списка: смех, драка, диалог, напряженная сцена, погоня, крик, падение. Ответ дай в формате JSON: {\"description\": \"...\", \"intensity\": 0.5, \"actions\": [\"action1\", \"action2\"]}.";

    // 3. Анализ каждого кадра через llama-vision
    for (i, (timestamp_sec, frame_path, _)) in frames_with_ts.into_iter().enumerate() {
        let current_frame = (i + 1) as u32;
        let percent = (current_frame as f64 / total_frames as f64) * 100.0;

        let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
            percent,
            stage: "analyzing".to_string(),
            current_frame,
            total_frames,
        });

        let llama_cmd = shell.sidecar("llama-vision")
            .map_err(|e| {
                let _ = tokio::fs::remove_dir_all(&temp_dir);
                AppError::Pipeline(format!("Бинарник llama-vision не найден: {}", e))
            })?
            .args([
                "-m", &model.to_string_lossy(),
                "--image", &frame_path.to_string_lossy(),
                "--prompt", prompt,
                "--json",
            ]);

        let (mut rx, child) = llama_cmd.spawn().map_err(|e| {
            let _ = tokio::fs::remove_dir_all(&temp_dir);
            AppError::Pipeline(format!("Не удалось запустить llama-vision: {}", e))
        })?;

        let analyze_future = async {
            let mut stdout_accum = String::new();
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(bytes) => {
                        stdout_accum.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    CommandEvent::Terminated(payload) => {
                        if payload.code == Some(0) {
                            return Ok(stdout_accum);
                        } else {
                            return Err(AppError::Pipeline(format!("llama-vision завершился с кодом {:?}", payload.code)));
                        }
                    }
                    _ => {}
                }
            }
            Err(AppError::Pipeline("Процесс llama-vision неожиданно прервался.".to_string()))
        };

        let output_json = tokio::select! {
            res = analyze_future => {
                match res {
                    Ok(out) => out,
                    Err(e) => {
                        log::error!("Ошибка анализа кадра {}: {}", current_frame, e);
                        continue; // пропускаем проблемный кадр и идем дальше
                    }
                }
            },
            _ = cancel_rx.recv() => {
                let _ = child.kill();
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Err(AppError::Pipeline("Анализ отменен пользователем.".to_string()));
            }
        };

        // Парсинг JSON-вывода
        let json_start = output_json.find('{').unwrap_or(0);
        let json_end = output_json.rfind('}').unwrap_or(output_json.len().saturating_sub(1));
        
        if json_start <= json_end {
            let clean_json = &output_json[json_start..=json_end];
            if let Ok(parsed) = serde_json::from_str::<LlamaOutput>(clean_json) {
                candidates.push(SceneCandidate {
                    timestamp_sec,
                    description: parsed.description,
                    intensity: parsed.intensity,
                    actions: parsed.actions,
                });
            } else {
                log::warn!("Не удалось распарсить JSON из ответа модели llama-vision: {}", clean_json);
            }
        }
    }

    let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
        percent: 100.0,
        stage: "saving".to_string(),
        current_frame: total_frames,
        total_frames,
    });

    // 4. Очистка временных файлов
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    Ok(candidates)
}
