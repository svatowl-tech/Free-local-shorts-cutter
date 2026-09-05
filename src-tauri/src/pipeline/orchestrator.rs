// src-tauri/src/pipeline/orchestrator.rs
use std::path::PathBuf;
use tauri::{AppHandle, Emit};
use serde::{Deserialize, Serialize};
use crate::error::{AppResult, AppError};
use tokio::sync::broadcast::Receiver;
use tokio::try_join;
use crate::render::{RenderConfig, SubtitleStyle};
use crate::analyzer::SceneCandidate;

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

pub async fn run_pipeline(
    video_path: PathBuf,
    config: PipelineConfig,
    app_handle: AppHandle,
    cancel_rx: Receiver<()>,
) -> AppResult<Vec<PathBuf>> {
    let mode = config.mode.clone().unwrap_or_else(|| "auto".to_string());
    log::info!("Запуск конвейера (Оркестратор) в режиме: {}", mode);
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { percent: 0.0, stage: "Инициализация...".to_string() });

    // Генерируем временный путь для аудио
    let mut audio_output = video_path.clone();
    let file_stem = video_path.file_stem().and_then(|s| s.to_str()).unwrap_or("extracted");
    audio_output.set_file_name(format!("{}_pipeline_audio.wav", file_stem));

    if mode == "manual" {
        // В ручном режиме мы извлекаем только аудио и делаем ASR (транскрипцию) для титров
        let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { percent: 10.0, stage: "Извлечение аудио дорожки...".to_string() });
        crate::ffmpeg::extract_audio(
            video_path.clone(),
            audio_output.clone(),
            app_handle.clone(),
            cancel_rx.resubscribe()
        ).await?;

        let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { percent: 50.0, stage: "ASR: генерация субтитров...".to_string() });
        let asr_model = config.asr_model_path.clone().map(PathBuf::from);
        let _srt_path = crate::whisper::transcribe_audio(
            audio_output.clone(),
            asr_model,
            app_handle.clone(),
            cancel_rx.resubscribe()
        ).await?;

        // Очистка временного аудио
        let _ = tokio::fs::remove_file(&audio_output).await;

        let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { percent: 100.0, stage: "Инициализация ручного режима завершена".to_string() });
        return Ok(Vec::new());
    }

    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { percent: 10.0, stage: "Извлечение аудио и визуальный анализ...".to_string() });

    // 1. Асинхронные задачи извлечения аудио и анализа видео (параллельно)
    let audio_future = crate::ffmpeg::extract_audio(
        video_path.clone(),
        audio_output.clone(),
        app_handle.clone(),
        cancel_rx.resubscribe()
    );

    let vision_model = config.vision_model_path.map(PathBuf::from);
    let analyzer_future = crate::analyzer::analyze_video(
        video_path.clone(),
        vision_model,
        app_handle.clone(),
        cancel_rx.resubscribe()
    );

    // Дожидаемся обе функции параллельно
    let (_, scenes) = try_join!(audio_future, analyzer_future)?;

    // 2. Распознавание речи
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { percent: 40.0, stage: "ASR: транскрипция...".to_string() });

    let asr_model = config.asr_model_path.map(PathBuf::from);
    let srt_path = crate::whisper::transcribe_audio(
        audio_output.clone(),
        asr_model,
        app_handle.clone(),
        cancel_rx.resubscribe()
    ).await?;

    // 3. Анализ текста (NLP)
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { percent: 55.0, stage: "Анализ текста LLM...".to_string() });

    let srt_content = tokio::fs::read_to_string(&srt_path).await.map_err(|e| AppError::Pipeline(format!("Не удалось прочитать SRT: {}", e)))?;
    let fragments = crate::text_analyzer::parse_srt(&srt_content);

    let text_scores = if config.text_engine == "none" || config.text_engine == "false" || config.text_engine == "disabled" {
        log::info!("Текстовый анализ отключен");
        vec![0.0; fragments.len()]
    } else {
        let text_config = crate::text_analyzer::TextAnalysisConfig {
            engine: match config.text_engine.as_str() {
                "ollama" => crate::text_analyzer::TextEngine::Ollama,
                "llama" => crate::text_analyzer::TextEngine::LlamaCli,
                _ => crate::text_analyzer::TextEngine::Auto,
            },
            model_name: config.text_model_name.clone(),
            ollama_url: config.ollama_url.clone(),
        };

        match crate::text_analyzer::analyze_text_with_llm(&fragments, &text_config, &app_handle, cancel_rx.resubscribe()).await {
            Ok(scores) => scores,
            Err(e) => {
                if format!("{:?}", e).contains("Отменено") {
                    return Err(e);
                }
                log::warn!("analyze_text_with_llm вернул ошибку: {}, используем fallback", e);
                crate::text_analyzer::compute_text_scores_fallback(&fragments)
            }
        }
    };

    if mode == "confirm" {
        // Формируем рекомендации и шлем событие без рендеринга
        let mut suggested_fragments = Vec::new();
        for scene in scenes.iter() {
            let start_sec = (scene.timestamp_sec - 5.0).max(0.0);
            let duration = config.max_clip_duration_sec;
            let end_sec = start_sec + duration;

            let mut max_text_score = 0.0_f64;
            for (frag_idx, frag) in fragments.iter().enumerate() {
                if frag.start < end_sec && frag.end > start_sec {
                    if let Some(&sc) = text_scores.get(frag_idx) {
                        if sc > max_text_score {
                            max_text_score = sc;
                        }
                    }
                }
            }

            let final_score = scene.intensity.max(max_text_score);
            if final_score >= config.min_intensity_threshold {
                suggested_fragments.push(crate::commands::FragmentDto {
                    start: start_sec,
                    end: end_sec,
                });
            }
        }

        #[derive(Serialize, Deserialize, Clone, Debug)]
        struct SuggestedPayload {
            fragments: Vec<crate::commands::FragmentDto>,
        }

        let _ = app_handle.emit("suggested-fragments", SuggestedPayload { fragments: suggested_fragments });
        
        // Очищаем временный аудио-звук
        let _ = tokio::fs::remove_file(&audio_output).await;

        let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { percent: 100.0, stage: "Предложения сформированы".to_string() });
        return Ok(Vec::new());
    }

    // 4. Подготовка к рендерингу (нарезке)
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { percent: 70.0, stage: "Подготовка и рендеринг клипов...".to_string() });

    let mut generated_clips = Vec::new();
    let render_config = RenderConfig {
        vertical_format: config.vertical_format,
        subtitle_style: config.subtitle_style.clone(),
        hw_accel: true,
    };

    // Определяем выходную директорию
    let base_out_dir = config.output_dir.clone().map(PathBuf::from).unwrap_or_else(|| {
        let mut d = video_path.clone();
        d.pop();
        d
    });

    // 5. Фильтрация кандидатов и рендеринг
    for (i, scene) in scenes.iter().enumerate() {
        let start_sec = (scene.timestamp_sec - 5.0).max(0.0);
        let duration = config.max_clip_duration_sec;
        let end_sec = start_sec + duration;

        // Определяем text_score для сцены
        let mut max_text_score = 0.0_f64;
        for (frag_idx, frag) in fragments.iter().enumerate() {
            if frag.start < end_sec && frag.end > start_sec {
                if let Some(&sc) = text_scores.get(frag_idx) {
                    if sc > max_text_score {
                        max_text_score = sc;
                    }
                }
            }
        }

        let final_score = scene.intensity.max(max_text_score);

        if final_score >= config.min_intensity_threshold {
            let mut clip_path = base_out_dir.clone();
            clip_path.push(format!("{}_clip_{}_start_{}s.mp4", file_stem, i, start_sec as u32));
            
            crate::render::render_clip(
                video_path.clone(),
                start_sec,
                duration,
                clip_path.clone(),
                Some(srt_path.clone()),
                render_config.clone(),
                app_handle.clone(),
                cancel_rx.resubscribe()
            ).await?;
            
            generated_clips.push(clip_path);
        }
    }

    // Очистка временного аудио (по желанию)
    let _ = tokio::fs::remove_file(&audio_output).await;

    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { percent: 100.0, stage: "Пайплайн завершен".to_string() });

    Ok(generated_clips)
}
