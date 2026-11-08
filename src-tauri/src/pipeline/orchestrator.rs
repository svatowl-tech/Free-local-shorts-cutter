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

#[derive(Serialize, Deserialize)]
struct CachedTextScores {
    version: u32,
    engine: String,
    model_name: String,
    fragments_fingerprint: u64,
    scores: Vec<f64>,
}

#[derive(Serialize, Deserialize)]
struct CachedVisionScenes {
    version: u32,
    vision_model: String,
    scenes: Vec<crate::analyzer::SceneCandidate>,
}

fn compute_fragments_fingerprint(frags: &[crate::text_analyzer::SubtitleFragment]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    frags.len().hash(&mut hasher);
    for f in frags {
        f.text.hash(&mut hasher);
        ((f.start * 100.0).round() as i64).hash(&mut hasher);
        ((f.end * 100.0).round() as i64).hash(&mut hasher);
    }
    hasher.finish()
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
    let video_parent_dir = video_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let project_cache_dir = video_parent_dir.join(format!("{}_Проект", file_stem));
    let _ = tokio::fs::create_dir_all(&project_cache_dir).await;

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

                let text_scores_cache_path = project_cache_dir.join(format!("{}_text_scores.json", file_stem));
                let frags_fp = compute_fragments_fingerprint(&frags);
                let mut cached_scores_opt: Option<Vec<f64>> = None;

                if text_scores_cache_path.exists() {
                    if let Ok(raw_cache) = tokio::fs::read_to_string(&text_scores_cache_path).await {
                        if let Ok(parsed_cache) = serde_json::from_str::<CachedTextScores>(&raw_cache) {
                            if parsed_cache.version == 2
                                && parsed_cache.engine == config.text_engine
                                && parsed_cache.model_name == openai_model_name
                                && parsed_cache.fragments_fingerprint == frags_fp
                                && parsed_cache.scores.len() == frags.len()
                            {
                                crate::logger::emit_log(
                                    &app_handle,
                                    "LLM",
                                    "info",
                                    &format!(
                                        "✓ Найдены готовые оценки диалогов в кэше проекта ({} реплик, движок '{}'). Повторный запуск LLM не требуется!",
                                        frags.len(),
                                        config.text_engine
                                    ),
                                );
                                cached_scores_opt = Some(parsed_cache.scores);
                            }
                        }
                    }
                }

                let scores = if let Some(cached_s) = cached_scores_opt {
                    cached_s
                } else {
                    crate::logger::emit_log(
                        &app_handle,
                        "LLM",
                        "info",
                        &format!("Найдено {} чистых реплик в субтитрах. Запуск семантического анализа через движок '{}'...", frags.len(), config.text_engine),
                    );
                    let text_config = crate::text_analyzer::TextAnalysisConfig {
                        engine: match config.text_engine.as_str() {
                            "openai" => crate::text_analyzer::TextEngine::OpenAi,
                            "ollama" => crate::text_analyzer::TextEngine::Ollama,
                            "llama" => crate::text_analyzer::TextEngine::LlamaCli,
                            "builtin" => crate::text_analyzer::TextEngine::Builtin,
                            _ => crate::text_analyzer::TextEngine::Builtin,
                        },
                        model_name: openai_model_name.clone(),
                        ollama_url: config.ollama_url.clone(),
                        openai_base_url,
                        openai_api_key,
                    };

                    let computed = match crate::text_analyzer::analyze_text_with_llm(
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

                    if computed.len() == frags.len() {
                        let cache_payload = CachedTextScores {
                            version: 2,
                            engine: config.text_engine.clone(),
                            model_name: openai_model_name,
                            fragments_fingerprint: frags_fp,
                            scores: computed.clone(),
                        };
                        if let Ok(json_str) = serde_json::to_string(&cache_payload) {
                            let _ = tokio::fs::write(&text_scores_cache_path, json_str).await;
                        }
                    }
                    computed
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

    // 4. Визуальный и аудио-энергетический анализ ключевых сцен (с кэшированием в папке проекта)
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 75.0, 
        stage: "Визуальный анализ ключевых сцен...".to_string() 
    });

    let vision_model_str = config.vision_model_path.clone().unwrap_or_default();
    let vision_cache_path = project_cache_dir.join(format!("{}_vision_scenes.json", file_stem));
    let mut cached_scenes_opt: Option<Vec<crate::analyzer::SceneCandidate>> = None;

    if vision_cache_path.exists() {
        if let Ok(raw_vcache) = tokio::fs::read_to_string(&vision_cache_path).await {
            if let Ok(parsed_vcache) = serde_json::from_str::<CachedVisionScenes>(&raw_vcache) {
                if parsed_vcache.version == 2
                    && parsed_vcache.vision_model == vision_model_str
                    && !parsed_vcache.scenes.is_empty()
                {
                    crate::logger::emit_log(
                        &app_handle,
                        "Analyzer",
                        "info",
                        &format!(
                            "✓ Найдены готовые визуальные сцены Vision AI в кэше проекта ({} сцен). Повторный запуск FFmpeg и Vision AI не требуется!",
                            parsed_vcache.scenes.len()
                        ),
                    );
                    cached_scenes_opt = Some(parsed_vcache.scenes);
                }
            }
        }
    }

    let scenes = if let Some(cached_sc) = cached_scenes_opt {
        cached_sc
    } else {
        let vision_model = config.vision_model_path.clone().map(PathBuf::from);
        let analyzed_sc = crate::analyzer::analyze_video(
            video_path.clone(),
            vision_model,
            app_handle.clone(),
            cancel_rx.resubscribe()
        ).await.unwrap_or_default();

        if !analyzed_sc.is_empty() {
            let vcache_payload = CachedVisionScenes {
                version: 2,
                vision_model: vision_model_str,
                scenes: analyzed_sc.clone(),
            };
            if let Ok(json_str) = serde_json::to_string(&vcache_payload) {
                let _ = tokio::fs::write(&vision_cache_path, json_str).await;
            }
        }
        analyzed_sc
    };

    // 4.5. Высокоточный профиль звуковой энергии (шаг 20 мс, адаптивный порог тишины, пики экшена и зоны OP/ED)
    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload {
        percent: 81.0,
        stage: "Построение карты интенсивности звука и поиск тихих границ...".to_string()
    });
    let audio_profile_opt = crate::audio_analyzer::get_or_create_audio_profile(
        &video_path,
        &fragments,
        &app_handle,
        cancel_rx.resubscribe(),
    )
    .await
    .ok();

    // 5. Умное построение списка лучших фрагментов (Хайлайтов) с эластичной длительностью по границам завершённых сцен:
    // `config.max_clip_duration_sec` задает базовый ориентир (например, 15с), но чтобы шортсы НИКОГДА не обрезались
    // по смыслу на середине мысли, вопросе или перед панчлайном, разрешаем эластичный коридор (например, 11–22с при настройке 15с).
    let base_max = config.max_clip_duration_sec.clamp(10.0, 60.0).min(video_duration);
    let min_dur = (base_max * 0.68).clamp(10.0, 16.0).min((base_max - 2.0).max(4.0));
    let target_dur = (base_max * 1.05).clamp(min_dur + 2.5, 28.0).min(video_duration);
    let max_dur = (base_max * 1.45)
        .max(base_max + 6.5)
        .clamp(16.0, 60.0)
        .min(video_duration);

    let _ = app_handle.emit("pipeline-progress", PipelineProgressPayload { 
        percent: 85.0, 
        stage: format!("Формирование цельных диалогов и хайлайтов ({:.0}–{:.0} сек)...", min_dur, max_dur) 
    });
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
            "Адаптивный подбор границ клипов по цельным сценам: целевая {:.0}с (эластичный диапазон {:.0}с – {:.0}с для сохранения смысла, лимит: {})",
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
                if let Some(ref prof) = audio_profile_opt {
                    if prof.overlaps_op_ed(cand.0, cand.1) {
                        continue;
                    }
                }
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
                    if let Some(ref prof) = audio_profile_opt {
                        if prof.overlaps_op_ed(cand.0, cand.1) {
                            continue;
                        }
                    }
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

    // В. Включаем самые мощные звуковые экшн-сцены серии (взрывы, сражения, магия + частая смена планов, вне OP/ED),
    // чтобы взрывной экшен без диалогов (или с короткими выкриками) попадал в пул кандидатов!
    if let Some(ref prof) = audio_profile_opt {
        let scene_cut_times: Vec<f64> = scenes.iter().map(|s| s.timestamp_sec).collect();
        let action_windows = crate::audio_analyzer::extract_high_intensity_action_windows(
            prof,
            &scene_cut_times,
            &fragments,
            &scenes,
            min_dur,
            target_dur,
            max_dur,
            intro_skip,
            outro_limit,
        );
        if !action_windows.is_empty() {
            crate::logger::emit_log(
                &app_handle,
                "Audio",
                "info",
                &format!(
                    "✓ В пул кандидатов добавлено {} мощных экшн-сцен по пикам звуковой интенсивности и смене кадров (OP/ED отфильтрованы)",
                    action_windows.len()
                ),
            );
            all_valid_windows.extend(action_windows);
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

    // 2б. Адаптивное пополнение для длинных видео / стримов:
    // Если пользователь запросил подборку клипов (например 5–10), а строгий порог пропустил слишком мало (меньше половины от лимита или меньше 3),
    // мягко расширяем порог с шагом 0.05..0.12 (не опускаясь ниже безопасного порога содержательности 0.62),
    // чтобы пользователь не остался с 1 клипом на двухчасовом стриме.
    let min_desired = if is_auto_clips {
        effective_limit.min(3)
    } else {
        (effective_limit / 2).max(2).min(effective_limit)
    };

    if selected_intervals.len() < min_desired && !non_overlapping_scenes.is_empty() {
        let relaxed_floor = (user_threshold - 0.12).max(0.62);
        let needed = effective_limit.saturating_sub(selected_intervals.len());
        let additional: Vec<(f64, f64, f64, String, String)> = non_overlapping_scenes
            .iter()
            .filter(|cand| cand.2 < user_threshold && cand.2 >= relaxed_floor)
            .take(needed)
            .cloned()
            .collect();

        if !additional.is_empty() {
            crate::logger::emit_log(
                &app_handle,
                "Pipeline",
                "info",
                &format!(
                    "Адаптивная довыборка клипов (найдено {} из {} при пороге {:.2}): порог мягко скорректирован до {:.2} для получения полноценной подборки лучших моментов длинного видео (+{} сцен).",
                    selected_intervals.len(),
                    effective_limit,
                    user_threshold,
                    relaxed_floor,
                    additional.len()
                ),
            );
            selected_intervals.extend(additional);
        }
    }

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

    // 4. НОВЫЙ АУДИО-ЭТАП: Мягкая доводка границ выбранных клипов по тихим местам в окне до ±2.5 сек!
    // Гарантирует, что ролик не обрывается на начале или хвосте голосовой фразы, вышедшей за тайминг субтитров,
    // и начинается/заканчивается строго во впадине тишины без щелчков и обрезков слов.
    if let Some(ref prof) = audio_profile_opt {
        let stats = crate::audio_analyzer::refine_selected_intervals_with_audio(
            prof,
            &mut selected_intervals,
            &fragments,
            min_dur,
            max_dur,
        );
        crate::logger::emit_log(
            &app_handle,
            "Audio",
            "info",
            &format!(
                "✓ Аудио-выравнивание границ по тихим местам (окно до ±2.5с): скорректировано {}/{} клипов (макс. сдвиг {:.2}с, спасено от обрыва: {} начал и {} окончаний фраз)",
                stats.adjusted_clips,
                stats.total_clips,
                stats.max_shift_sec,
                stats.rescued_speech_starts,
                stats.rescued_speech_ends
            ),
        );
    }

    let suggested_fragments: Vec<crate::commands::FragmentDto> = selected_intervals
        .iter()
        .map(|(s, e, _, _, _)| crate::commands::FragmentDto { start: *s, end: *e })
        .collect();

    crate::logger::emit_log(
        &app_handle,
        "Pipeline",
        "info",
        &format!(
            "[Шаг 4/5] Отбор хайлайтов завершен: сформировано {} качественных клипов ({:.0}–{:.0}с, порог эмоций: {:.2}, {}) для таймлайна",
            suggested_fragments.len(),
            min_dur,
            max_dur,
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
    // (чтобы клип не начинался со слов «Поэтому...», «Из его кристалла...», «На большее я и не надеюсь...» или с ответа на вопрос)
    let mut start_idx = anchor_idx;
    while start_idx > 0
        && !crate::text_analyzer::is_sentence_start(fragments, start_idx)
        && (anchor_idx - start_idx) < 7
    {
        let gap = (fragments[start_idx].start - fragments[start_idx - 1].end).max(0.0);
        let max_back_gap = if !crate::text_analyzer::has_terminal_punctuation(&fragments[start_idx - 1].text)
            || crate::text_analyzer::starts_with_lowercase_continuation(&fragments[start_idx].text)
        {
            4.5
        } else if crate::text_analyzer::starts_with_discourse_continuation(&fragments[start_idx].text) {
            3.8
        } else {
            2.2
        };
        if gap > max_back_gap {
            break;
        }
        start_idx -= 1;
    }

    // 1б. Если `start_idx` пришелся на эмоциональный возглас в середине непрерывного монолога/обмена без пауз (gap <= 0.95с)
    // (например, «Поможем союзникам отступить!» сразу после «За нами наблюдают со всех сторон. Мы должны обеспечить полноценное сопровождение.»),
    // подтягиваем до 3 предыдущих предложений этого же непрерывного блока (до 7.5 сек подводки), чтобы клип имел полноценную завязку!
    if start_idx > 0 && !crate::text_analyzer::is_new_scene_opener(&fragments[start_idx].text) {
        let anchor_start_time = fragments[start_idx].start;
        for _step in 0..3 {
            if start_idx == 0 {
                break;
            }
            let gap_prev = (fragments[start_idx].start - fragments[start_idx - 1].end).max(0.0);
            if gap_prev > 0.95 {
                break;
            }
            let mut prev_sent = start_idx - 1;
            while prev_sent > 0
                && !crate::text_analyzer::is_sentence_start(fragments, prev_sent)
                && (start_idx - prev_sent) < 4
            {
                let g = (fragments[prev_sent].start - fragments[prev_sent - 1].end).max(0.0);
                if g > 2.5 {
                    break;
                }
                prev_sent -= 1;
            }
            if fragments[prev_sent].start < intro_skip
                || (anchor_start_time - fragments[prev_sent].start) > 7.5
                || crate::text_analyzer::is_weak_clip_start(fragments, prev_sent)
                || crate::whisper::is_whisper_hallucination(&fragments[prev_sent].text)
                || crate::text_analyzer::is_opening_or_song_fragment(fragments, prev_sent)
            {
                break;
            }
            start_idx = prev_sent;
            if crate::text_analyzer::is_new_scene_opener(&fragments[start_idx].text) {
                break;
            }
        }
    }

    // Если начальная реплика является диалоговым ответом / реакцией («Хорошо, полагаюсь на тебя!», «Да, конечно», «Вы уверены?»),
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
                } else if crate::text_analyzer::starts_with_discourse_continuation(&fragments[prompt_start].text) {
                    3.8
                } else {
                    2.0
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

    // Если `start_idx` оказался на коротком пустом подтверждении («Да.», «Ничего страшного.») или на последней брошенной фразе
    // уходящей сцены прямо перед открытием новой сцены («Я тебе крылышки-то пообломаю!» -> «Мы приближаемся к зоне боевых действий!»),
    // сдвигаем старт вперед на содержательное начало следующей реплики
    while start_idx + 1 < fragments.len()
        && crate::text_analyzer::is_weak_clip_start(fragments, start_idx)
        && (crate::text_analyzer::is_sentence_start(fragments, start_idx + 1)
            || crate::text_analyzer::is_new_scene_opener(&fragments[start_idx + 1].text))
        && (fragments[start_idx + 1].start - fragments[start_idx].end).max(0.0) <= 2.5
    {
        start_idx += 1;
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
        let mut exclamations_seen = 0usize;

        for end_idx in s_idx..fragments.len() {
            if crate::whisper::is_whisper_hallucination(&fragments[end_idx].text) {
                continue;
            }

            if end_idx > s_idx {
                let gap = (fragments[end_idx].start - fragments[end_idx - 1].end).max(0.0);
                let prev_dur = fragments[end_idx - 1].end - pre_roll;
                let prev_complete = crate::text_analyzer::is_sentence_end(fragments, end_idx - 1)
                    && !crate::text_analyzer::is_weak_clip_ending(fragments, end_idx - 1);

                // 1) Никогда не приклеиваем к концу текущей сцены явное открытие НОВОЙ сцены («Мы приближаемся к зоне...», «Саламандра — транспортировщику!»)!
                if crate::text_analyzer::is_new_scene_opener(&fragments[end_idx].text)
                    && prev_dur >= min_dur * 0.72
                {
                    break;
                }

                // 2) Если предыдущее предложение полностью завершено, хронометраж уже достаточен (>= 85% min_dur),
                // и наступила межсценовая пауза >= 2.1с — закрываем сцену, чтобы не захватывать начало следующего эпизода!
                if prev_complete && prev_dur >= min_dur * 0.85 && gap >= 2.1 {
                    break;
                }

                let max_bridge_gap = if prev_dur < min_dur + 1.5 { 4.2 } else { 2.8 };
                if gap > max_bridge_gap && prev_dur >= min_dur * 0.82 {
                    break;
                }
                if gap >= 2.0 {
                    crossed_scene_transition = true;
                }
            }

            let curr_txt = fragments[end_idx].text.trim();
            let curr_is_question = curr_txt.ends_with('?')
                || curr_txt.ends_with("?!")
                || curr_txt.ends_with("!?");
            let curr_is_pure_exclam = curr_txt.contains('!') && !curr_is_question;
            if curr_is_pure_exclam || curr_txt.contains("?!") || curr_txt.contains("!?") {
                exclamations_seen += 1;
            }

            let mut post_roll = (fragments[end_idx].end + 0.24).min(video_duration);
            if end_idx + 1 < fragments.len() {
                // Не захватываем первый слог следующей реплики!
                post_roll = post_roll
                    .min(fragments[end_idx + 1].start - 0.04)
                    .max(fragments[end_idx].end);
            }

            let dur = post_roll - pre_roll;
            // Эластичный потолок (+3.5с..+5.5с сверх max_dur для эмоциональных восклицаний и реакций стримера)
            // для гарантированного закрытия начатой мысли, ответа на вопрос или панчлайна
            let has_high_energy = curr_is_pure_exclam
                || exclamations_seen > 0
                || (end_idx + 1 < fragments.len() && (fragments[end_idx + 1].text.contains('!') || fragments[end_idx + 1].text.contains("?!")));
            let ceiling_extra = if has_high_energy { 5.5 } else { 3.5 };
            let max_ceiling = (max_dur + ceiling_extra).min(video_duration);
            if dur > max_ceiling {
                break;
            }

            if dur >= min_dur * 0.88 {
                let is_sent_end = crate::text_analyzer::is_sentence_end(fragments, end_idx);
                let is_weak_end = crate::text_analyzer::is_weak_clip_ending(fragments, end_idx);
                let has_punct = crate::text_analyzer::has_terminal_punctuation(curr_txt);
                let pause_after = if end_idx + 1 < fragments.len() {
                    (fragments[end_idx + 1].start - fragments[end_idx].end).max(0.0)
                } else {
                    1.5
                };

                // Если длительность вышла за `max_dur`, принимаем её ТОЛЬКО если она закрывает предложение и не является слабым концом!
                if dur > max_dur && (!is_sent_end || is_weak_end) {
                    continue;
                }

                // Оценка качества границы окончания:
                // 1. Главный приоритет (+2.6) — полное логическое окончание предложения и мысли (is_sent_end)
                let mut b_quality = if is_sent_end { 2.6 } else { -0.4 };
                if has_punct {
                    b_quality += 0.45;
                }

                // 2. Жесткий штраф за обрыв на слабой реплике, вопросе без ответа или зачине («Знакомая атака.», «Это там... Рейнский дьявол?», «Что вообще происходит?»),
                // чтобы алгоритм всегда доводил клип до ответа и настоящего панчлайна!
                if is_weak_end {
                    b_quality -= 1.85;
                }

                // 3. Оценка паузы после финальной реплики:
                // - Если `pause_after >= 0.60с` (естественная пауза после завершенной мысли) — солидный бонус!
                // - Если `pause_after <= 0.20с` (персонаж продолжает говорить следующую фразу без вдоха, `gap == 0.00с`)
                //   и следующая фраза НЕ является новой сценой и помещается в `max_ceiling` — сильный штраф за разрыв на полуслове!
                let next_fits_ceiling = end_idx + 1 < fragments.len()
                    && (fragments[end_idx + 1].end - pre_roll) <= max_ceiling
                    && !crate::text_analyzer::is_new_scene_opener(&fragments[end_idx + 1].text);
                let is_zero_pause_continuation = pause_after <= 0.20 && next_fits_ceiling;

                if pause_after >= 0.60 {
                    b_quality += (pause_after * 0.48).min(0.92);
                } else if pause_after >= 0.35 {
                    b_quality += 0.25;
                } else if is_zero_pause_continuation {
                    b_quality -= 2.00;
                } else if pause_after < 0.25 && next_fits_ceiling {
                    b_quality -= 1.25;
                }

                // 3б. Если сразу после `end_idx` (пауза <= 0.85с) следует смысловое продолжение / вывод мысли
                // («Надеюсь, доведется ещё рассказать...», «Опять она!», «Ну, что? Понял, что со мной шутки плохи, Существо Икс?!») —
                // штрафуем раннюю остановку, чтобы клип захватил финальную точку!
                if end_idx + 1 < fragments.len()
                    && pause_after <= 0.85
                    && next_fits_ceiling
                    && (crate::text_analyzer::starts_with_discourse_continuation(&fragments[end_idx + 1].text)
                        || crate::text_analyzer::starts_with_dialogue_response(&fragments[end_idx + 1].text))
                {
                    b_quality -= 1.10;
                }

                // 3в. Корректная обработка вопросов (`?`) и восклицательных панчлайнов (`!`):
                // НИКОГДА не поощряем остановку на вопросе, если следом идет ответ собеседника (pause_after <= 8.5с)!
                let has_following_answer = curr_is_question
                    && end_idx + 1 < fragments.len()
                    && pause_after <= 8.5;

                let is_answer_to_prev_q = end_idx > s_idx
                    && {
                        let prev_t = fragments[end_idx - 1].text.trim();
                        prev_t.ends_with('?') || prev_t.ends_with("?!") || prev_t.ends_with("!?")
                    }
                    && (fragments[end_idx].start - fragments[end_idx - 1].end).max(0.0) <= 8.5;

                if has_following_answer {
                    b_quality -= 2.50;
                } else if is_sent_end && !is_weak_end && curr_is_pure_exclam && pause_after >= 0.35 {
                    // Настоящий восклицательный панчлайн с паузой после него!
                    b_quality += 0.62;
                } else if is_answer_to_prev_q && is_sent_end && !is_weak_end {
                    // Поощряем закрытие клипа на полном ответе после вопроса собеседника!
                    b_quality += 0.72;
                } else if !curr_is_pure_exclam && !curr_is_question && exclamations_seen >= 2 && dur >= min_dur + 2.5 && pause_after < 0.4 {
                    b_quality -= 0.35;
                }

                // 3г. Если фраза заканчивается на слова благодарности («спасибо, [Имя]», «благодарю»),
                // а следом (pause_after <= 0.45с) идет ответ собеседника («рада, что смогла помочь», «не за что», «всегда пожалуйста»),
                // и он укладывается в max_ceiling — не обрываем клип на «спасибо», захватываем ответ!
                let curr_low = curr_txt.to_lowercase().replace('ё', "е");
                let curr_is_thanks = curr_low.ends_with("спасибо")
                    || curr_low.ends_with("спасибо.")
                    || curr_low.ends_with("спасибо!")
                    || curr_low.contains("огромное спасибо")
                    || curr_low.ends_with("благодарю")
                    || curr_low.ends_with("благодарю.");
                let next_is_thanks_reply = end_idx + 1 < fragments.len()
                    && {
                        let nt = fragments[end_idx + 1].text.to_lowercase().replace('ё', "е");
                        nt.contains("рада")
                            || nt.contains("рад,")
                            || nt.contains("рад ")
                            || nt.contains("не за что")
                            || nt.contains("пожалуйста")
                            || nt.contains("помочь вам")
                            || nt.contains("смогла помочь")
                    };
                if curr_is_thanks && next_is_thanks_reply && pause_after <= 0.45 && next_fits_ceiling {
                    b_quality -= 1.85;
                }

                // 4. Штраф за склейку двух разных сцен через длинную паузу >= 2.0с
                if crossed_scene_transition {
                    b_quality -= 0.85;
                }

                // 5. Бонус за яркий финальный панчлайн
                let end_cue_score = text_scores.get(end_idx).copied().unwrap_or(0.45);
                if end_cue_score >= 0.68 && !is_weak_end {
                    b_quality += 0.30;
                }

                // 6. Мягкое центрирование вокруг целевой длительности target_dur (не перебивающее смысловую целостность!)
                let sweet_min = min_dur.min(target_dur);
                let sweet_max = (max_dur - 0.5).max(target_dur);
                if dur >= sweet_min && dur <= sweet_max {
                    b_quality += 0.32;
                }
                let dist_penalty = ((dur - target_dur).abs() / max_dur) * 0.42;
                b_quality -= dist_penalty;

                // Чистое окончание предложения (без слабого обрыва, без продолжения без вдоха втугую и без открытого вопроса)
                let curr_clean = is_sent_end && !is_weak_end && !is_zero_pause_continuation && !has_following_answer;
                let beats_prev = match &best_choice {
                    None => true,
                    Some(prev) => {
                        if curr_clean && !prev.3 {
                            true
                        } else if !curr_clean && prev.3 {
                            false
                        } else {
                            b_quality > prev.5
                        }
                    }
                };

                if beats_prev {
                    best_choice = Some((
                        end_idx,
                        pre_roll,
                        post_roll,
                        curr_clean,
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
    // пришлось перешагнуть паузу смены сцен >= 2.0с `crossed_scene_transition == true`, либо диалог закончился на слабой фразе),
    // шагаем назад по репликам ЭТОЙ ЖЕ сцены (пока паузы между ними <= 2.0с и сдвиг <= 13.0с), чтобы клип захватил
    // подводку и завершился ровно на кульминации без склейки со следующей сценой!
    if chosen.as_ref().map_or(true, |c| !c.3 || c.4) && start_idx > 0 {
        let mut cursor = start_idx;
        for _step in 0..5 {
            if cursor == 0 {
                break;
            }
            let gap_before = (fragments[cursor].start - fragments[cursor - 1].end).max(0.0);
            if gap_before > 2.0 {
                break;
            }
            let mut prev_start = cursor - 1;
            while prev_start > 0
                && !crate::text_analyzer::is_sentence_start(fragments, prev_start)
                && (cursor - prev_start) < 6
            {
                let g = (fragments[prev_start].start - fragments[prev_start - 1].end).max(0.0);
                let max_prev_gap = if !crate::text_analyzer::has_terminal_punctuation(&fragments[prev_start - 1].text)
                    || crate::text_analyzer::starts_with_lowercase_continuation(&fragments[prev_start].text)
                {
                    4.5
                } else if crate::text_analyzer::starts_with_discourse_continuation(&fragments[prev_start].text) {
                    3.8
                } else {
                    2.0
                };
                if g > max_prev_gap {
                    break;
                }
                prev_start -= 1;
            }
            if fragments[prev_start].start < intro_skip
                || (fragments[start_idx].start - fragments[prev_start].start) > 13.0
            {
                break;
            }
            if crate::text_analyzer::is_weak_clip_start(fragments, prev_start) {
                cursor = prev_start;
                continue;
            }
            if let Some(alt) = find_best_end(prev_start) {
                let is_better = match &chosen {
                    None => true,
                    Some(curr) => {
                        (alt.3 && !alt.4 && (!curr.3 || curr.4))
                            || (alt.3 == curr.3 && alt.4 == curr.4 && alt.5 > curr.5 + 0.12)
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
    if clip_dur < min_dur * 0.85 || clip_dur > max_dur + 3.8 {
        return None;
    }

    // 2. Оцениваем содержательность, плотность живой речи и реальный эмоциональный накал внутри окна [start_idx..=end_idx]
    let mut cue_scores: Vec<f64> = Vec::with_capacity(end_idx - start_idx + 1);
    let mut cue_count = 0usize;
    let mut song_cue_count = 0usize;
    let mut exclam_cues = 0usize;
    let mut question_cues = 0usize;
    let mut intense_punct_cues = 0usize;
    let mut high_llm_cues = 0usize;
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
        if sc >= 0.72 {
            high_llm_cues += 1;
        }

        if f.text.contains('!') {
            exclam_cues += 1;
        }
        if f.text.contains('?') {
            question_cues += 1;
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

    // Бонус/штраф эмоционального накала внутри окна (учитывающий как восклицания в экшене, так и вопросы/драматический обмен в детективах/комедиях):
    let emotion_burst_modifier = if exclam_cues >= 4
        || (exclam_cues >= 3 && peak_score >= 0.82)
        || (exclam_cues >= 2 && high_llm_cues >= 3)
    {
        0.11
    } else if exclam_cues >= 3
        || (exclam_cues >= 2 && (intense_punct_cues >= 1 || question_cues >= 1 || high_llm_cues >= 2))
    {
        0.085
    } else if exclam_cues == 2
        || intense_punct_cues >= 1
        || (exclam_cues == 1 && (question_cues >= 1 || high_llm_cues >= 2))
    {
        0.055
    } else if exclam_cues == 1
        || question_cues >= 2
        || high_llm_cues >= 2
        || (question_cues == 1 && high_llm_cues >= 1)
    {
        0.025
    } else if question_cues == 1 || high_llm_cues == 1 || speech_density >= 0.58 {
        0.0
    } else {
        -0.05
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

    // Широкий и сбалансированный динамический диапазон оценки окна (0.28 .. 0.97):
    // - Спокойная экспозиция без вопросов и пиков: 0.45 – 0.68
    // - Содержательные диалоговые сцены (детектив / комедия / вопросы / 1 восклицание): 0.65 – 0.82
    // - Сильные эмоциональные сцены (2 восклицания / драма / расследование): 0.80 – 0.88
    // - Супер-интенсивные кульминации (3+ восклицаний / бой / шок, для порога 0.85–0.90+): 0.87 – 0.96
    let mut raw_window_score = peak_score * 0.56
        + top2_avg * 0.26
        + avg_score * 0.10
        + speech_density.min(0.85) * 0.04
        + completeness_bonus
        + emotion_burst_modifier
        + scene_bonus
        - transition_penalty;

    if exclam_cues == 0
        && intense_punct_cues == 0
        && question_cues == 0
        && high_llm_cues == 0
        && max_scene_in_win < 0.80
    {
        raw_window_score = raw_window_score.min(0.68);
    } else if exclam_cues == 0 && intense_punct_cues == 0 && max_scene_in_win < 0.82 {
        raw_window_score = raw_window_score.min(0.79);
    } else if exclam_cues == 1 && intense_punct_cues == 0 && peak_score < 0.85 && max_scene_in_win < 0.85 {
        raw_window_score = raw_window_score.min(0.84);
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
