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

/// Асинхронная команда: Ручной экспорт фрагментов (без ИИ анализа видео)
#[tauri::command]
pub async fn render_clips_from_fragments(
    app: AppHandle,
    state: State<'_, AppState>,
    video_path: String,
    fragments: Vec<FragmentDto>,
    config: PipelineConfig,
) -> AppResult<Vec<String>> {
    log::info!("IPC: render_clips_from_fragments: {}, fragments: {:?}", video_path, fragments);
    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { percent: 0.0, stage: "Инициализация...".to_string() });

    let video_buf = PathBuf::from(&video_path);
    if !video_buf.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Файл видео не найден: {}", video_path)));
    }

    let cancel_rx = state.cancel_tx.subscribe();

    // 1. Извлечение аудио во временную системную директорию temp_dir
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let audio_output = std::env::temp_dir().join(format!("vc_manual_audio_{}_{}.wav", std::process::id(), ts));

    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { percent: 10.0, stage: "Извлечение аудио...".to_string() });
    
    crate::ffmpeg::extract_audio(
        video_buf.clone(),
        audio_output.clone(),
        app.clone(),
        cancel_rx.resubscribe()
    ).await?;

    // 2. Распознавание речи через Whisper (субтитры для Reels)
    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { 
        percent: 40.0, 
        stage: "ASR: распознавание речи и титров (Whisper)...".to_string() 
    });
    let asr_model = config.asr_model_path.clone().map(PathBuf::from);
    let srt_path = match crate::whisper::transcribe_audio(
        audio_output.clone(),
        asr_model,
        app.clone(),
        cancel_rx.resubscribe()
    ).await {
        Ok(path) => {
            log::info!("Whisper субтитры успешно созданы: {:?}", path);
            Some(path)
        },
        Err(e) => {
            log::warn!("Whisper ASR завершился с предупреждением: {}. Рендеринг продолжится.", e);
            None
        }
    };

    // 3. Рендеринг клипов в 9:16 с наложением субтитров
    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { 
        percent: 70.0, 
        stage: "Подготовка к нарезке Reels (9:16 + Субтитры)...".to_string() 
    });

    // Для Reels всегда активируем вертикальный формат 9:16
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

    // Сохраняем субтитры в проект, если они получены
    if let Some(ref srt) = srt_path {
        if srt.exists() {
            let subs_dir = project_dir.join("Субтитры");
            let _ = tokio::fs::create_dir_all(&subs_dir).await;
            let dest_srt = subs_dir.join(format!("{}_Субтитры.srt", file_stem));
            let _ = tokio::fs::copy(srt, &dest_srt).await;
        }
    }

    crate::logger::emit_log(
        &app,
        "Render",
        "info",
        &format!("Директория сохранения готовых клипов: '{}'", clips_dir.display()),
    );

    let mut generated_clips = Vec::new();
    let total_fragments = fragments.len();

    for (i, frag) in fragments.iter().enumerate() {
        let pct = 70.0 + ((i as f64 + 1.0) / total_fragments as f64) * 28.0;
        let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { 
            percent: pct, 
            stage: format!("Рендеринг Reels {} из {} (9:16)...", i + 1, total_fragments) 
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

    // Гарантированная очистка временных файлов
    let _ = tokio::fs::remove_file(&audio_output).await;
    if let Some(ref srt) = srt_path {
        let _ = tokio::fs::remove_file(srt).await;
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
    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { percent: 0.0, stage: "Инициализация субтитров...".to_string() });

    let video_buf = PathBuf::from(&video_path);
    if !video_buf.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Файл видео не найден: {}", video_path)));
    }

    let cancel_rx = state.cancel_tx.subscribe();

    // 1. Извлечение аудио в системный temp_dir
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let audio_output = std::env::temp_dir().join(format!("vc_subs_audio_{}_{}.wav", std::process::id(), ts));

    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { percent: 20.0, stage: "Извлечение аудио...".to_string() });
    
    crate::ffmpeg::extract_audio(
        video_buf,
        audio_output.clone(),
        app.clone(),
        cancel_rx.resubscribe()
    ).await?;

    // 2. Распознавание речи
    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { percent: 50.0, stage: "Транскрипция аудио...".to_string() });
    let asr_model = config.asr_model_path.map(PathBuf::from);
    let srt_path = crate::whisper::transcribe_audio(
        audio_output.clone(),
        asr_model,
        app.clone(),
        cancel_rx.resubscribe()
    ).await?;

    // Читаем содержимое SRT
    let content = tokio::fs::read_to_string(&srt_path)
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось прочитать сгенерированный SRT файл: {}", e)))?;

    // Очистка временного аудио
    let _ = tokio::fs::remove_file(&audio_output).await;

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
    folder_path: String,
) -> AppResult<()> {
    let path = PathBuf::from(&folder_path);
    let target_dir = if path.is_file() {
        path.parent().map(|p| p.to_path_buf()).unwrap_or(path)
    } else {
        path
    };

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(&target_dir)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(&target_dir)
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(&target_dir)
            .spawn();
    }
    Ok(())
}
