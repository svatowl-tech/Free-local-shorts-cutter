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
    pub openai_base_url: Option<String>,
    pub openai_api_key: Option<String>,
    pub openai_model: Option<String>,
    pub openai_profile_id: Option<String>,
    pub output_dir: Option<String>,
    pub min_intensity_threshold: f64,
    pub max_clip_duration_sec: f64,
    pub max_clips_count: Option<usize>,
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
                let effective_model_name = if config.text_engine == "openai" {
                    config
                        .openai_model
                        .clone()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| {
                            if config.text_model_name.ends_with(".gguf") || config.text_model_name.trim().is_empty() {
                                "gpt-4o-mini".to_string()
                            } else {
                                config.text_model_name.clone()
                            }
                        })
                } else {
                    config.text_model_name.clone()
                };

                let mut openai_base_url = config
                    .openai_base_url
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
                let mut openai_api_key = config.openai_api_key.clone().unwrap_or_default();
                let mut openai_model_name = effective_model_name;

                // Защитный слой: если выбран OpenAI API, но ключ пустой или остался дефолтный адрес api.openai.com,
                // автоматически подгружаем сохраненный пользователем профиль (например PlusVibe) из openai_profiles.json
                if config.text_engine == "openai" && (openai_api_key.trim().is_empty() || openai_base_url.contains("api.openai.com")) {
                    let profiles_path = crate::commands::resolve_openai_profiles_path(&app_handle);
                    if profiles_path.exists() {
                        if let Ok(content) = std::fs::read_to_string(&profiles_path) {
                            if let Ok(profiles) = serde_json::from_str::<Vec<crate::commands::OpenAiProfile>>(&content) {
                                let target_prof = if let Some(ref prof_id) = config.openai_profile_id {
                                    profiles.iter().find(|p| p.id == *prof_id && !p.api_key.trim().is_empty())
                                } else {
                                    None
                                }.or_else(|| profiles.iter().find(|p| !p.api_key.trim().is_empty()));

                                if let Some(prof) = target_prof {
                                    crate::logger::emit_log(
                                        &app_handle,
                                        "LLM",
                                        "info",
                                        &format!(
                                            "Автоматически подключен сохраненный профиль OpenAI API '{}' (сервер: {}, модель: {}) из хранилища настроек",
                                            prof.name, prof.base_url, prof.model
                                        ),
                                    );
                                    openai_base_url = prof.base_url.clone();
                                    openai_api_key = prof.api_key.clone();
                                    if !prof.model.trim().is_empty() {
                                        openai_model_name = prof.model.clone();
                                    }
                                }
                            }
                        }
                    }
                }

                let text_config = crate::text_analyzer::TextAnalysisConfig {
                    engine: match config.text_engine.as_str() {
                        "openai" => crate::text_analyzer::TextEngine::OpenAi,
                        "ollama" => crate::text_analyzer::TextEngine::Ollama,
                        "llama" => crate::text_analyzer::TextEngine::LlamaCli,
                        "builtin" => crate::text_analyzer::TextEngine::Builtin,
                        _ => crate::text_analyzer::TextEngine::Builtin,
                    },
                    model_name: openai_model_name,
                    ollama_url: config.ollama_url.clone(),
                    openai_base_url,
                    openai_api_key,
                };

                let scores = match crate::text_analyzer::analyze_text_with_llm(
                    &frags, 
                    &text_config, 
                    &app_handle, 
                    cancel_rx.resubscribe()
                ).await {
                    Ok(s) => s,
                    Err(e) => {
                        if e.to_string().contains("Отменено") {
                            return Err(e);
                        }
                        crate::logger::emit_log(
                            &app_handle,
                            "LLM",
                            "warn",
                            &format!("Предохранитель текстового анализа: {}. Каскадный переход на llama-cli...", e),
                        );
                        match crate::text_analyzer::analyze_with_llama_cli(
                            &frags,
                            None,
                            &app_handle,
                            cancel_rx.resubscribe(),
                        ).await {
                            Ok(llama_s) => {
                                crate::logger::emit_log(
                                    &app_handle,
                                    "LLM",
                                    "info",
                                    &format!("✓ Резервный llama-cli успешно оценил {} реплик", frags.len()),
                                );
                                llama_s
                            }
                            Err(e2) => {
                                if e2.to_string().contains("Отменено") {
                                    return Err(e2);
                                }
                                crate::logger::emit_log(
                                    &app_handle,
                                    "LLM",
                                    "warn",
                                    &format!("Сбой llama-cli: {}. Финальный переход на высокоскоростной встроенный NLP расчет.", e2),
                                );
                                crate::text_analyzer::compute_text_scores_advanced(&frags)
                            }
                        }
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
    let (is_auto_clips, max_clips_limit) = match config.max_clips_count {
        Some(0) => (true, 60usize),
        Some(n) => (false, n.clamp(1, 100)),
        None => (false, 10usize),
    };

    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        &format!(
            "Адаптивный подбор границ клипов по цельным фразам: целевая {:.0}с (гибкий диапазон {:.0}с – {:.0}с, лимит: {})",
            target_dur, min_dur, max_dur,
            if is_auto_clips { "Авто (все качественные сцены)".to_string() } else { format!("до {} клипов", max_clips_limit) }
        ),
    );

    // (start_sec, end_sec, score, first_phrase_preview, last_phrase_preview)
    let mut all_valid_windows: Vec<(f64, f64, f64, String, String)> = Vec::new();

    // Пропускаем первые секунды интро и финальные титры на длинных видео (> 3 мин)
    let intro_skip = if video_duration > 180.0 { 4.0 } else { 0.0 };
    let outro_limit = if video_duration > 300.0 { (video_duration - 25.0).max(min_dur) } else { video_duration };

    // А. Строим все кандидаты цельных диалоговых окон по границам предложений
    if !fragments.is_empty() {
        for anchor_idx in 0..fragments.len() {
            if crate::whisper::is_whisper_hallucination(&fragments[anchor_idx].text)
                || crate::text_analyzer::is_opening_or_song_fragment(&fragments, anchor_idx)
            {
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
                all_valid_windows.push(cand);
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
            if let Some((nearest_idx, _)) = fragments
                .iter()
                .enumerate()
                .filter(|(idx, f)| {
                    !crate::whisper::is_whisper_hallucination(&f.text)
                        && !crate::text_analyzer::is_opening_or_song_fragment(&fragments, *idx)
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
                    // Усиливаем окно только если сама визуальная сцена имеет высокий накал (>= 0.75)
                    if scene.intensity >= 0.75 {
                        cand.2 = (cand.2 + (scene.intensity - 0.72) * 0.14).min(0.96);
                    }
                    all_valid_windows.push(cand);
                }
            }
        } else {
            // Только если в видео вообще нет речи (чисто инструментальный клип)
            let start = (scene.timestamp_sec - 2.0).max(intro_skip).min((outro_limit - target_dur).max(0.0));
            let end = (start + target_dur).min(video_duration);
            if end - start >= 4.0 {
                all_valid_windows.push((
                    start,
                    end,
                    scene.intensity.clamp(0.45, 0.94),
                    "Визуальная динамика".to_string(),
                    scene.description.clone(),
                ));
            }
        }
    }

    // Если строгий построитель диалоговых окон не дал кандидатов (например, редкие одиночные реплики с огромными паузами),
    // строим резервные окна вокруг лучших реплик
    if all_valid_windows.is_empty() && !fragments.is_empty() {
        let mut frag_with_scores: Vec<(usize, f64)> = fragments
            .iter()
            .enumerate()
            .filter(|(_, f)| !crate::whisper::is_whisper_hallucination(&f.text))
            .map(|(i, _)| (i, text_scores.get(i).copied().unwrap_or(0.45)))
            .collect();
        frag_with_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (idx, sc) in frag_with_scores.into_iter().take(max_clips_limit * 2) {
            let f = &fragments[idx];
            if f.start < intro_skip || f.start >= outro_limit {
                continue;
            }
            let start = (f.start - 0.30).max(0.0);
            let mut end = (start + target_dur).min(video_duration);
            let mut last_txt = f.text.clone();
            for next_f in &fragments[idx..] {
                if next_f.end <= start + max_dur && next_f.end >= start + min_dur * 0.8 {
                    end = (next_f.end + 0.24).min(video_duration);
                    last_txt = next_f.text.clone();
                } else if next_f.start > start + max_dur {
                    break;
                }
            }
            if end - start >= min_dur * 0.75 {
                all_valid_windows.push((
                    start,
                    end,
                    sc.clamp(0.42, 0.92),
                    f.text.chars().take(36).collect(),
                    last_txt.chars().take(36).collect(),
                ));
            }
        }
    }

    if all_valid_windows.is_empty() && !scenes.is_empty() {
        for scn in scenes.iter().take(max_clips_limit * 2) {
            let start = (scn.timestamp_sec - 1.0).max(intro_skip).min((video_duration - target_dur).max(0.0));
            let end = (start + target_dur).min(video_duration);
            if end - start >= 3.0 {
                all_valid_windows.push((
                    start,
                    end,
                    scn.intensity.clamp(0.45, 0.92),
                    "Визуальная сцена".to_string(),
                    scn.description.clone(),
                ));
            }
        }
    }

    if all_valid_windows.is_empty() {
        let num_clips = if video_duration < 30.0 { 1 } else if video_duration < 90.0 { 2 } else { 3 };
        let step = video_duration / (num_clips as f64);
        for i in 0..num_clips {
            let start = (i as f64 * step).min(video_duration - 5.0).max(0.0);
            let end = (start + target_dur).min(video_duration);
            if end - start >= 3.0 {
                all_valid_windows.push((
                    start,
                    end,
                    0.70 - (i as f64 * 0.05),
                    "Фрагмент".to_string(),
                    "Авто".to_string(),
                ));
            }
        }
    }

    // 1. Сначала выполняем Non-Maximum Suppression (NMS) по всем валидным окнам серии,
    // чтобы получить чистый список независимых сцен без перекрытий, отсортированный от самой мощной кульминации к спокойным сценам
    let min_gap = (video_duration / 45.0).clamp(4.0, 25.0);
    all_valid_windows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut non_overlapping_scenes: Vec<(f64, f64, f64, String, String)> = Vec::new();
    for cand in all_valid_windows {
        // В последних 85 секундах 20+-минутной серии (титры/эндинг) требуем повышенную достоверность >= 0.74
        if video_duration >= 1200.0 && cand.0 >= video_duration - 85.0 && cand.2 < 0.74 {
            continue;
        }
        let overlaps = non_overlapping_scenes.iter().any(|sel| {
            cand.0 < (sel.1 + min_gap) && cand.1 > (sel.0 - min_gap)
        });
        if !overlaps {
            non_overlapping_scenes.push(cand);
        }
        if non_overlapping_scenes.len() >= 60 {
            break;
        }
    }

    let user_threshold = config.min_intensity_threshold.clamp(0.10, 0.98);
    let max_scene_score = non_overlapping_scenes.first().map(|s| s.2).unwrap_or(0.0);

    // Динамический потолок количества роликов в режиме «Авто» в зависимости от запрошенного уровня эмоций (`min_intensity_threshold`):
    // - при 0.95+: максимум 2 самых взрывных клипа на серию
    // - при 0.90: максимум 4 супер-интенсивных клипа
    // - при 0.85: максимум 6 клипов
    // - при 0.80: максимум 8 клипов
    // - при 0.75: максимум 11 клипов
    // - при 0.70: максимум 14 клипов
    // - при <= 0.60: до 20 клипов
    let auto_intensity_cap: usize = if user_threshold >= 0.94 {
        2
    } else if user_threshold >= 0.89 {
        4
    } else if user_threshold >= 0.84 {
        6
    } else if user_threshold >= 0.79 {
        8
    } else if user_threshold >= 0.74 {
        11
    } else if user_threshold >= 0.68 {
        14
    } else {
        20
    };

    let effective_limit = if is_auto_clips {
        auto_intensity_cap
    } else {
        max_clips_limit
    };

    // 2. Строгий отбор клипов, реально преодолевших заданный пользователем порог интенсивности эмоций
    let mut selected_intervals: Vec<(f64, f64, f64, String, String)> = non_overlapping_scenes
        .iter()
        .filter(|cand| cand.2 >= user_threshold)
        .take(effective_limit)
        .cloned()
        .collect();

    // 3. Если пользователь выставил экстремальный порог (например 0.90–0.95), а в данной серии пиковая сцена имеет 0.86–0.89:
    // НИ В КОЕМ СЛУЧАЕ не сбрасываем фильтр до 20 проходных сцен по 0.66!
    // Выбираем ТОЛЬКО узкую верхушку абсолютных пиков серии (не более 2–3 сцен в пределах 0.05 от максимума серии).
    if selected_intervals.is_empty() && !non_overlapping_scenes.is_empty() {
        let strict_fallback_cap = if user_threshold >= 0.90 {
            3usize.min(effective_limit)
        } else if user_threshold >= 0.82 {
            4usize.min(effective_limit)
        } else if user_threshold >= 0.72 {
            6usize.min(effective_limit)
        } else {
            8usize.min(effective_limit)
        };
        let min_peak_floor = (max_scene_score - 0.05).max(0.68);

        selected_intervals = non_overlapping_scenes
            .into_iter()
            .filter(|cand| cand.2 >= min_peak_floor)
            .take(strict_fallback_cap)
            .collect();

        crate::logger::emit_log(
            &app_handle,
            "Pipeline",
            "info",
            &format!(
                "Строгий фильтр интенсивности (порог {:.2}, макс. накал в серии {:.2}): отобрано только {} самых пиковых кульминационных сцен (>= {:.2}).",
                user_threshold,
                max_scene_score,
                selected_intervals.len(),
                min_peak_floor
            ),
        );
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
        &format!(
            "[Шаг 4/5] Отбор хайлайтов завершен: сформировано {} качественных клипов (10–20с, порог эмоций: {:.2}, {}) для таймлайна",
            suggested_fragments.len(),
            user_threshold,
            if is_auto_clips { format!("режим: Авто (лимит по интенсивности до {})", effective_limit) } else { format!("лимит: до {}", max_clips_limit) }
        ),
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
/// привязывая начало клипа строго к началу предложения, а конец — к полному завершению диалогового обмена без обрывов на полуслове или слабой реплике.
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

    // 1. Сдвигаем точку старта назад до истинного НАЧАЛА предложения / мысли
    // (чтобы клип не начинался со слов «Поэтому...», «И в моих руках...» или с середины незаконченной фразы)
    let mut start_idx = anchor_idx;
    while start_idx > 0
        && !crate::text_analyzer::is_sentence_start(fragments, start_idx)
        && (anchor_idx - start_idx) < 6
    {
        let gap = (fragments[start_idx].start - fragments[start_idx - 1].end).max(0.0);
        let max_back_gap = if !crate::text_analyzer::has_terminal_punctuation(&fragments[start_idx - 1].text)
            || crate::text_analyzer::starts_with_lowercase_continuation(&fragments[start_idx].text)
        {
            4.5
        } else {
            1.8
        };
        if gap > max_back_gap {
            break;
        }
        start_idx -= 1;
    }

    // Если начальная реплика является диалоговым ответом / реакцией («Хорошо, полагаюсь на тебя!», «Да, конечно», «Ладно!»),
    // и собеседник произнес вводную реплику/вопрос совсем недавно (gap <= 2.8с) — включаем реплику собеседника, чтобы не терять контекст!
    if start_idx > 0 && crate::text_analyzer::starts_with_dialogue_response(&fragments[start_idx].text) {
        let gap_prev = (fragments[start_idx].start - fragments[start_idx - 1].end).max(0.0);
        if gap_prev <= 2.8 {
            let mut prompt_start = start_idx - 1;
            while prompt_start > 0
                && !crate::text_analyzer::is_sentence_start(fragments, prompt_start)
                && (start_idx - prompt_start) < 4
            {
                let g = (fragments[prompt_start].start - fragments[prompt_start - 1].end).max(0.0);
                let max_prompt_gap = if !crate::text_analyzer::has_terminal_punctuation(&fragments[prompt_start - 1].text)
                    || crate::text_analyzer::starts_with_lowercase_continuation(&fragments[prompt_start].text)
                {
                    4.5
                } else {
                    1.8
                };
                if g > max_prompt_gap {
                    break;
                }
                prompt_start -= 1;
            }
            if fragments[prompt_start].start >= intro_skip {
                start_idx = prompt_start;
            }
        }
    }

    if fragments[start_idx].start < intro_skip || fragments[start_idx].start >= outro_limit {
        return None;
    }

    // Вспомогательное замыкание поиска лучшей конечной реплики `end_idx` от заданного `s_idx`
    let find_best_end = |s_idx: usize| -> Option<(usize, f64, f64, bool, bool, f64)> {
        let mut pre_roll = (fragments[s_idx].start - 0.18).max(intro_skip);
        if s_idx > 0 {
            // Не залезаем на хвост предыдущей реплики!
            pre_roll = pre_roll.max(fragments[s_idx - 1].end + 0.04).min(fragments[s_idx].start);
        }

        let mut best_choice: Option<(usize, f64, f64, bool, bool, f64)> = None; // (end_idx, c_start, c_end, is_clean_end, crossed_scene_transition, boundary_quality)
        let mut crossed_scene_transition = false;

        for end_idx in s_idx..fragments.len() {
            if crate::whisper::is_whisper_hallucination(&fragments[end_idx].text) {
                continue;
            }

            if end_idx > s_idx {
                let gap = (fragments[end_idx].start - fragments[end_idx - 1].end).max(0.0);
                let prev_dur = fragments[end_idx - 1].end - pre_roll;
                // Если мы еще на самом минимуме длительности (< 12.2с), допускаем драматичную паузу до 4.3с
                // (чтобы не обрезать кульминацию после эмоционального возгласа, как в «Немедленно закрой свой рот... Какой бы ни была цена...»).
                // Когда уже набрано >= 12.2с, закрываем сцену при паузе > 3.2с.
                let max_bridge_gap = if prev_dur < 12.2 { 4.3 } else { 3.2 };
                if gap > max_bridge_gap && prev_dur >= min_dur * 0.9 {
                    break;
                }
                if gap >= 2.2 {
                    crossed_scene_transition = true;
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
            // Допускаем аккуратное адаптивное расширение до +1.2с сверх max_dur для завершения красивого панчлайна/мысли
            let max_ceiling = max_dur + 1.2;
            if dur > max_ceiling {
                break;
            }

            if dur >= min_dur {
                let is_sent_end = crate::text_analyzer::is_sentence_end(fragments, end_idx);
                let is_weak_end = crate::text_analyzer::is_weak_clip_ending(fragments, end_idx);
                let has_punct = crate::text_analyzer::has_terminal_punctuation(&fragments[end_idx].text);
                let pause_after = if end_idx + 1 < fragments.len() {
                    (fragments[end_idx + 1].start - fragments[end_idx].end).max(0.0)
                } else {
                    1.0
                };

                // Если длительность вышла за `max_dur`, принимаем её ТОЛЬКО если она закрывает предложение и не является слабым концом!
                if dur > max_dur && (!is_sent_end || is_weak_end) {
                    continue;
                }

                // Оценка качества границы окончания:
                // 1. Главный приоритет (+2.4) — полное логическое окончание предложения (is_sent_end)
                let mut b_quality = if is_sent_end { 2.4 } else { 0.1 };
                if has_punct {
                    b_quality += 0.45;
                }

                // 2. Сильный штраф за обрыв на слабой реплике («- Да уж.», «- Ясно.», «Какая мерзость!», «Значит, ты и есть та самая дочка...»),
                // чтобы алгоритм продолжал окно еще на 2–3 секунды и захватывал финальный панчлайн собеседника!
                if is_weak_end {
                    b_quality -= 1.45;
                }

                // 3. Бонус за естественную паузу после финальной реплики (признак завершения микро-диалога),
                // и небольшой штраф, если pause_after == 0.0с (собеседник мгновенно отвечает в ту же миллисекунду)
                if pause_after >= 0.45 {
                    b_quality += (pause_after * 0.35).min(0.55);
                } else if pause_after < 0.12 && dur < max_dur - 2.5 {
                    b_quality -= 0.22;
                }

                // 3б. Если сразу после `end_idx` (пауза <= 0.40с) следует завершающий вывод мысли («И, патологически, нужна вера.», «Поэтому...») —
                // штрафуем раннюю остановку, чтобы клип захватил финальную точку мысли, если позволяет длительность
                if end_idx + 1 < fragments.len()
                    && pause_after <= 0.40
                    && crate::text_analyzer::starts_with_discourse_continuation(&fragments[end_idx + 1].text)
                {
                    b_quality -= 0.55;
                }

                // 4. Штраф за склейку двух разных сцен через длинную паузу >= 2.2с
                if crossed_scene_transition {
                    b_quality -= 0.65;
                }

                // 5. Небольшой бонус за насыщенность диалога (чтобы полноценная сцена с ярким панчлайном выигрывала у сухого обрывка)
                let end_cue_score = text_scores.get(end_idx).copied().unwrap_or(0.45);
                if end_cue_score >= 0.68 && !is_weak_end {
                    b_quality += 0.25;
                }

                // 6. Бонус «золотой середины» (12.5с..=17.8с вокруг 15с), чтобы алгоритм не обрывал клип
                // на первой же короткой фразе в 10.0–10.6с, если следом органично помещается следующее предложение!
                if dur >= 12.5 && dur <= 17.8 {
                    b_quality += 0.42;
                }

                // Четкое центрирование вокруг целевой длительности target_dur (15с: типично 13с–17с, максимум ~20с)
                let dist_penalty = ((dur - target_dur).abs() / max_dur) * 0.70;
                b_quality -= dist_penalty;

                if best_choice.as_ref().map_or(true, |prev| b_quality > prev.5) {
                    best_choice = Some((
                        end_idx,
                        pre_roll,
                        post_roll,
                        is_sent_end && !is_weak_end,
                        crossed_scene_transition,
                        b_quality,
                    ));
                }
            }
        }

        best_choice
    };

    let mut chosen = find_best_end(start_idx);

    // Если `start_idx` пришелся на кульминационную фразу в самом конце сцены (так что при движении вперед
    // пришлось перешагнуть паузу смены сцен >= 2.2с `crossed_scene_transition == true`, либо диалог закончился на слабой фразе),
    // шагаем назад по репликам ЭТОЙ ЖЕ сцены (пока паузы между ними <= 1.8с и сдвиг <= 11.5с), чтобы клип захватил
    // подводку и завершился ровно на кульминации без склейки со следующей сценой!
    if chosen.as_ref().map_or(true, |c| !c.3 || c.4) && start_idx > 0 {
        let mut cursor = start_idx;
        for _step in 0..4 {
            if cursor == 0 {
                break;
            }
            let gap_before = (fragments[cursor].start - fragments[cursor - 1].end).max(0.0);
            if gap_before > 1.8 {
                break;
            }
            let mut prev_start = cursor - 1;
            while prev_start > 0
                && !crate::text_analyzer::is_sentence_start(fragments, prev_start)
                && (cursor - prev_start) < 5
            {
                let g = (fragments[prev_start].start - fragments[prev_start - 1].end).max(0.0);
                let max_prev_gap = if !crate::text_analyzer::has_terminal_punctuation(&fragments[prev_start - 1].text)
                    || crate::text_analyzer::starts_with_lowercase_continuation(&fragments[prev_start].text)
                {
                    4.5
                } else {
                    1.8
                };
                if g > max_prev_gap {
                    break;
                }
                prev_start -= 1;
            }
            if fragments[prev_start].start < intro_skip
                || (fragments[start_idx].start - fragments[prev_start].start) > 11.5
            {
                break;
            }
            if let Some(alt) = find_best_end(prev_start) {
                let is_better = match &chosen {
                    None => true,
                    Some(curr) => {
                        (alt.3 && !alt.4 && (!curr.3 || curr.4))
                            || (alt.3 == curr.3 && alt.4 == curr.4 && alt.5 > curr.5 + 0.15)
                    }
                };
                if is_better {
                    start_idx = prev_start;
                    chosen = Some(alt);
                    if alt.3 && !alt.4 {
                        break;
                    }
                }
            }
            cursor = prev_start;
        }
    }

    let (end_idx, clip_start, clip_end, is_clean_end, _, _) = chosen?;
    let clip_dur = clip_end - clip_start;
    if clip_dur < min_dur * 0.9 || clip_dur > max_dur + 1.5 {
        return None;
    }

    // 2. Оцениваем содержательность, плотность живой речи и реальный эмоциональный накал внутри окна [start_idx..=end_idx]
    let mut cue_scores: Vec<f64> = Vec::with_capacity(end_idx - start_idx + 1);
    let mut cue_count = 0usize;
    let mut song_cue_count = 0usize;
    let mut exclam_cues = 0usize;
    let mut intense_punct_cues = 0usize;
    let mut active_speech_sec = 0.0;
    let mut max_internal_gap = 0.0_f64;

    for j in start_idx..=end_idx {
        let f = &fragments[j];
        if j > start_idx {
            let g = (f.start - fragments[j - 1].end).max(0.0);
            if g > max_internal_gap {
                max_internal_gap = g;
            }
        }
        if crate::whisper::is_whisper_hallucination(&f.text) {
            continue;
        }
        if crate::text_analyzer::is_opening_or_song_fragment(fragments, j) {
            song_cue_count += 1;
            continue;
        }
        let sc = text_scores.get(j).copied().unwrap_or(0.40);
        cue_scores.push(sc);
        cue_count += 1;

        if f.text.contains('!') {
            exclam_cues += 1;
        }
        if f.text.contains("?!") || f.text.contains("!?") || f.text.contains("!!") {
            intense_punct_cues += 1;
        }

        // Ограничиваем вклад одной реплики в active_speech_sec разумным пределом (до 5.5с),
        // на случай если Whisper растянул таймкод короткой фразы на 9 секунд тишины/ходьбы
        let words = f.text.split_whitespace().count();
        let realistic_max_dur = (words as f64 * 0.85).max(1.8);
        let effective_cue_dur = (f.end - f.start).max(0.0).min(realistic_max_dur);
        active_speech_sec += effective_cue_dur;
    }

    // Защита от музыкальных заставок/опенингов: отсекаем окна с песнями и требуем >= 2 реплик и >= 3.2с живой речи
    if song_cue_count >= 2 || cue_count < 2 || active_speech_sec < 3.2 {
        return None;
    }

    cue_scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let peak_score = cue_scores[0];
    let second_score = cue_scores.get(1).copied().unwrap_or(peak_score);
    let top2_avg = (peak_score + second_score) * 0.5;
    let avg_score: f64 = cue_scores.iter().sum::<f64>() / (cue_count as f64);
    let speech_density = (active_speech_sec / clip_dur).clamp(0.0, 1.0);

    // Бонус за идеальную целостность начала и конца диалога
    let is_clean_start = crate::text_analyzer::is_sentence_start(fragments, start_idx)
        && !crate::text_analyzer::starts_with_discourse_continuation(&fragments[start_idx].text);
    let completeness_bonus = match (is_clean_start, is_clean_end) {
        (true, true) => 0.03,
        (true, false) | (false, true) => 0.0,
        (false, false) => -0.08,
    };

    // Штраф за длинный провал тишины/музыки посередине клипа (когда склеены две разные сцены)
    let transition_penalty = if max_internal_gap >= 2.5 {
        0.09
    } else if max_internal_gap >= 2.0 {
        0.05
    } else {
        0.0
    };

    // Бонус/штраф эмоционального накала внутри окна:
    // - Спокойный разговор без единого восклицательного знака получает ощутимый штраф `-0.10`, чтобы не попадать в выборки высокого уровня эмоций (0.85+)!
    // - Настоящая взрывная кульминация (несколько восклицаний подряд, `?!`/`!!`, высокий пик) получает бонус `+0.085..+0.11`, позволяя пробить 0.90–0.96!
    let emotion_burst_modifier = if exclam_cues == 0 && intense_punct_cues == 0 {
        -0.10
    } else if exclam_cues >= 4 || (exclam_cues >= 3 && peak_score >= 0.85) {
        0.11
    } else if exclam_cues >= 3 || (exclam_cues >= 2 && intense_punct_cues >= 1) {
        0.085
    } else if exclam_cues == 2 || intense_punct_cues >= 1 {
        0.045
    } else {
        0.0
    };

    // Бонус за совпадение с сильной визуальной динамикой / аудио-пиком
    let max_scene_in_win = scenes
        .iter()
        .filter(|s| s.timestamp_sec >= clip_start && s.timestamp_sec <= clip_end)
        .map(|s| s.intensity)
        .fold(0.0_f64, f64::max);

    let scene_bonus = if max_scene_in_win >= 0.75 {
        ((max_scene_in_win - 0.70) * 0.18).clamp(0.01, 0.05)
    } else {
        0.0
    };

    // Широкий динамический диапазон оценки окна (0.28 .. 0.97):
    // - Спокойные разговорные сцены (0 восклицаний): 0.38 – 0.68
    // - Одиночные реплики с 1 восклицанием: 0.68 – 0.81
    // - Сильные эмоциональные сцены (2 восклицания / драма): 0.82 – 0.88
    // - Супер-интенсивные кульминации (3+ восклицаний / бой, для порога 0.90+): 0.89 – 0.96
    let mut raw_window_score = peak_score * 0.56
        + top2_avg * 0.26
        + avg_score * 0.10
        + speech_density.min(0.85) * 0.04
        + completeness_bonus
        + emotion_burst_modifier
        + scene_bonus
        - transition_penalty;

    // Окно без единого восклицательного знака / двойной пунктуации и без явного визуального пика (>= 0.82)
    // жестко ограничено сверху порогом 0.68, а с одиночным обычным восклицанием — порогом 0.81,
    // чтобы при пороге 0.85–0.90 отбирались только по-настоящему бурные эмоциональные сцены!
    if exclam_cues == 0 && intense_punct_cues == 0 && max_scene_in_win < 0.82 {
        raw_window_score = raw_window_score.min(0.68);
    } else if exclam_cues == 1 && intense_punct_cues == 0 && peak_score < 0.85 && max_scene_in_win < 0.85 {
        raw_window_score = raw_window_score.min(0.81);
    }

    let window_score = raw_window_score.clamp(0.28, 0.97);

    let format_preview_text = |txt: &str, _is_last: bool| -> String {
        let t = txt
            .trim()
            .replace(['\r', '\n'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let chars: Vec<char> = t.chars().collect();
        if chars.len() <= 68 {
            t
        } else {
            let prefix: String = chars[..64].iter().collect();
            if let Some(last_sp) = prefix.rfind(' ') {
                format!("{}...", prefix[..last_sp].trim_end())
            } else {
                format!("{}...", prefix.trim_end())
            }
        }
    };

    let first_preview = format_preview_text(&fragments[start_idx].text, false);
    let last_preview = format_preview_text(&fragments[end_idx].text, true);

    Some((clip_start, clip_end, window_score, first_preview, last_preview))
}
