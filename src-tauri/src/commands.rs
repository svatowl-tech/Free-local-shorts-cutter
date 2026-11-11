// src-tauri/src/commands.rs
use crate::error::{AppResult, AppError};
use crate::AppState;
use tauri::{AppHandle, Emitter, State};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
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
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub repaired: Option<bool>,
    #[serde(default)]
    pub repair_note: Option<String>,
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
    
    let mut config = config;
    if config.bumper_config.as_ref().map(|b| !b.enabled).unwrap_or(true) {
        if let Some(saved) = crate::bumpers::load_saved_bumper_config(&app) {
            if saved.enabled {
                crate::logger::emit_log(
                    &app,
                    "Bumpers",
                    "info",
                    &format!("Загружена активная сохраненная конфигурация заставок с диска (пресет: '{}')", saved.active_preset_id),
                );
                config.bumper_config = Some(saved);
            }
        }
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

    let mut config = config;
    if config.bumper_config.as_ref().map(|b| !b.enabled).unwrap_or(true) {
        if let Some(saved) = crate::bumpers::load_saved_bumper_config(&app) {
            if saved.enabled {
                crate::logger::emit_log(
                    &app,
                    "Bumpers",
                    "info",
                    &format!("Загружена активная сохраненная конфигурация заставок с диска для экспорта (пресет: '{}')", saved.active_preset_id),
                );
                config.bumper_config = Some(saved);
            }
        }
    }

    let cancel_rx = state.cancel_tx.subscribe();

    // 1. Получаем готовые субтитры из папки проекта (или создаем их, если пользователь сразу нажал экспорт в ручном режиме)
    let project_srt = crate::whisper::get_project_srt_path(&video_buf);
    let srt_path = if let Some(ref custom_path_str) = config.custom_subtitles_path {
        let custom_p = PathBuf::from(custom_path_str);
        if custom_p.exists() {
            let _ = crate::whisper::process_custom_subtitles_into_project(&custom_p, &project_srt);
            Some(project_srt)
        } else {
            let asr_model = config.asr_model_path.clone().map(PathBuf::from);
            crate::whisper::get_or_create_subtitles(
                video_buf.clone(),
                asr_model,
                app.clone(),
                cancel_rx.resubscribe(),
            ).await?
        }
    } else {
        let asr_model = config.asr_model_path.clone().map(PathBuf::from);
        crate::whisper::get_or_create_subtitles(
            video_buf.clone(),
            asr_model,
            app.clone(),
            cancel_rx.resubscribe(),
        ).await?
    };

    // 2. Рендеринг клипов с учетом выбранного формата (9:16 или 16:9) и наложением синхронизированных субтитров
    let format_label = if config.vertical_format { "9:16" } else { "16:9" };
    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { 
        percent: 50.0, 
        stage: format!("Подготовка к нарезке клипов ({} + Субтитры)...", format_label)
    });

    let render_config = crate::render::RenderConfig {
        vertical_format: config.vertical_format,
        subtitle_style: config.subtitle_style.clone(),
        hw_accel: true,
    };

    // Определяем базовую директорию сохранения и создаем отдельные подпапки: Шортсы, Хайлайты, Ток-видео
    let file_stem = video_buf.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    let video_parent = video_buf.parent().unwrap_or_else(|| std::path::Path::new("."));
    let default_clips_root = video_parent.join(format!("{}_Проект", file_stem)).join("Готовые клипы");
    let clips_root = match config.output_dir.as_deref().map(str::trim) {
        Some(custom) if !custom.is_empty() && custom != "output_cuts/" && custom != "output_cuts" => {
            let custom_path = PathBuf::from(custom);
            if custom_path.is_absolute() {
                custom_path
            } else {
                video_parent.join(custom_path)
            }
        }
        _ => default_clips_root,
    };
    let clips_dir = clips_root.join("Шортсы");
    let _ = tokio::fs::create_dir_all(&clips_dir).await;

    crate::logger::emit_log(
        &app,
        "Render",
        "info",
        &format!("Директория сохранения готовых шортсов: '{}'", clips_dir.display()),
    );

    let srt_fragments = if let Some(ref srt) = srt_path {
        tokio::fs::read_to_string(srt)
            .await
            .ok()
            .map(|c| crate::text_analyzer::parse_srt(&c))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let audio_profile_opt = crate::audio_analyzer::get_or_create_audio_profile(
        &video_buf,
        &srt_fragments,
        &app,
        cancel_rx.resubscribe(),
    )
    .await
    .ok();

    let mut generated_clips = Vec::new();
    let total_fragments = fragments.len().max(1);

    for (i, frag) in fragments.iter().enumerate() {
        let pct = 50.0 + ((i as f64 + 1.0) / total_fragments as f64) * 48.0;
        let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { 
            percent: pct, 
            stage: format!("Рендеринг клипа {} из {} ({})...", i + 1, fragments.len(), format_label) 
        });

        let (start_sec, end_sec) = if let Some(ref prof) = audio_profile_opt {
            let max_d = (frag.end - frag.start).max(8.0);
            let min_d = (max_d * 0.5).clamp(4.0, 10.0);
            let (ns, ne, _, _) = crate::audio_analyzer::refine_clip_bounds_by_audio(
                prof,
                frag.start,
                frag.end,
                &srt_fragments,
                0.25,
                min_d,
                max_d + 1.0,
            );
            (ns, ne)
        } else {
            (frag.start, frag.end)
        };
        let duration = (end_sec - start_sec).max(0.5);

        let start_min = (start_sec / 60.0) as u32;
        let start_s = (start_sec % 60.0) as u32;
        let end_min = (end_sec / 60.0) as u32;
        let end_s = (end_sec % 60.0) as u32;

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

        let _ = crate::render::attach_bumpers_if_enabled(
            &clip_path,
            config.bumper_config.as_ref(),
            config.vertical_format,
            &video_buf,
            &app,
        ).await;

        generated_clips.push(clip_path.to_string_lossy().to_string());
    }

    let selected_chars = config.selected_characters.as_deref().unwrap_or(&[]);
    let has_char_filter = !selected_chars.is_empty()
        && !selected_chars.iter().any(|c| c.to_lowercase() == "all" || c == "Все персонажи");
    let char_prefix = if has_char_filter {
        format!("{}_", selected_chars.first().cloned().unwrap_or_default())
    } else {
        String::new()
    };

    // Рендеринг Хайлайтов (16:9 YouTube) при ручном экспорте
    if config.render_highlights.unwrap_or(false) && !fragments.is_empty() {
        let highlights_dir = clips_root.join("Хайлайты");
        let _ = tokio::fs::create_dir_all(&highlights_dir).await;
        let highlights_path = highlights_dir.join(format!("Хайлайты_{}{}.mp4", char_prefix, file_stem));

        let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { 
            percent: 98.0, 
            stage: "Экспорт сборника хайлайтов (16:9 Full HD + Crossfade 0.4с + субтитры)...".to_string() 
        });

        let intervals: Vec<(f64, f64)> = fragments.iter().map(|f| (f.start, f.end)).collect();
        if let Ok(()) = crate::render::render_highlights_assembly(
            video_buf.clone(),
            &intervals,
            highlights_path.clone(),
            srt_path.clone(),
            false, // Хайлайты — ВСЕГДА полноэкранный 16:9 альбомный ролик из оригинального видео
            config.subtitle_style.clone(),
            app.clone(),
            cancel_rx.resubscribe(),
        ).await {
            let _ = crate::render::attach_bumpers_if_enabled(
                &highlights_path,
                config.bumper_config.as_ref(),
                false, // 16:9 альбомный пресет бамперов
                &video_buf,
                &app,
            ).await;
            generated_clips.push(highlights_path.to_string_lossy().to_string());
        }
    }

    // Рендеринг Разговорного видео (Ток-видео / Демки) при ручном экспорте
    if config.render_talk_video.unwrap_or(false) && srt_path.is_some() {
        let talk_dir = clips_root.join("Ток-видео");
        let _ = tokio::fs::create_dir_all(&talk_dir).await;
        let talk_label = if has_char_filter { "Демка" } else { "Ток_видео" };
        let talk_path = talk_dir.join(format!("{}_{}{}.mp4", talk_label, char_prefix, file_stem));

        let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { 
            percent: 99.0, 
            stage: format!("Экспорт разговорного видео ({}, 16:9 без пауз)...", talk_label) 
        });

        if let Ok(()) = crate::render::render_talk_video_assembly(
            video_buf.clone(),
            srt_path.clone(),
            config.selected_characters.as_deref(),
            false, // Демка и Ток-видео — ВСЕГДА полноэкранный 16:9 ролик из оригинального видео
            config.subtitle_style.clone(),
            talk_path.clone(),
            app.clone(),
            cancel_rx.resubscribe(),
        ).await {
            let _ = crate::render::attach_bumpers_if_enabled(
                &talk_path,
                config.bumper_config.as_ref(),
                false, // 16:9 альбомный пресет бамперов
                &video_buf,
                &app,
            ).await;
            generated_clips.push(talk_path.to_string_lossy().to_string());
        }
    }

    let _ = app.emit("pipeline-progress", crate::pipeline::orchestrator::PipelineProgressPayload { 
        percent: 100.0, 
        stage: "Экспорт всех роликов успешно завершен!".to_string() 
    });

    Ok(generated_clips)
}

/// Асинхронная команда: Парсинг загруженных готовых субтитров (ASS, SSA, SRT, VTT, SUB) по пути к файлу
#[tauri::command]
pub async fn parse_custom_subtitles(
    file_path: String,
) -> AppResult<crate::subtitles_parser::CustomSubtitlesParseResult> {
    log::info!("IPC: parse_custom_subtitles: {}", file_path);
    let p = PathBuf::from(&file_path);
    if !p.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Файл субтитров не найден: {}", file_path)));
    }
    crate::subtitles_parser::parse_custom_subtitles_file(&p)
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!(e)))
}

/// Асинхронная команда: Парсинг загруженных субтитров напрямую из содержимого файла (сохраняет копию во временную папку приложения)
#[tauri::command]
pub async fn parse_custom_subtitles_content(
    app: AppHandle,
    file_name: String,
    content: String,
) -> AppResult<crate::subtitles_parser::CustomSubtitlesParseResult> {
    use tauri::Manager;
    let app_data = app.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir());
    let subs_dir = app_data.join("uploaded_subtitles");
    let _ = std::fs::create_dir_all(&subs_dir);
    let safe_name: String = file_name
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    let dest_path = subs_dir.join(if safe_name.is_empty() { "custom_subtitles.ass".to_string() } else { safe_name });
    std::fs::write(&dest_path, content.as_bytes()).map_err(AppError::Io)?;
    crate::subtitles_parser::parse_custom_subtitles_file(&dest_path)
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!(e)))
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
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        log::info!(
            "Платформа не поддерживает десктопный файловый менеджер, возвращаем путь: {}",
            existing_dir.display()
        );
    }

    Ok(existing_dir.to_string_lossy().to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiProfile {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub available_models: Vec<String>,
}

pub fn resolve_openai_profiles_path(app: &AppHandle) -> PathBuf {
    use tauri::Manager;
    let dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let _ = std::fs::create_dir_all(&dir);
    dir.join("openai_profiles.json")
}

/// Загрузка сохранённых локально на компьютере профилей OpenAI API
#[tauri::command]
pub async fn get_openai_profiles(app: AppHandle) -> AppResult<Vec<OpenAiProfile>> {
    let path = resolve_openai_profiles_path(&app);
    if !path.exists() {
        return Ok(Vec::new());
    }
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => match serde_json::from_str::<Vec<OpenAiProfile>>(&content) {
            Ok(profiles) => Ok(profiles),
            Err(e) => {
                log::warn!("Не удалось десериализовать профили OpenAI из {:?}: {}", path, e);
                Ok(Vec::new())
            }
        },
        Err(e) => {
            log::warn!("Не удалось прочитать файл профилей OpenAI {:?}: {}", path, e);
            Ok(Vec::new())
        }
    }
}

/// Сохранение профилей OpenAI API локально на компьютере
#[tauri::command]
pub async fn save_openai_profiles(
    app: AppHandle,
    profiles: Vec<OpenAiProfile>,
) -> AppResult<String> {
    let path = resolve_openai_profiles_path(&app);
    let json = serde_json::to_string_pretty(&profiles)
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка сериализации профилей OpenAI: {}", e)))?;
    tokio::fs::write(&path, json.as_bytes())
        .await
        .map_err(AppError::Io)?;
    crate::logger::emit_log(
        &app,
        "System",
        "info",
        &format!(
            "Сохранено {} профилей OpenAI API в локальную память: '{}'",
            profiles.len(),
            path.display()
        ),
    );
    Ok(path.to_string_lossy().to_string())
}

fn build_openai_models_urls(base_url: &str) -> Vec<String> {
    let mut trimmed = base_url.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        trimmed = "https://api.openai.com/v1".to_string();
    }

    for suffix in ["/chat/completions", "/completions", "/models"] {
        if let Some(stripped) = trimmed.strip_suffix(suffix) {
            trimmed = stripped.trim_end_matches('/').to_string();
        }
    }

    let mut urls = Vec::new();
    if trimmed.ends_with("/v1")
        || trimmed.ends_with("/v1beta")
        || trimmed.ends_with("/v4")
        || trimmed.ends_with("/openai")
    {
        urls.push(format!("{}/models", trimmed));
    } else {
        urls.push(format!("{}/v1/models", trimmed));
        urls.push(format!("{}/models", trimmed));
    }

    // Дополнительный fallback для локальных серверов типа Ollama (/api/tags)
    if let Some(root) = trimmed.strip_suffix("/v1") {
        urls.push(format!("{}/api/tags", root.trim_end_matches('/')));
    } else {
        urls.push(format!("{}/api/tags", trimmed));
    }

    urls
}

fn extract_models_from_json(val: &serde_json::Value) -> Vec<String> {
    let mut models = Vec::new();
    let mut seen = std::collections::HashSet::new();

    fn extract_from_array(
        arr: &[serde_json::Value],
        models: &mut Vec<String>,
        seen: &mut std::collections::HashSet<String>,
    ) {
        for item in arr {
            let raw_id = if let Some(s) = item.as_str() {
                Some(s)
            } else if let Some(obj) = item.as_object() {
                obj.get("id")
                    .or_else(|| obj.get("model"))
                    .or_else(|| obj.get("name"))
                    .and_then(|v| v.as_str())
            } else {
                None
            };
            if let Some(id) = raw_id {
                let clean = id.trim();
                if !clean.is_empty() && seen.insert(clean.to_string()) {
                    models.push(clean.to_string());
                }
            }
        }
    }

    if let Some(arr) = val.as_array() {
        extract_from_array(arr, &mut models, &mut seen);
    } else if let Some(obj) = val.as_object() {
        if let Some(arr) = obj.get("data").and_then(|v| v.as_array()) {
            extract_from_array(arr, &mut models, &mut seen);
        }
        if models.is_empty() {
            if let Some(arr) = obj.get("models").and_then(|v| v.as_array()) {
                extract_from_array(arr, &mut models, &mut seen);
            }
        }
    }

    models
}

/// Запрос списка доступных Model ID напрямую с OpenAI-совместимого сервера
#[tauri::command]
pub async fn fetch_openai_models(
    app: AppHandle,
    base_url: String,
    api_key: String,
) -> AppResult<Vec<String>> {
    let candidate_urls = build_openai_models_urls(&base_url);
    let clean_key = api_key.trim();

    crate::logger::emit_log(
        &app,
        "LLM",
        "info",
        &format!(
            "Запрос списка доступных моделей с сервера: {}",
            candidate_urls.first().cloned().unwrap_or_default()
        ),
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка создания HTTP-клиента: {}", e)))?;

    let mut last_error = String::from("Сервер не вернул список моделей");

    for url in &candidate_urls {
        let mut req = client
            .get(url)
            .header("Accept", "application/json");

        if !clean_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", clean_key));
        }

        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();

                if !status.is_success() {
                    let preview: String = body_text.chars().take(180).collect();
                    last_error = format!("HTTP {} ({}): {}", status, url, preview);
                    // Если 401/403 (неверный API-ключ), сразу сообщаем пользователю без перебора остальных путей
                    if status.as_u16() == 401 || status.as_u16() == 403 {
                        break;
                    }
                    continue;
                }

                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&body_text) {
                    let models = extract_models_from_json(&json_val);
                    if !models.is_empty() {
                        crate::logger::emit_log(
                            &app,
                            "LLM",
                            "info",
                            &format!("✓ Получено {} моделей с сервера {}", models.len(), url),
                        );
                        return Ok(models);
                    } else {
                        last_error = format!("Сервер {} ответил успешно, но список моделей пуст", url);
                    }
                } else {
                    last_error = format!("Сервер {} вернул некорректный JSON", url);
                }
            }
            Err(e) => {
                last_error = format!("Ошибка подключения к {}: {}", url, e);
            }
        }
    }

    crate::logger::emit_log(
        &app,
        "LLM",
        "warn",
        &format!("Не удалось получить список моделей: {}", last_error),
    );

    Err(AppError::Anyhow(anyhow::anyhow!(last_error)))
}

/// Асинхронная команда: Предрендеринг или генерация предпросмотра заставки (преролла/постролла)
#[tauri::command]
pub async fn render_bumper_preview(
    app: AppHandle,
    item: crate::bumpers::BumperItemConfig,
    is_vertical: bool,
    source_video_path: String,
    preset_id: String,
    bumper_type: String,
) -> AppResult<String> {
    log::info!("IPC: render_bumper_preview (type={}, vertical={}): preset_id={}", bumper_type, is_vertical, preset_id);
    let video_buf = PathBuf::from(&source_video_path);
    
    use tauri::Manager;
    let app_data = app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."));
    let fmt_tag = if is_vertical { "916" } else { "169" };
    let cache_dir = app_data.join("bumpers_cache").join(&preset_id);
    let _ = tokio::fs::create_dir_all(&cache_dir).await;
    let out_path = cache_dir.join(format!("{}_{}.mp4", bumper_type, fmt_tag));

    crate::bumpers::render_single_bumper_video(
        &item,
        is_vertical,
        &video_buf,
        &out_path,
        &app
    ).await?;

    // Автоматически синхронизируем состояние заставок на диск: пресет активен и включен
    let mut disk_cfg = crate::bumpers::load_saved_bumper_config(&app).unwrap_or_else(|| {
        let mut def = crate::bumpers::BumperGlobalConfig {
            enabled: true,
            active_preset_id: preset_id.clone(),
            presets: Vec::new(),
        };
        def.ensure_populated();
        def
    });
    disk_cfg.enabled = true;
    disk_cfg.active_preset_id = preset_id.clone();
    if let Some(p) = disk_cfg.presets.iter_mut().find(|p| p.id == preset_id) {
        let cat = if is_vertical { &mut p.v916 } else { &mut p.v169 };
        if bumper_type == "preroll" {
            cat.preroll = item.clone();
            cat.preroll.enabled = true;
        } else {
            cat.postroll = item.clone();
            cat.postroll.enabled = true;
        }
    }
    let _ = crate::bumpers::save_bumper_config_to_disk(&app, &disk_cfg);

    Ok(out_path.to_string_lossy().to_string())
}

/// Асинхронная команда: Сохранение глобальной конфигурации заставок на диск
#[tauri::command]
pub async fn save_bumper_config(
    app: AppHandle,
    config: crate::bumpers::BumperGlobalConfig,
) -> AppResult<()> {
    crate::bumpers::save_bumper_config_to_disk(&app, &config)
}

/// Асинхронная команда: Загрузка глобальной конфигурации заставок с диска
#[tauri::command]
pub async fn load_bumper_config(
    app: AppHandle,
) -> AppResult<crate::bumpers::BumperGlobalConfig> {
    if let Some(cfg) = crate::bumpers::load_saved_bumper_config(&app) {
        Ok(cfg)
    } else {
        let mut def = crate::bumpers::BumperGlobalConfig {
            enabled: true,
            active_preset_id: "default".to_string(),
            presets: Vec::new(),
        };
        def.ensure_populated();
        Ok(def)
    }
}

/// Асинхронная команда: Определение точной длительности загруженного аудиофайла через FFmpeg
#[tauri::command]
pub async fn probe_audio_file_duration(
    app: AppHandle,
    audio_path: String,
) -> AppResult<f64> {
    let p = PathBuf::from(&audio_path);
    if !p.exists() {
        return Ok(2.5);
    }
    Ok(crate::bumpers::probe_audio_duration(&p, &app).await)
}

/// Асинхронная команда: Экспорт пресета преролла и постролла со всеми медиафайлами в единый архив .vcpb (.zip)
#[tauri::command]
pub async fn export_bumper_preset_bundle(
    preset: crate::bumpers::BumperPreset,
    dest_path: String,
) -> AppResult<String> {
    let dest = PathBuf::from(&dest_path);
    crate::bumpers::export_preset_to_vcpb_file(&preset, &dest)
}

/// Асинхронная команда: Импорт пресета преролла и постролла из архива .vcpb (.zip) со всеми медиафайлами
#[tauri::command]
pub async fn import_bumper_preset_bundle(
    app: AppHandle,
    archive_path: String,
) -> AppResult<crate::bumpers::BumperPreset> {
    use tauri::Manager;
    let app_data = app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."));
    let src = PathBuf::from(&archive_path);
    if !src.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Файл архива пресета не найден: {}", archive_path)));
    }
    crate::bumpers::import_preset_from_vcpb_file(&src, &app_data)
}

/// Асинхронная команда: Ручная отправка выбранных рилсов/клипов на доработку через ИИ до рендеринга
#[tauri::command]
pub async fn repair_selected_fragments(
    app: AppHandle,
    video_path: String,
    fragments: Vec<FragmentDto>,
    repair_mode: Option<String>,
    config: PipelineConfig,
) -> AppResult<Vec<FragmentDto>> {
    crate::logger::emit_log(
        &app,
        "Pipeline",
        "info",
        &format!("Ручная отправка {} клипов на доработку через ИИ (режим: {:?})...", fragments.len(), repair_mode),
    );

    let video_buf = PathBuf::from(&video_path);
    if !video_buf.exists() {
        return Err(AppError::Anyhow(anyhow::anyhow!("Файл видео не найден: {}", video_path)));
    }

    let video_duration = match crate::ffmpeg::probe_video(video_buf.clone(), app.clone()).await {
        Ok(meta) if meta.duration > 1.0 => meta.duration,
        _ => 180.0,
    };

    // Загружаем готовые субтитры проекта
    let project_srt = crate::whisper::get_project_srt_path(&video_buf);
    let srt_content = if project_srt.exists() {
        tokio::fs::read_to_string(&project_srt).await.unwrap_or_default()
    } else if let Some(ref custom_path_str) = config.custom_subtitles_path {
        let cp = PathBuf::from(custom_path_str);
        if cp.exists() {
            tokio::fs::read_to_string(&cp).await.unwrap_or_default()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let all_subtitles = crate::text_analyzer::parse_srt(&srt_content);

    // Загружаем аудио-профиль если есть в кэше проекта
    let audio_prof_path = crate::audio_analyzer::get_project_audio_profile_path(&video_buf);
    let audio_profile_opt: Option<crate::audio_analyzer::AudioEnergyProfile> = if audio_prof_path.exists() {
        std::fs::read_to_string(&audio_prof_path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
    } else {
        None
    };

    // Текстовые баллы из кэша (если есть)
    let file_stem = video_buf.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    let video_parent = video_buf.parent().unwrap_or_else(|| std::path::Path::new("."));
    let project_dir = video_parent.join(format!("{}_Проект", file_stem));
    let cache_dir = project_dir.join("Кэш_анализа");
    let text_scores_path = cache_dir.join("text_scores_cache.json");
    let text_scores: Vec<f64> = if text_scores_path.exists() {
        std::fs::read_to_string(&text_scores_path)
            .ok()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
            .and_then(|v| v.get("scores").and_then(|s| serde_json::from_value(s.clone()).ok()))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let min_dur = (config.max_clip_duration_sec as f64 * 0.45).clamp(8.0, 20.0);
    let max_dur = (config.max_clip_duration_sec as f64).clamp(15.0, 90.0);
    let intro_skip = 0.0;
    let outro_limit = video_duration;

    let target_defect = match repair_mode.as_deref() {
        Some("boring") => crate::text_analyzer::ClipDefectType::BoringOrWeakDynamics,
        Some("incomplete_thought") => crate::text_analyzer::ClipDefectType::IncompleteThought,
        _ => crate::text_analyzer::ClipDefectType::IncompleteThought,
    };

    let mut repaired_results: Vec<FragmentDto> = Vec::new();

    for (idx, frag) in fragments.iter().enumerate() {
        let f_start = frag.start;
        let f_end = frag.end;

        if all_subtitles.is_empty() {
            // Без субтитров: эластичное расширение границ на 1.5с
            let new_s = (f_start - 1.0).max(0.0);
            let new_e = (f_end + 1.5).min(video_duration);
            let diff = (new_e - new_s) - (f_end - f_start);
            let sign = if diff >= 0.0 { "+" } else { "" };
            repaired_results.push(FragmentDto {
                start: new_s,
                end: new_e,
                title: frag.title.clone().map(|t| format!("{} (✓ Доработан)", t.replace(" (✓ Доработан ИИ)", "").replace(" (✓ Доработан)", ""))),
                repaired: Some(true),
                repair_note: Some(format!("Скорректированы границы [{:.1}с - {:.1}с] ({}{:.1}с)", new_s, new_e, sign, diff)),
            });
            continue;
        }

        // Находим пересекающиеся индексы реплик субтитров
        let first_cue = all_subtitles.iter().position(|s| s.end > f_start + 0.05).unwrap_or(0);
        let last_cue = all_subtitles.iter().rposition(|s| s.start < f_end - 0.05).unwrap_or(first_cue);

        let mut dialogue_parts = Vec::new();
        for s in &all_subtitles[first_cue..=last_cue] {
            if !s.text.trim().is_empty() {
                dialogue_parts.push(s.text.trim().to_string());
            }
        }
        let first_phrase = all_subtitles.get(first_cue).map(|s| s.text.chars().take(40).collect()).unwrap_or_default();
        let last_phrase = all_subtitles.get(last_cue).map(|s| s.text.chars().take(40).collect()).unwrap_or_default();

        let candidate = crate::text_analyzer::AssembledClipCandidate {
            start: f_start,
            end: f_end,
            initial_score: 0.85,
            first_phrase,
            last_phrase,
            full_dialogue: dialogue_parts.join(" — "),
            first_cue_idx: Some(first_cue),
            last_cue_idx: Some(last_cue),
            cue_count: dialogue_parts.len(),
            round: 1,
            repair_note: None,
        };

        if let Some(repaired) = crate::text_analyzer::repair_clip_candidate(
            &candidate,
            &target_defect,
            &all_subtitles,
            &text_scores,
            audio_profile_opt.as_ref(),
            min_dur,
            max_dur,
            intro_skip,
            outro_limit,
            video_duration,
        ) {
            let mut final_start = repaired.start;
            let mut final_end = repaired.end;

            // Дополнительно выравниваем по тихим точкам в аудио
            if let Some(ref prof) = audio_profile_opt {
                let (refined_s, refined_e, _, _) = crate::audio_analyzer::refine_clip_bounds_by_audio(
                    prof,
                    final_start,
                    final_end,
                    &all_subtitles,
                    1.2,
                    min_dur * 0.7,
                    max_dur + 5.0,
                );
                final_start = refined_s;
                final_end = refined_e;
            }

            let diff = (final_end - final_start) - (f_end - f_start);
            let sign = if diff >= 0.0 { "+" } else { "" };
            let note = repaired.repair_note.unwrap_or_else(|| {
                format!(
                    "Расширены границы [{:.1}с - {:.1}с] ({}{:.1}с) до законченного панчлайна",
                    final_start,
                    final_end,
                    sign,
                    diff
                )
            });

            crate::logger::emit_log(
                &app,
                "Pipeline",
                "info",
                &format!("  ✓ Клип #{} успешно доработан: [{:.1}с - {:.1}с] -> [{:.1}с - {:.1}с] ({})", idx + 1, f_start, f_end, final_start, final_end, note),
            );

            let old_title = frag.title.clone().unwrap_or_else(|| format!("Клип #{}", idx + 1));
            let clean_title = old_title.replace(" (✓ Доработан ИИ)", "").replace(" (✓ Доработан)", "");
            repaired_results.push(FragmentDto {
                start: final_start,
                end: final_end,
                title: Some(format!("{} (✓ Доработан ИИ)", clean_title)),
                repaired: Some(true),
                repair_note: Some(note),
            });
        } else {
            // Если границы уже упираются в лимит, мягко доводим по тихим точкам в окне
            let mut new_start = f_start;
            let mut new_end = f_end;
            if let Some(ref prof) = audio_profile_opt {
                let (refined_s, refined_e, _, _) = crate::audio_analyzer::refine_clip_bounds_by_audio(
                    prof,
                    new_start,
                    new_end,
                    &all_subtitles,
                    1.5,
                    min_dur * 0.7,
                    max_dur + 5.0,
                );
                new_start = refined_s;
                new_end = refined_e;
            } else {
                new_start = (f_start - 0.5).max(0.0);
                new_end = (f_end + 0.8).min(video_duration);
            }
            let diff = (new_end - new_start) - (f_end - f_start);
            let sign = if diff >= 0.0 { "+" } else { "" };
            let note = format!("Скорректированы границы [{:.1}с - {:.1}с] ({}{:.1}с) по границам речи", new_start, new_end, sign, diff);
            crate::logger::emit_log(
                &app,
                "Pipeline",
                "info",
                &format!("  ✓ Клип #{} скорректирован: [{:.1}с - {:.1}с]", idx + 1, new_start, new_end),
            );
            let old_title = frag.title.clone().unwrap_or_else(|| format!("Клип #{}", idx + 1));
            let clean_title = old_title.replace(" (✓ Доработан ИИ)", "").replace(" (✓ Доработан)", "");
            repaired_results.push(FragmentDto {
                start: new_start,
                end: new_end,
                title: Some(format!("{} (✓ Доработан ИИ)", clean_title)),
                repaired: Some(true),
                repair_note: Some(note),
            });
        }
    }

    Ok(repaired_results)
}
