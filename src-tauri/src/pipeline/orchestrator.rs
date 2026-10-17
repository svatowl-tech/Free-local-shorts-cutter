// src-tauri/src/pipeline/orchestrator.rs
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use serde::{Deserialize, Serialize};
use crate::error::AppResult;
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
) -> AppResult<(Vec<PathBuf>, Vec<crate::commands::FragmentDto>)> {
    let mode = config.mode.clone().unwrap_or_else(|| "auto".to_string());
    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        &format!("=== СТАРТ ПАЙПЛАЙНА ОБРАБОТКИ ВИДЕО: '{}' (Режим: {}, Текстовый движок: {}) ===", video_path.display(), mode, config.text_engine),
    );
    
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 2.0, 
        stage: "Инициализация конвейера...".to_string() 
    });

    // Определяем длительность и метаданные видео через FFmpeg
    let video_duration = match crate::ffmpeg::probe_video(video_path.clone(), app_handle.clone()).await {
        Ok(meta) if meta.duration > 1.0 => {
            crate::logger::emit_log(
                &app_handle,
                "Pipeline",
                "info",
                &format!("Метаданные видео: длительность {:.2} сек, кодек: {}", meta.duration, meta.codec.as_deref().unwrap_or("unknown")),
            );
            meta.duration
        },
        _ => 180.0,
    };

    // Генерируем временный путь для аудио
    let mut audio_output = video_path.clone();
    let file_stem = video_path.file_stem().and_then(|s| s.to_str()).unwrap_or("extracted");
    audio_output.set_file_name(format!("{}_pipeline_audio_{}.wav", file_stem, std::process::id()));

    // 1. Извлечение аудио-дорожки через FFmpeg (быстро и надежно)
    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        "[Шаг 1/5] Извлечение аудио-дорожки в формат 16kHz WAV через FFmpeg...",
    );
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
    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        "[Шаг 2/5] Распознавание речи и формирование субтитров (Whisper ASR)...",
    );
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
        Ok(path) => {
            crate::logger::emit_log(
                &app_handle,
                "Pipeline",
                "info",
                &format!("Whisper успешно завершил транскрибацию: '{}'", path.display()),
            );
            Some(path)
        },
        Err(e) => {
            crate::logger::emit_log(
                &app_handle,
                "Pipeline",
                "warn",
                &format!("Whisper ASR завершился с предупреждением: {}. Продолжаем обработку на основе видео-сцен.", e),
            );
            None
        }
    };

    if mode == "manual" {
        // Очистка временного аудио
        let _ = tokio::fs::remove_file(&audio_output).await;
        crate::logger::emit_log(&app_handle, "Pipeline", "info", "Ручной режим: аудио извлечено, готово к пользовательскому редактированию.");
        let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
            percent: 100.0, 
            stage: "Ручной режим готов к редактированию".to_string() 
        });
        return Ok((Vec::new(), Vec::new()));
    }

    // 3. Анализ диалогов и семантики
    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        &format!("[Шаг 3/5] Семантический ИИ-анализ реплик (Движок: {})...", config.text_engine),
    );
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 60.0, 
        stage: "ИИ-анализ диалогов, эмоций и динамики...".to_string() 
    });

    let (fragments, text_scores) = if let Some(ref srt) = srt_path {
        if let Ok(srt_content) = tokio::fs::read_to_string(srt).await {
            let frags = crate::text_analyzer::parse_srt(&srt_content);
            if frags.is_empty() {
                crate::logger::emit_log(
                    &app_handle,
                    "LLM",
                    "info",
                    "Субтитры пусты: в аудиодорожке не обнаружено реплик. Анализ LLM пропущен.",
                );
                (Vec::new(), Vec::new())
            } else {
                crate::logger::emit_log(
                    &app_handle,
                    "LLM",
                    "info",
                    &format!("Найдено {} реплик в субтитрах. Запуск семантического анализа через движок '{}'...", frags.len(), config.text_engine),
                );
                let text_config = crate::text_analyzer::TextAnalysisConfig {
                    engine: match config.text_engine.as_str() {
                        "ollama" => crate::text_analyzer::TextEngine::Ollama,
                        "llama" => crate::text_analyzer::TextEngine::LlamaCli,
                        "builtin" => crate::text_analyzer::TextEngine::Builtin,
                        _ => crate::text_analyzer::TextEngine::Builtin,
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
                        crate::logger::emit_log(
                            &app_handle,
                            "LLM",
                            "warn",
                            &format!("Сбой анализа LLM: {}. Переход на высокоскоростной встроенный NLP расчет.", e),
                        );
                        crate::text_analyzer::compute_text_scores_advanced(&frags)
                    }
                };
                (frags, scores)
            }
        } else {
            (Vec::new(), Vec::new())
        }
    } else {
        crate::logger::emit_log(
            &app_handle,
            "LLM",
            "info",
            "Субтитры отсутствуют (Whisper не распознал речь). Текстовый анализ LLM пропущен, отбор хайлайтов выполняется по визуальным сценам.",
        );
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

    let clip_duration = config.max_clip_duration_sec.min(video_duration).max(5.0);
    let mut candidate_intervals: Vec<(f64, f64, f64)> = Vec::new(); // (start, end, score)

    // А. Из реплик и диалогов с высоким text_score
    for (i, frag) in fragments.iter().enumerate() {
        let score = text_scores.get(i).copied().unwrap_or(0.4);
        if score >= config.min_intensity_threshold {
            let start = (frag.start - 1.5).max(0.0).min((video_duration - 5.0).max(0.0));
            let end = (start + clip_duration).min(video_duration);
            if end - start >= 3.0 {
                candidate_intervals.push((start, end, score));
            }
        }
    }

    // Б. Из сцен визуального анализа
    for scene in &scenes {
        if scene.intensity >= config.min_intensity_threshold {
            let start = (scene.timestamp_sec - 2.0).max(0.0).min((video_duration - 5.0).max(0.0));
            let end = (start + clip_duration).min(video_duration);
            if end - start >= 3.0 {
                candidate_intervals.push((start, end, scene.intensity));
            }
        }
    }

    // Если нет ни одного фрагмента выше порога — выбираем топ лучших моментов
    if candidate_intervals.is_empty() {
        crate::logger::emit_log(
            &app_handle,
            "Pipeline",
            "info",
            &format!("Ни один фрагмент не превысил порог {}. Автоматический выбор моментов по относительной ценности...", config.min_intensity_threshold),
        );
        if !fragments.is_empty() {
            let mut frag_with_scores: Vec<(usize, f64)> = fragments
                .iter()
                .enumerate()
                .map(|(i, _)| (i, text_scores.get(i).copied().unwrap_or(0.5)))
                .collect();
            frag_with_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            for (idx, sc) in frag_with_scores.into_iter().take(5) {
                let frag = &fragments[idx];
                let start = (frag.start - 1.0).max(0.0).min((video_duration - 5.0).max(0.0));
                let end = (start + clip_duration).min(video_duration);
                if end - start >= 3.0 {
                    candidate_intervals.push((start, end, sc));
                }
            }
        } else if !scenes.is_empty() {
            for scn in scenes.iter().take(5) {
                let start = (scn.timestamp_sec - 1.0).max(0.0).min((video_duration - 5.0).max(0.0));
                let end = (start + clip_duration).min(video_duration);
                if end - start >= 3.0 {
                    candidate_intervals.push((start, end, scn.intensity));
                }
            }
        } else {
            let num_clips = if video_duration < 30.0 { 1 } else if video_duration < 90.0 { 2 } else { 3 };
            let step = video_duration / (num_clips as f64);
            for i in 0..num_clips {
                let start = (i as f64 * step).min(video_duration - 5.0).max(0.0);
                let end = (start + clip_duration).min(video_duration);
                if end - start >= 3.0 {
                    candidate_intervals.push((start, end, 0.75 - (i as f64 * 0.05)));
                }
            }
        }
    }

    // Устраняем сильные перекрытия (Non-Maximum Suppression): подавляем только если пересечение > 40%
    candidate_intervals.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let mut selected_intervals: Vec<(f64, f64, f64)> = Vec::new();
    for candidate in candidate_intervals {
        let overlaps = selected_intervals.iter().any(|sel| {
            let overlap_start = candidate.0.max(sel.0);
            let overlap_end = candidate.1.min(sel.1);
            let overlap_len = (overlap_end - overlap_start).max(0.0);
            let min_len = (candidate.1 - candidate.0).min(sel.1 - sel.0);
            min_len > 0.0 && (overlap_len / min_len) > 0.4
        });
        if !overlaps {
            selected_intervals.push(candidate);
        }
        if selected_intervals.len() >= 10 {
            break;
        }
    }

    // Абсолютная гарантия: если по какой-то причине selected_intervals пуст, создаем минимум 3 умных хайлайта по всей длине видео
    if selected_intervals.is_empty() && video_duration > 5.0 {
        crate::logger::emit_log(
            &app_handle,
            "Pipeline",
            "warn",
            "Внимание: не найдено интервалов по стандартным критериям. Задействован гарантированный алгоритм генерации топ-хайлайтов.",
        );
        let num_clips = if video_duration < 30.0 { 1 } else if video_duration < 90.0 { 3 } else { 5 };
        let step = video_duration / (num_clips as f64 + 1.0);
        for i in 1..=num_clips {
            let start = (step * (i as f64) - (clip_duration / 2.0)).max(0.0).min(video_duration - clip_duration);
            let end = (start + clip_duration).min(video_duration);
            if end - start >= 2.0 {
                selected_intervals.push((start, end, 0.8 - (i as f64 * 0.02)));
            }
        }
    }

    // Сортируем по времени начала
    selected_intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let suggested_fragments: Vec<crate::commands::FragmentDto> = selected_intervals
        .iter()
        .map(|(s, e, _)| crate::commands::FragmentDto { start: *s, end: *e })
        .collect();

    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        &format!("[Шаг 4/5] Отбор хайлайтов завершен: сформировано {} лучших моментов для таймлайна", suggested_fragments.len()),
    );
    for (i, frag) in suggested_fragments.iter().enumerate() {
        crate::logger::emit_log(
            &app_handle,
            "Pipeline",
            "info",
            &format!("  → Клип #{}: [{:.1}с - {:.1}с] (длительность {:.1}с)", i + 1, frag.start, frag.end, frag.end - frag.start),
        );
    }

    // Отправляем найденные хайлайты во фронтенд для предпросмотра
    let _ = app_handle.emit("suggested-fragments", SuggestedPayload {
        fragments: suggested_fragments.clone(),
    });

    if mode == "confirm" {
        let _ = tokio::fs::remove_file(&audio_output).await;
        crate::logger::emit_log(
            &app_handle,
            "Pipeline",
            "info",
            "Режим подтверждения: фрагменты отправлены на таймлайн для подтверждения и экспорта пользователем.",
        );
        let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
            percent: 100.0, 
            stage: "Хайлайты успешно найдены и готовы к предпросмотру".to_string() 
        });
        return Ok((Vec::new(), suggested_fragments));
    }

    // 6. Рендеринг клипов в автоматическом режиме
    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        &format!("[Шаг 5/5] Автоматический рендеринг {} клипов (вертикально 9:16 = {})...", selected_intervals.len(), config.vertical_format),
    );
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

    let base_out_dir = match config.output_dir.as_ref() {
        Some(dir_str) if !dir_str.trim().is_empty() => {
            let p = PathBuf::from(dir_str);
            if p.is_absolute() {
                p
            } else {
                let mut d = video_path.clone();
                d.pop();
                d.join(p)
            }
        }
        _ => {
            let mut d = video_path.clone();
            d.pop();
            d.join("output_cuts")
        }
    };
    let _ = tokio::fs::create_dir_all(&base_out_dir).await;
    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        &format!("Директория сохранения клипов: '{}'", base_out_dir.display()),
    );

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

        crate::logger::emit_log(
            &app_handle,
            "Pipeline",
            "info",
            &format!("Запуск нарезки клипа {}/{}: {:.1}с -> {:.1}с (длительность {:.1}с)", i + 1, total_clips, start_sec, end_sec, duration),
        );

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
    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        &format!("=== ПАЙПЛАЙН УСПЕШНО ЗАВЕРШЕН! Создано клипов: {} ===", generated_clips.len()),
    );
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 100.0, 
        stage: "Обработка завершена успешно!".to_string() 
    });

    Ok((generated_clips, suggested_fragments))
}

