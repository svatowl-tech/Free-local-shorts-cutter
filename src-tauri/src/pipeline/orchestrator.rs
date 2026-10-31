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

    // Определяем длительность и метаданных видео через FFmpeg
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

    let file_stem = video_path.file_stem().and_then(|s| s.to_str()).unwrap_or("extracted");

    // 1-2. Получение готовых субтитров из кэша проекта или первичная транскрибация через FFmpeg + Whisper
    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        "[Шаг 1-2/5] Подготовка аудио и субтитров (проверка кэша проекта / Whisper ASR)...",
    );
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 20.0, 
        stage: "Подготовка субтитров (Whisper ASR)...".to_string() 
    });

    let asr_model = config.asr_model_path.clone().map(PathBuf::from);
    let srt_path = crate::whisper::get_or_create_subtitles(
        video_path.clone(),
        asr_model,
        app_handle.clone(),
        cancel_rx.resubscribe(),
    ).await?;

    if mode == "manual" {
        crate::logger::emit_log(&app_handle, "Pipeline", "info", "Ручной режим: субтитры подготовлены и сохранены в папке проекта, готово к редактированию.");
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
                    &format!("Найдено {} чистых реплик в субтитрах. Запуск семантического анализа через движок '{}'...", frags.len(), config.text_engine),
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

    // 4. Визуальный и аудио-энергетический анализ ключевых сцен
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

    // 5. Умное построение списка лучших фрагментов (Хайлайтов) с динамической длительностью (10–20с) по границам полных фраз
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 85.0, 
        stage: "Формирование цельных диалогов и хайлайтов (10–20 сек)...".to_string() 
    });

    let target_dur = config.max_clip_duration_sec.min(video_duration).max(6.0);
    // Вместо жесткой привязки к 15.0 секундам задаем гибкий коридор длительности (например, от 10 до 20 секунд при цели 15с),
    // чтобы каждый ролик начинался ровно с начала первой фразы и заканчивался ровно после завершения последней фразы!
    let min_dur = (target_dur - 5.0).max(5.0).min(video_duration);
    let max_dur = (target_dur + 5.0).max(min_dur + 4.0).min(video_duration);

    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        &format!(
            "Адаптивный подбор границ клипов по цельным фразам: целевая {:.0}с (гибкий диапазон {:.0}с – {:.0}с, без обрыва фраз)",
            target_dur, min_dur, max_dur
        ),
    );

    // (start_sec, end_sec, score, first_phrase_preview, last_phrase_preview)
    let mut candidate_intervals: Vec<(f64, f64, f64, String, String)> = Vec::new();

    // Пропускаем первые секунды интро и финальные титры на длинных видео (> 3 мин)
    let intro_skip = if video_duration > 180.0 { 4.0 } else { 0.0 };
    let outro_limit = if video_duration > 300.0 { (video_duration - 25.0).max(min_dur) } else { video_duration };

    // А. Строим кандидатов по границам полных предложений вокруг реплик
    if !fragments.is_empty() {
        for anchor_idx in 0..fragments.len() {
            if crate::whisper::is_whisper_hallucination(&fragments[anchor_idx].text) {
                continue;
            }
            if let Some(cand) = build_complete_phrase_window(
                &fragments,
                &text_scores,
                &scenes,
                anchor_idx,
                min_dur,
                target_dur,
                max_dur,
                intro_skip,
                outro_limit,
                video_duration,
            ) {
                if cand.2 >= config.min_intensity_threshold * 0.88 {
                    candidate_intervals.push(cand);
                }
            }
        }
    }

    // Б. Из сцен визуального/аудио анализа:
    // Если в видео есть распознанные субтитры, привязываем визуальные пики к ближайшему цельному диалоговому окну (10–20с),
    // чтобы исключить нарезку клипов посреди безмолвного музыкального опенинга или на полуслове!
    for scene in &scenes {
        if scene.timestamp_sec < intro_skip || scene.timestamp_sec >= outro_limit {
            continue;
        }
        if !fragments.is_empty() {
            // Ищем ближайшую реплику рядом с визуальным пиком (в радиусе 6 секунд)
            if let Some((nearest_idx, _)) = fragments
                .iter()
                .enumerate()
                .filter(|(_, f)| {
                    !crate::whisper::is_whisper_hallucination(&f.text)
                        && (f.start - scene.timestamp_sec).abs() <= 6.0
                })
                .min_by(|(_, a), (_, b)| {
                    (a.start - scene.timestamp_sec)
                        .abs()
                        .partial_cmp(&(b.start - scene.timestamp_sec).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            {
                if let Some(mut cand) = build_complete_phrase_window(
                    &fragments,
                    &text_scores,
                    &scenes,
                    nearest_idx,
                    min_dur,
                    target_dur,
                    max_dur,
                    intro_skip,
                    outro_limit,
                    video_duration,
                ) {
                    cand.2 = (cand.2 + 0.03).min(0.97);
                    if cand.2 >= config.min_intensity_threshold * 0.88 {
                        candidate_intervals.push(cand);
                    }
                }
            }
        } else {
            // Только если в видео вообще нет речи (например чисто инструментальный клип)
            let start = (scene.timestamp_sec - 2.0).max(intro_skip).min((outro_limit - target_dur).max(0.0));
            let end = (start + target_dur).min(video_duration);
            if end - start >= 4.0 && scene.intensity >= config.min_intensity_threshold {
                candidate_intervals.push((
                    start,
                    end,
                    scene.intensity.clamp(0.50, 0.94),
                    "Визуальная динамика".to_string(),
                    scene.description.clone(),
                ));
            }
        }
    }

    // Если нет ни одного фрагмента выше порога — выбираем топ лучших цельных моментов по относительной ценности
    if candidate_intervals.is_empty() {
        crate::logger::emit_log(
            &app_handle,
            "Pipeline",
            "info",
            &format!("Ни один фрагмент не превысил порог {}. Автоматический выбор лучших цельных сцен по относительной ценности...", config.min_intensity_threshold),
        );
        if !fragments.is_empty() {
            let mut frag_with_scores: Vec<(usize, f64)> = fragments
                .iter()
                .enumerate()
                .filter(|(_, f)| !crate::whisper::is_whisper_hallucination(&f.text))
                .map(|(i, _)| (i, text_scores.get(i).copied().unwrap_or(0.5)))
                .collect();
            frag_with_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            for (idx, _) in frag_with_scores.into_iter().take(25) {
                if let Some(cand) = build_complete_phrase_window(
                    &fragments,
                    &text_scores,
                    &scenes,
                    idx,
                    min_dur,
                    target_dur,
                    max_dur,
                    intro_skip,
                    outro_limit,
                    video_duration,
                ) {
                    candidate_intervals.push(cand);
                }
            }
        } else if !scenes.is_empty() {
            for scn in scenes.iter().take(10) {
                let start = (scn.timestamp_sec - 1.0).max(0.0).min((video_duration - target_dur).max(0.0));
                let end = (start + target_dur).min(video_duration);
                if end - start >= 3.0 {
                    candidate_intervals.push((
                        start,
                        end,
                        scn.intensity.clamp(0.55, 0.92),
                        "Визуальная сцена".to_string(),
                        scn.description.clone(),
                    ));
                }
            }
        } else {
            let num_clips = if video_duration < 30.0 { 1 } else if video_duration < 90.0 { 2 } else { 3 };
            let step = video_duration / (num_clips as f64);
            for i in 0..num_clips {
                let start = (i as f64 * step).min(video_duration - 5.0).max(0.0);
                let end = (start + target_dur).min(video_duration);
                if end - start >= 3.0 {
                    candidate_intervals.push((
                        start,
                        end,
                        0.75 - (i as f64 * 0.05),
                        "Фрагмент".to_string(),
                        "Авто".to_string(),
                    ));
                }
            }
        }
    }

    // Устраняем перекрытия и скученность (Non-Maximum Suppression с минимальной дистанцией между клипами)
    let min_gap = (video_duration / 45.0).clamp(4.0, 25.0);
    candidate_intervals.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut selected_intervals: Vec<(f64, f64, f64, String, String)> = Vec::new();
    for candidate in candidate_intervals {
        let too_close_or_overlaps = selected_intervals.iter().any(|sel| {
            candidate.0 < (sel.1 + min_gap) && candidate.1 > (sel.0 - min_gap)
        });
        if !too_close_or_overlaps {
            selected_intervals.push(candidate);
        }
        if selected_intervals.len() >= 10 {
            break;
        }
    }

    // Сортируем по времени начала в хронологическом порядке
    selected_intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let suggested_fragments: Vec<crate::commands::FragmentDto> = selected_intervals
        .iter()
        .map(|(s, e, _, _, _)| crate::commands::FragmentDto { start: *s, end: *e })
        .collect();

    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        &format!("[Шаг 4/5] Отбор хайлайтов завершен: сформировано {} логически завершенных клипов (10–20с) для таймлайна", suggested_fragments.len()),
    );
    for (i, (start, end, score, first_txt, last_txt)) in selected_intervals.iter().enumerate() {
        crate::logger::emit_log(
            &app_handle,
            "Pipeline",
            "info",
            &format!(
                "  → Клип #{}: [{:.1}с - {:.1}с] (длительность {:.1}с, рейтинг {:.2}) | Начало: «{}» ... Конец: «{}»",
                i + 1,
                start,
                end,
                end - start,
                score,
                first_txt,
                last_txt
            ),
        );
    }

    // Отправляем найденные хайлайты во фронтенд для предпросмотра
    let _ = app_handle.emit("suggested-fragments", SuggestedPayload {
        fragments: suggested_fragments.clone(),
    });

    if mode == "confirm" {
        crate::logger::emit_log(
            &app_handle,
            "Pipeline",
            "info",
            "Режим подтверждения: фрагменты отправлены на таймлайн, субтитры сохранены в кэше проекта (повторный запуск Whisper при экспорте не потребуется).",
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

    let video_parent = video_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let project_dir = video_parent.join(format!("{}_Проект", file_stem));
    let clips_dir = project_dir.join("Готовые клипы");
    let _ = tokio::fs::create_dir_all(&clips_dir).await;

    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        &format!("Директория сохранения клипов проекта: '{}'", clips_dir.display()),
    );

    let total_clips = selected_intervals.len();
    for (i, (start_sec, end_sec, _, _, _)) in selected_intervals.iter().enumerate() {
        let duration = end_sec - start_sec;

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

/// Строит логически завершенное окно клипа в гибком диапазоне `[min_dur, max_dur]` (например, 10–20 секунд),
/// привязывая начало клипа строго к началу предложения, а конец — к полному завершению последней фразы без обрывов на полуслове.
#[allow(clippy::too_many_arguments)]
fn build_complete_phrase_window(
    fragments: &[crate::text_analyzer::SubtitleFragment],
    text_scores: &[f64],
    scenes: &[crate::analyzer::SceneCandidate],
    anchor_idx: usize,
    min_dur: f64,
    target_dur: f64,
    max_dur: f64,
    intro_skip: f64,
    outro_limit: f64,
    video_duration: f64,
) -> Option<(f64, f64, f64, String, String)> {
    if anchor_idx >= fragments.len() {
        return None;
    }

    // 1. Сдвигаем точку старта назад до истинного НАЧАЛА предложения (если якорь попал в середину фразы)
    let mut start_idx = anchor_idx;
    while start_idx > 0
        && !crate::text_analyzer::is_sentence_start(fragments, start_idx)
        && (anchor_idx - start_idx) < 5
    {
        let gap = (fragments[start_idx].start - fragments[start_idx - 1].end).max(0.0);
        if gap > 1.5 {
            break;
        }
        start_idx -= 1;
    }

    if fragments[start_idx].start < intro_skip || fragments[start_idx].start >= outro_limit {
        return None;
    }

    // Вспомогательное замыкание поиска лучшей конечной реплики `end_idx` от заданного `s_idx`
    let find_best_end = |s_idx: usize| -> Option<(usize, f64, f64, bool)> {
        let mut pre_roll = (fragments[s_idx].start - 0.18).max(intro_skip);
        if s_idx > 0 {
            // Не залезаем на хвост предыдущей реплики!
            pre_roll = pre_roll.max(fragments[s_idx - 1].end + 0.04).min(fragments[s_idx].start);
        }

        let mut best_choice: Option<(usize, f64, f64, bool, f64)> = None; // (end_idx, c_start, c_end, is_clean_end, boundary_quality)

        for end_idx in s_idx..fragments.len() {
            if crate::whisper::is_whisper_hallucination(&fragments[end_idx].text) {
                continue;
            }

            // Если между предыдущей и текущей репликой огромная музыкальная пауза (> 5с), а мы уже набрали min_dur — останавливаемся
            if end_idx > s_idx {
                let gap = (fragments[end_idx].start - fragments[end_idx - 1].end).max(0.0);
                let prev_dur = fragments[end_idx - 1].end - pre_roll;
                if gap > 5.0 && prev_dur >= min_dur {
                    break;
                }
            }

            let mut post_roll = (fragments[end_idx].end + 0.24).min(video_duration);
            if end_idx + 1 < fragments.len() {
                // Не захватываем первый слог следующей реплики!
                post_roll = post_roll
                    .min(fragments[end_idx + 1].start - 0.04)
                    .max(fragments[end_idx].end);
            }

            let dur = post_roll - pre_roll;
            // Допускаем небольшое расширение (+1.5с сверх max_dur), ТОЛЬКО если это нужно для завершения длинного предложения
            if dur > max_dur + 1.5 {
                break;
            }

            if dur >= min_dur {
                let is_sent_end = crate::text_analyzer::is_sentence_end(fragments, end_idx);
                let has_punct = crate::text_analyzer::has_terminal_punctuation(&fragments[end_idx].text);
                let pause_after = if end_idx + 1 < fragments.len() {
                    (fragments[end_idx + 1].start - fragments[end_idx].end).max(0.0)
                } else {
                    1.0
                };

                // Если длительность вышла за `max_dur` (в буфер +1.5с), принимаем её ТОЛЬКО если она закрывает предложение!
                if dur > max_dur && !is_sent_end {
                    continue;
                }

                // Оценка качества границы окончания:
                // Главный приоритет (+2.0) — полное логическое окончание предложения (is_sent_end)
                let mut b_quality = if is_sent_end { 2.0 } else { 0.2 };
                if has_punct {
                    b_quality += 0.45;
                }
                b_quality += (pause_after * 0.25).min(0.35);
                // Плавное предпочтение длительности ближе к золотой середине target_dur (15с), но в пределах 10..20с
                let dist_penalty = ((dur - target_dur).abs() / max_dur) * 0.35;
                b_quality -= dist_penalty;

                if best_choice.as_ref().map_or(true, |prev| b_quality > prev.4) {
                    best_choice = Some((end_idx, pre_roll, post_roll, is_sent_end, b_quality));
                }
            }
        }

        best_choice.map(|(e_idx, cs, ce, clean, _)| (e_idx, cs, ce, clean))
    };

    let mut chosen = find_best_end(start_idx);

    // Если от `start_idx` вперед диалог закончился раньше 10 секунд (или не нашлось ни одного полного конца фразы),
    // пробуем захватить предыдущее полное предложение перед `start_idx`, чтобы получился полноценный диалог 10–20с!
    if chosen.as_ref().map_or(true, |c| !c.3) && start_idx > 0 {
        let mut prev_start = start_idx - 1;
        while prev_start > 0
            && !crate::text_analyzer::is_sentence_start(fragments, prev_start)
            && (start_idx - prev_start) < 5
        {
            prev_start -= 1;
        }
        if fragments[prev_start].start >= intro_skip {
            if let Some(alt) = find_best_end(prev_start) {
                if alt.3 || chosen.is_none() {
                    start_idx = prev_start;
                    chosen = Some(alt);
                }
            }
        }
    }

    let (end_idx, clip_start, clip_end, is_clean_end) = chosen?;
    let clip_dur = clip_end - clip_start;
    if clip_dur < min_dur * 0.9 || clip_dur > max_dur + 1.6 {
        return None;
    }

    // 2. Оцениваем содержательность, плотность живой речи и драматургию внутри окна [start_idx..=end_idx]
    let mut peak_score: f64 = 0.0;
    let mut sum_score: f64 = 0.0;
    let mut cue_count = 0usize;
    let mut active_speech_sec = 0.0;

    for j in start_idx..=end_idx {
        let f = &fragments[j];
        if crate::whisper::is_whisper_hallucination(&f.text) {
            continue;
        }
        let sc = text_scores.get(j).copied().unwrap_or(0.42);
        if sc > peak_score {
            peak_score = sc;
        }
        sum_score += sc;
        cue_count += 1;
        active_speech_sec += (f.end - f.start).max(0.0);
    }

    // Защита от музыкальных заставок/опенингов: в 10–20 секундном ролике должно быть минимум 2 реплики и >= 3.2с живой речи
    if cue_count < 2 || active_speech_sec < 3.2 {
        return None;
    }

    let avg_score = sum_score / (cue_count as f64);
    let speech_density = (active_speech_sec / clip_dur).clamp(0.0, 1.0);

    // Бонус за идеальную целостность начала и конца фразы
    let is_clean_start = crate::text_analyzer::is_sentence_start(fragments, start_idx);
    let completeness_bonus = match (is_clean_start, is_clean_end) {
        (true, true) => 0.06,
        (true, false) | (false, true) => 0.02,
        (false, false) => -0.06,
    };

    // Бонус за совпадение с визуальной динамикой / аудио-пиком
    let max_scene_in_win = scenes
        .iter()
        .filter(|s| s.timestamp_sec >= clip_start && s.timestamp_sec <= clip_end)
        .map(|s| s.intensity)
        .fold(0.0_f64, f64::max);
    let scene_bonus = if max_scene_in_win > 0.0 {
        (max_scene_in_win * 0.05).clamp(0.01, 0.05)
    } else {
        0.0
    };

    let window_score = (peak_score * 0.48
        + avg_score * 0.30
        + speech_density.min(0.85) * 0.11
        + completeness_bonus
        + scene_bonus)
        .clamp(0.35, 0.97);

    let first_preview: String = fragments[start_idx].text.trim().chars().take(36).collect();
    let last_preview: String = fragments[end_idx].text.trim().chars().take(36).collect();

    Some((clip_start, clip_end, window_score, first_preview, last_preview))
}
