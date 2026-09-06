// src-tauri/src/pipeline/orchestrator.rs
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use serde::{Deserialize, Serialize};
use crate::error::{AppResult, AppError};
use tokio::sync::broadcast::Receiver;
use crate::render::{RenderConfig, SubtitleStyle};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    pub asr_model_path: Option<String>,
    pub vision_model_path: Option<String>,
    pub text_engine: String,
    pub text_model_name: String,
    pub ollama_url: String,
    pub output_dir: Option<String>,
    pub min_intensity_threshold: f64,
    pub max_clip_duration_sec: f64,
    pub vertical_format: bool,
    pub subtitle_style: Option<SubtitleStyle>,
    pub mode: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct PipelineProgressPayload {
    pub percent: f64,
    pub stage: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SuggestedPayload {
    fragments: Vec<crate::commands::FragmentDto>,
}

pub async fn run_pipeline(
    video_path: PathBuf,
    config: PipelineConfig,
    app_handle: AppHandle,
    cancel_rx: Receiver<()>,
) -> AppResult<Vec<PathBuf>> {
    let mode = config.mode.clone().unwrap_or_else(|| "auto".to_string());
    log::info!("Запуск надежного конвейера видео-обработки в режиме: {}", mode);
    
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 2.0, 
        stage: "Инициализация конвейера...".to_string() 
    });

    // Генерируем временный путь для аудио
    let mut audio_output = video_path.clone();
    let file_stem = video_path.file_stem().and_then(|s| s.to_str()).unwrap_or("extracted");
    audio_output.set_file_name(format!("{}_pipeline_audio_{}.wav", file_stem, std::process::id()));

    // 1. Извлечение аудио-дорожки через FFmpeg (быстро и надежно)
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 10.0, 
        stage: "Извлечение аудио дорожки (FFmpeg)...".to_string() 
    });

    crate::ffmpeg::extract_audio(
        video_path.clone(),
        audio_output.clone(),
        app_handle.clone(),
        cancel_rx.resubscribe()
    ).await?;

    // 2. Распознавание речи через Whisper
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 30.0, 
        stage: "ASR: распознавание речи и титров (Whisper)...".to_string() 
    });

    let asr_model = config.asr_model_path.clone().map(PathBuf::from);
    let srt_path = match crate::whisper::transcribe_audio(
        audio_output.clone(),
        asr_model,
        app_handle.clone(),
        cancel_rx.resubscribe()
    ).await {
        Ok(path) => Some(path),
        Err(e) => {
            log::warn!("Whisper ASR завершился с предупреждением: {}. Продолжаем обработку.", e);
            None
        }
    };

    if mode == "manual" {
        // Очистка временного аудио
        let _ = tokio::fs::remove_file(&audio_output).await;
        let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
            percent: 100.0, 
            stage: "Ручной режим готов к редактированию".to_string() 
        });
        return Ok(Vec::new());
    }

    // 3. Анализ диалогов и семантики
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 60.0, 
        stage: "ИИ-анализ диалогов, эмоций и динамики...".to_string() 
    });

    let (fragments, text_scores) = if let Some(ref srt) = srt_path {
        if let Ok(srt_content) = tokio::fs::read_to_string(srt).await {
            let frags = crate::text_analyzer::parse_srt(&srt_content);
            let text_config = crate::text_analyzer::TextAnalysisConfig {
                engine: match config.text_engine.as_str() {
                    "ollama" => crate::text_analyzer::TextEngine::Ollama,
                    "llama" => crate::text_analyzer::TextEngine::LlamaCli,
                    "builtin" => crate::text_analyzer::TextEngine::Builtin,
                    _ => crate::text_analyzer::TextEngine::Builtin, // По умолчанию автономный встроенный
                },
                model_name: config.text_model_name.clone(),
                ollama_url: config.ollama_url.clone(),
            };

            let scores = match crate::text_analyzer::analyze_text_with_llm(
                &frags, 
                &text_config, 
                &app_handle, 
                cancel_rx.resubscribe()
            ).await {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("NLP анализ вернул ошибку: {}, используем быстрый расчет", e);
                    crate::text_analyzer::compute_text_scores_advanced(&frags)
                }
            };
            (frags, scores)
        } else {
            (Vec::new(), Vec::new())
        }
    } else {
        (Vec::new(), Vec::new())
    };

    // 4. Опциональный визуальный анализ (адаптивно, если есть модель)
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 75.0, 
        stage: "Визуальный анализ ключевых сцен...".to_string() 
    });

    let vision_model = config.vision_model_path.clone().map(PathBuf::from);
    let scenes = crate::analyzer::analyze_video(
        video_path.clone(),
        vision_model,
        app_handle.clone(),
        cancel_rx.resubscribe()
    ).await.unwrap_or_default();

    // 5. Построение списка лучших фрагментов (Хайлайтов)
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 85.0, 
        stage: "Формирование лучших моментов (хайлайтов)...".to_string() 
    });

    let mut candidate_intervals: Vec<(f64, f64, f64)> = Vec::new(); // (start, end, score)

    // А. Из реплик и диалогов с высоким text_score
    for (i, frag) in fragments.iter().enumerate() {
        let score = text_scores.get(i).copied().unwrap_or(0.3);
        if score >= config.min_intensity_threshold {
            let start = (frag.start - 2.0).max(0.0);
            let end = start + config.max_clip_duration_sec;
            candidate_intervals.push((start, end, score));
        }
    }

    // Б. Из сцен визуального анализа
    for scene in &scenes {
        let start = (scene.timestamp_sec - 3.0).max(0.0);
        let end = start + config.max_clip_duration_sec;
        if scene.intensity >= config.min_intensity_threshold {
            candidate_intervals.push((start, end, scene.intensity));
        }
    }

    // Если нет ни одного фрагмента выше порога (например, порог высокий или видео без слов),
    // берем топ-3 самых ярких интервала или равномерные отрезки
    if candidate_intervals.is_empty() {
        if !fragments.is_empty() {
            for (i, frag) in fragments.iter().take(5).enumerate() {
                let score = text_scores.get(i).copied().unwrap_or(0.5);
                candidate_intervals.push((frag.start.max(0.0), frag.start + config.max_clip_duration_sec, score));
            }
        } else {
            // Равномерные 3 клипа
            candidate_intervals.push((0.0, config.max_clip_duration_sec, 0.7));
            candidate_intervals.push((config.max_clip_duration_sec * 2.0, config.max_clip_duration_sec * 3.0, 0.6));
        }
    }

    // Устраняем сильные перекрытия (Non-Maximum Suppression)
    candidate_intervals.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let mut selected_intervals: Vec<(f64, f64, f64)> = Vec::new();

    for candidate in candidate_intervals {
        let overlaps = selected_intervals.iter().any(|sel| {
            let overlap_start = candidate.0.max(sel.0);
            let overlap_end = candidate.1.min(sel.1);
            overlap_end > overlap_start && (overlap_end - overlap_start) > 5.0
        });

        if !overlaps {
            selected_intervals.push(candidate);
        }
        if selected_intervals.len() >= 10 {
            break; // максимум 10 клипов
        }
    }

    // Сортируем по времени начала
    selected_intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let suggested_fragments: Vec<crate::commands::FragmentDto> = selected_intervals
        .iter()
        .map(|(s, e, _)| crate::commands::FragmentDto { start: *s, end: *e })
        .collect();

    // Отправляем найденные хайлайты во фронтенд для предпросмотра
    let _ = app_handle.emit("suggested-fragments", SuggestedPayload {
        fragments: suggested_fragments.clone(),
    });

    if mode == "confirm" {
        let _ = tokio::fs::remove_file(&audio_output).await;
        let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
            percent: 100.0, 
            stage: "Хайлайты успешно найдены и готовы к предпросмотру".to_string() 
        });
        return Ok(Vec::new());
    }

    // 6. Рендеринг клипов в автоматическом режиме
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 88.0, 
        stage: "Рендеринг готовых видеоклипов (FFmpeg)...".to_string() 
    });

    let mut generated_clips = Vec::new();
    let render_config = RenderConfig {
        vertical_format: config.vertical_format,
        subtitle_style: config.subtitle_style.clone(),
        hw_accel: true,
    };

    let base_out_dir = config.output_dir.clone().map(PathBuf::from).unwrap_or_else(|| {
        let mut d = video_path.clone();
        d.pop();
        d
    });

    let total_clips = selected_intervals.len();
    for (i, (start_sec, end_sec, _)) in selected_intervals.iter().enumerate() {
        let duration = end_sec - start_sec;
        let mut clip_path = base_out_dir.clone();
        clip_path.push(format!("{}_clip_{}_start_{}s.mp4", file_stem, i + 1, *start_sec as u32));

        let clip_percent = 88.0 + ((i as f64 + 1.0) / total_clips as f64) * 10.0;
        let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload {
            percent: clip_percent,
            stage: format!("Рендеринг клипа {} из {}...", i + 1, total_clips),
        });

        crate::render::render_clip(
            video_path.clone(),
            *start_sec,
            duration,
            clip_path.clone(),
            srt_path.clone(),
            render_config.clone(),
            app_handle.clone(),
            cancel_rx.resubscribe()
        ).await?;

        generated_clips.push(clip_path);
    }

    // Очистка временного аудио
    let _ = tokio::fs::remove_file(&audio_output).await;

    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 100.0, 
        stage: "Обработка завершена успешно!".to_string() 
    });

    Ok(generated_clips)
}

