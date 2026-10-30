// src-tauri/src/commands.rs
use crate::error::{AppResult, AppError};
use crate::AppState;
use tauri::{AppHandle, Emitter, State};
use std::path::PathBuf;
use serde::Serialize;
use crate::pipeline::orchestrator::PipelineConfig;

#[derive(Serialize)]
pub struct TranscribeResult {
    pub srt_path: String,
    pub srt_content: String,
}

/// Асинхронная команда: Извлечение аудиодорожки через встроенный FFmpeg Sidecar.
#[tauri::command]
pub async fn extract_audio(
    app: AppHandle,
    state: State<'_, AppState>,
    video_path: String,
    output_audio_path: Option<String>,
) -> AppResult<String> {
    log::info!("IPC: extract_audio для видео: {}", video_path);
    
    let video_buf = PathBuf::from(&video_path);
    if !video_buf.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Файл видео не найден: {}", video_path)));
    }

    let resolved_audio_path = match output_audio_path {
        Some(path) => PathBuf::from(path),
        None => {
            let mut path = video_buf.clone();
            let file_stem = video_buf.file_stem().and_then(|s| s.to_str()).unwrap_or("extracted");
            path.set_file_name(format!("{}_audio.wav", file_stem));
            path
        }
    };

    let cancel_rx = state.cancel_tx.subscribe();
    
    crate::ffmpeg::extract_audio(
        video_buf, 
        resolved_audio_path.clone(), 
        app, 
        cancel_rx
    ).await?;

    Ok(resolved_audio_path.to_string_lossy().to_string())
}

/// Асинхронная команда: Запуск transcription через whisper.cpp (Sidecar).
#[tauri::command]
pub async fn transcribe_audio(
    app: AppHandle,
    state: State<'_, AppState>,
    audio_path: String,
    model_path: Option<String>,
) -> AppResult<TranscribeResult> {
    log::info!("IPC: transcribe_audio: {}", audio_path);
    
    let audio_buf = PathBuf::from(&audio_path);
    if !audio_buf.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Аудиофайл не найден: {}", audio_path)));
    }
    
    let model_buf = model_path.map(PathBuf::from);
    let cancel_rx = state.cancel_tx.subscribe();
    
    // Вызываем модуль распознавания
    let srt_path = crate::whisper::transcribe_audio(
        audio_buf, 
        model_buf, 
        app, 
        cancel_rx
    ).await?;
    
    // Читаем содержимое SRT
    let content = tokio::fs::read_to_string(&srt_path)
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось прочитать сгенерированный SRT файл: {}", e)))?;
        
    Ok(TranscribeResult {
        srt_path: srt_path.to_string_lossy().to_string(),
        srt_content: content,
    })
}

/// Асинхронная команда: Анализ видео через Qwen3-VL (llama-vision Sidecar).
#[tauri::command]
pub async fn analyze_video(
    app: AppHandle,
    state: State<'_, AppState>,
    video_path: String,
    model_path: Option<String>,
) -> AppResult<Vec<crate::analyzer::SceneCandidate>> {
    log::info!("IPC: analyze_video: {}", video_path);
    
    let video_buf = PathBuf::from(&video_path);
    if !video_buf.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Файл видео не найден: {}", video_path)));
    }
    
    let model_buf = model_path.map(PathBuf::from);
    let cancel_rx = state.cancel_tx.subscribe();
    
    // Вызываем модуль анализатора Qwen-VL
    crate::analyzer::analyze_video(
        video_buf, 
        model_buf, 
        app, 
        cancel_rx
    ).await
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
pub struct FragmentDto {
    pub start: f64,
    pub end: f64,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
pub struct PipelineResult {
    pub clips: Vec<String>,
    pub suggested_fragments: Vec<FragmentDto>,
}

/// Асинхронная команда: Оркестратор полного процесса от загрузки до клипов.
#[tauri::command]
pub async fn start_pipeline(
    app: AppHandle,
    state: State<'_, AppState>,
    video_path: String,
    config: PipelineConfig,
) -> AppResult<PipelineResult> {
    crate::logger::emit_log(&app, "IPC", "info", &format!("IPC: Получена команда start_pipeline для '{}'", video_path));
    
    let video_buf = PathBuf::from(&video_path);
    if !video_buf.exists() {
        let err = format!("Файл видео не найден: {}", video_path);
        crate::logger::emit_log(&app, "IPC", "error", &err);
        return Err(AppError::Anyhow(anyhow::anyhow!(err)));
    }
    
    let cancel_rx = state.cancel_tx.subscribe();
    
    let (results, suggested) = crate::pipeline::orchestrator::run_pipeline(
        video_buf, 
        config, 
        app, 
        cancel_rx
    ).await?;

    Ok(PipelineResult {
        clips: results.into_iter().map(|p| p.to_string_lossy().to_string()).collect(),
        suggested_fragments: suggested,
    })
}

/// Асинхронная команда: Экспорт выбранных/отредактированных фрагментов (использует готовые субтитры из кэша проекта без повторного запуска Whisper!)
#[tauri::command]
pub async fn render_clips_from_fragments(
    app: AppHandle,
    state: State<'_, AppState>,
    video_path: String,
    fragments: Vec<FragmentDto>,
    config: PipelineConfig,
) -> AppResult<Vec<String>> {
    log::info!("IPC: render_clips_from_fragments: {}, fragments: {:?}", video_path, fragments);
    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { percent: 5.0, stage: "Проверка субтитров проекта...".to_string() });

    let video_buf = PathBuf::from(&video_path);
    if !video_buf.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Файл видео не найден: {}", video_path)));
    }

    let cancel_rx = state.cancel_tx.subscribe();

    // 1. Получаем готовые субтитры из папки проекта (или создаем их, если пользователь сразу нажал экспорт в ручном режиме без предварительного запуска пайплайна)
    let asr_model = config.asr_model_path.clone().map(PathBuf::from);
    let srt_path = crate::whisper::get_or_create_subtitles(
        video_buf.clone(),
        asr_model,
        app.clone(),
        cancel_rx.resubscribe(),
    ).await?;

    // 2. Рендеринг клипов в 9:16 с наложением синхронизированных субтитров
    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { 
        percent: 50.0, 
        stage: "Подготовка к нарезке Reels (9:16 + Субтитры)...".to_string() 
    });

    // Для Reels активируем вертикальный формат 9:16
    let render_config = crate::render::RenderConfig {
        vertical_format: true,
        subtitle_style: config.subtitle_style.clone(),
        hw_accel: true,
    };

    // Создаем структурированную директорию проекта и папку «Готовые клипы»
    let file_stem = video_buf.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    let video_parent = video_buf.parent().unwrap_or_else(|| std::path::Path::new("."));
    let project_dir = video_parent.join(format!("{}_Проект", file_stem));
    let clips_dir = project_dir.join("Готовые клипы");
    let _ = tokio::fs::create_dir_all(&clips_dir).await;

    crate::logger::emit_log(
        &app,
        "Render",
        "info",
        &format!("Директория сохранения готовых клипов: '{}'", clips_dir.display()),
    );

    let mut generated_clips = Vec::new();
    let total_fragments = fragments.len().max(1);

    for (i, frag) in fragments.iter().enumerate() {
        let pct = 50.0 + ((i as f64 + 1.0) / total_fragments as f64) * 48.0;
        let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { 
            percent: pct, 
            stage: format!("Рендеринг Reels {} из {} (9:16)...", i + 1, fragments.len()) 
        });

        let start_sec = frag.start;
        let duration = (frag.end - frag.start).max(0.5);

        let start_min = (start_sec / 60.0) as u32;
        let start_s = (start_sec % 60.0) as u32;
        let end_min = (frag.end / 60.0) as u32;
        let end_s = (frag.end % 60.0) as u32;

        let clip_filename = format!(
            "Клип {:02} [{:02}m{:02}s - {:02}m{:02}s].mp4",
            i + 1,
            start_min,
            start_s,
            end_min,
            end_s
        );
        let clip_path = clips_dir.join(clip_filename);

        crate::render::render_clip(
            video_buf.clone(),
            start_sec,
            duration,
            clip_path.clone(),
            srt_path.clone(),
            render_config.clone(),
            app.clone(),
            cancel_rx.resubscribe()
        ).await?;

        generated_clips.push(clip_path.to_string_lossy().to_string());
    }

    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { 
        percent: 100.0, 
        stage: "Экспорт всех Reels успешно завершен!".to_string() 
    });

    Ok(generated_clips)
}

/// Асинхронная команда: Генерация только файлов субтитров
#[tauri::command]
pub async fn generate_subtitles_only(
    app: AppHandle,
    state: State<'_, AppState>,
    video_path: String,
    config: PipelineConfig,
) -> AppResult<String> {
    log::info!("IPC: generate_subtitles_only: {}", video_path);
    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { percent: 10.0, stage: "Подготовка субтитров...".to_string() });

    let video_buf = PathBuf::from(&video_path);
    if !video_buf.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Файл видео не найден: {}", video_path)));
    }

    let cancel_rx = state.cancel_tx.subscribe();
    let asr_model = config.asr_model_path.map(PathBuf::from);

    let srt_path = crate::whisper::get_or_create_subtitles(
        video_buf,
        asr_model,
        app.clone(),
        cancel_rx,
    )
    .await?
    .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("Не удалось сформировать файл субтитров")))?;

    let content = tokio::fs::read_to_string(&srt_path)
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось прочитать сгенерированный SRT файл: {}", e)))?;

    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { percent: 100.0, stage: "Субтитры успешно сгенерированы".to_string() });

    Ok(content)
}

/// Асинхронная команда: Получение реальной длительности и метаданных видео через FFmpeg
#[tauri::command]
pub async fn get_video_metadata(
    app: AppHandle,
    video_path: String,
) -> AppResult<crate::ffmpeg::VideoMetadata> {
    let video_buf = PathBuf::from(&video_path);
    if !video_buf.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Файл видео не найден: {}", video_path)));
    }
    crate::ffmpeg::probe_video(video_buf, app).await
}

/// Асинхронная команда: Отмена выполнения конвейера и прерывание sidecar-процессов
#[tauri::command]
pub async fn cancel_pipeline(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    crate::logger::emit_log(&app, "Pipeline", "warn", "IPC: Получен сигнал отмены операции от пользователя.");
    let _ = state.cancel_tx.send(());
    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload {
        percent: 0.0,
        stage: "Операция отменена".to_string(),
    });
    Ok(())
}

/// Асинхронная команда: Открытие директории в проводнике операционной системы
#[tauri::command]
pub async fn open_folder(
    app: AppHandle,
    folder_path: String,
) -> AppResult<String> {
    let trimmed = folder_path.trim();
    let path = if trimmed.is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        PathBuf::from(trimmed)
    };

    let raw_target = if path.is_dir() {
        path.clone()
    } else if path.is_file() || path.extension().is_some() {
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        path.clone()
    };

    let abs_target = if raw_target.is_relative() {
        std::env::current_dir()
            .map(|cwd| cwd.join(&raw_target))
            .unwrap_or(raw_target)
    } else {
        raw_target
    };

    let existing_dir = abs_target
        .ancestors()
        .find(|p| p.exists() && p.is_dir())
        .map(|p| p.to_path_buf())
        .unwrap_or(abs_target);

    crate::logger::emit_log(
        &app,
        "System",
        "info",
        &format!("Открытие папки с готовыми роликами в проводнике ОС: '{}'", existing_dir.display()),
    );

    #[cfg(target_os = "windows")]
    {
        let win_dir = existing_dir
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('/', r"\");
        std::process::Command::new("explorer")
            .arg(&win_dir)
            .spawn()
            .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось запустить проводник Windows: {}", e)))?;
    }
    #[cfg(target_os = "macos")]
    {
        if path.exists() && path.is_file() {
            let status = std::process::Command::new("open")
                .arg("-R")
                .arg(&path)
                .spawn();
            if status.is_err() {
                std::process::Command::new("open")
                    .arg(&existing_dir)
                    .spawn()
                    .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось открыть Finder: {}", e)))?;
            }
        } else {
            std::process::Command::new("open")
                .arg(&existing_dir)
                .spawn()
                .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось открыть Finder: {}", e)))?;
        }
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&existing_dir)
            .spawn()
            .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось открыть файловый менеджер (xdg-open): {}", e)))?;
    }

    Ok(existing_dir.to_string_lossy().to_string())
}

