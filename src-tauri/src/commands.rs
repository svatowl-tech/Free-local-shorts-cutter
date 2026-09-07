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

/// Асинхронная команда: Оркестратор полного процесса от загрузки до клипов.
#[tauri::command]
pub async fn start_pipeline(
    app: AppHandle,
    state: State<'_, AppState>,
    video_path: String,
    config: PipelineConfig,
) -> AppResult<Vec<String>> {
    log::info!("IPC: start_pipeline: {}", video_path);
    
    let video_buf = PathBuf::from(&video_path);
    if !video_buf.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Файл видео не найден: {}", video_path)));
    }
    
    let cancel_rx = state.cancel_tx.subscribe();
    
    let results = crate::pipeline::orchestrator::run_pipeline(
        video_buf, 
        config, 
        app, 
        cancel_rx
    ).await?;

    Ok(results.into_iter().map(|p| p.to_string_lossy().to_string()).collect())
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
pub struct FragmentDto {
    pub start: f64,
    pub end: f64,
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

    // 1. Извлечение аудио
    let mut audio_output = video_buf.clone();
    let file_stem = video_buf.file_stem().and_then(|s| s.to_str()).unwrap_or("extracted");
    audio_output.set_file_name(format!("{}_manual_audio.wav", file_stem));

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

    let base_out_dir = config.output_dir.clone().map(PathBuf::from).unwrap_or_else(|| {
        let mut d = video_buf.clone();
        d.pop();
        d
    });

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

        let mut clip_path = base_out_dir.clone();
        clip_path.push(format!("{}_reels_{}_start_{}s.mp4", file_stem, i + 1, start_sec as u32));

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

    // Очистка временного аудио
    let _ = tokio::fs::remove_file(&audio_output).await;

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

    // 1. Извлечение аудио
    let mut audio_output = video_buf.clone();
    let file_stem = video_buf.file_stem().and_then(|s| s.to_str()).unwrap_or("extracted");
    audio_output.set_file_name(format!("{}_subs_audio.wav", file_stem));

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
