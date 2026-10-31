// src-tauri/src/analyzer.rs
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandEvent;
use serde::{Deserialize, Serialize};
use crate::error::{AppResult, AppError};
use tokio::sync::broadcast::Receiver;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SceneCandidate {
    pub timestamp_sec: f64,
    pub description: String,
    pub intensity: f64,
    pub actions: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct AnalyzerProgressPayload {
    pub percent: f64,
    pub stage: String,
    pub current_frame: u32,
    pub total_frames: u32,
}

#[derive(Deserialize, Debug)]
struct LlamaOutput {
    #[serde(default)]
    description: String,
    #[serde(default)]
    intensity: f64,
    #[serde(default)]
    actions: Vec<String>,
}

/// Выполняет адаптивный визуальный анализ видео
pub async fn analyze_video(
    video_path: PathBuf,
    model_path: Option<PathBuf>,
    app_handle: AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<Vec<SceneCandidate>> {
    crate::logger::emit_log(
        &app_handle,
        "Analyzer",
        "info",
        &format!("Запуск анализа видео-сцен: '{}'", video_path.display()),
    );
    
    let shell = app_handle.shell();
    
    // Ищем модель llama-vision
    let model = match model_path {
        Some(p) => crate::models::resolve_model_path(&app_handle, &p.to_string_lossy()),
        None => None,
    }
    .or_else(|| crate::models::resolve_model_path(&app_handle, "qwen-vl-2b.gguf"))
    .or_else(|| crate::models::resolve_model_path(&app_handle, "qwen2-vl-2b-instruct-q4_k_m.gguf"));

    let has_vision_model = model.as_ref().map(|m| m.exists()).unwrap_or(false);
    let has_vision_sidecar = shell.sidecar("llama-vision").is_ok();

    // Если модели нет или sidecar не установлен — сразу запускаем двухфакторный алгоритм (FFmpeg Scene Detect + Audio Energy)
    if !has_vision_model || !has_vision_sidecar {
        return run_two_factor_scene_analysis(video_path, app_handle, cancel_rx).await;
    }

    let resolved_model = model.unwrap();

    // Создаем временную директорию для хранения кадров
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros();
    let pid = std::process::id();
    let temp_dir = std::env::temp_dir().join(format!("video_cutter_frames_{}_{}", pid, ts));
    tokio::fs::create_dir_all(&temp_dir).await.map_err(AppError::Io)?;
    
    let frames_pattern = temp_dir.join("frame_%03d.jpg");

    let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
        percent: 5.0,
        stage: "extracting_keyframes".to_string(),
        current_frame: 0,
        total_frames: 24,
    });

    // 1. Адаптивное извлечение кадров через FFmpeg:
    // Внимание: в FFmpeg 7.x/8.x (Lavf63+) устаревший флаг `-vsync 0` удалён и вызывает мгновенный выход с кодом 1.
    // Используем совместимый фильтр `fps=1/20,scale=640:-2` без `-vsync`.
    let ffmpeg_cmd = shell.sidecar("ffmpeg")
        .map_err(|e| {
            let _ = std::fs::remove_dir_all(&temp_dir);
            AppError::Pipeline(format!("Бинарник ffmpeg не найден в sidecar ресурсах: {}", e))
        })?
        .env("PATH", crate::logger::build_sidecar_env_path(&app_handle))
        .args([
            "-nostdin",
            "-y",
            "-i", &video_path.to_string_lossy(),
            "-vf", "fps=1/20,scale=640:-2",
            "-vframes", "24",
            &frames_pattern.to_string_lossy()
        ]);

    let (mut rx, child) = ffmpeg_cmd.spawn()
        .map_err(|e| {
            let _ = std::fs::remove_dir_all(&temp_dir);
            AppError::Pipeline(format!("Не удалось запустить FFmpeg для извлечения кадров: {}", e))
        })?;

    let extract_future = async {
        while let Some(event) = rx.recv().await {
            if let CommandEvent::Terminated(payload) = event {
                if payload.code == Some(0) {
                    return Ok(());
                } else {
                    return Err(AppError::Pipeline(format!("FFmpeg (извлечение кадров) завершился с кодом: {:?}", payload.code)));
                }
            }
        }
        Err(AppError::Pipeline("Процесс FFmpeg неожиданно прервался.".to_string()))
    };

    tokio::select! {
        res = extract_future => {
            if let Err(e) = res {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                crate::logger::emit_log(
                    &app_handle,
                    "Analyzer",
                    "warn",
                    &format!("Извлечение кадров для Vision-модели пропущено ({}). Переход на двухфакторный анализ сцен FFmpeg + Audio Energy...", e),
                );
                return run_two_factor_scene_analysis(video_path, app_handle, cancel_rx).await;
            }
        },
        _ = cancel_rx.recv() => {
            let _ = child.kill();
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(AppError::Pipeline("Анализ отменен пользователем.".to_string()));
        }
    }

    // 2. Сбор извлечённых кадров
    let mut frames_with_ts = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(&temp_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("jpg") {
                if let Some(file_name) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Some(num_str) = file_name.strip_prefix("frame_") {
                        if let Ok(num) = num_str.parse::<u32>() {
                            let timestamp_sec = (num as f64) * 20.0;
                            frames_with_ts.push((timestamp_sec, path, num));
                        }
                    }
                }
            }
        }
    }

    frames_with_ts.sort_by_key(|k| k.2);

    let total_frames = frames_with_ts.len() as u32;
    if total_frames == 0 {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        crate::logger::emit_log(
            &app_handle,
            "Analyzer",
            "info",
            "Ключевые кадры не сформированы. Переход на двухфакторный анализ сцен FFmpeg + Audio Energy...",
        );
        return run_two_factor_scene_analysis(video_path, app_handle, cancel_rx).await;
    }

    let mut candidates = Vec::new();
    let prompt = "Оцени эмоциональную напряженность сцены от 0 до 1 и определи действия: {\"description\": \"...\", \"intensity\": 0.7, \"actions\": [\"dialogue\"]}";

    // 3. Анализ кадров (максимум до 10 самых показательных)
    let sample_step = (total_frames / 10).max(1) as usize;
    let sampled_frames: Vec<_> = frames_with_ts.into_iter().step_by(sample_step).collect();
    let sampled_count = sampled_frames.len() as u32;
    let mut consecutive_vision_failures = 0usize;

    for (i, (timestamp_sec, frame_path, _)) in sampled_frames.into_iter().enumerate() {
        let current_frame = (i + 1) as u32;
        let percent = (current_frame as f64 / sampled_count as f64) * 100.0;

        let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
            percent,
            stage: "analyzing_keyframes".to_string(),
            current_frame,
            total_frames: sampled_count,
        });

        let llama_cmd = match shell.sidecar("llama-vision") {
            Ok(cmd) => cmd
                .env("PATH", crate::logger::build_sidecar_env_path(&app_handle))
                .args([
                    "-m", &resolved_model.to_string_lossy(),
                    "--image", &frame_path.to_string_lossy(),
                    "--prompt", prompt,
                    "--json",
                ]),
            Err(_) => break,
        };

        if let Ok((mut rx, child)) = llama_cmd.spawn() {
            let analyze_future = async {
                let mut stdout_accum = String::new();
                while let Some(event) = rx.recv().await {
                    match event {
                        CommandEvent::Stdout(bytes) => {
                            stdout_accum.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        CommandEvent::Terminated(payload) => {
                            if payload.code == Some(0) {
                                return Ok(stdout_accum);
                            } else {
                                return Err(AppError::Pipeline("llama-vision error".to_string()));
                            }
                        }
                        _ => {}
                    }
                }
                Err(AppError::Pipeline("error".to_string()))
            };

            let output_json = tokio::select! {
                res = analyze_future => res.ok(),
                _ = cancel_rx.recv() => {
                    let _ = child.kill();
                    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                    return Err(AppError::Pipeline("Анализ отменен пользователем.".to_string()));
                }
            };

            let mut frame_parsed = false;
            if let Some(out) = output_json {
                if let (Some(json_start), Some(json_end)) = (out.find('{'), out.rfind('}')) {
                    if json_start < json_end {
                        let clean_json = &out[json_start..=json_end];
                        if let Ok(parsed) = serde_json::from_str::<LlamaOutput>(clean_json) {
                            if parsed.intensity > 0.0 {
                                frame_parsed = true;
                                candidates.push(SceneCandidate {
                                    timestamp_sec,
                                    description: parsed.description,
                                    intensity: parsed.intensity,
                                    actions: parsed.actions,
                                });
                            }
                        }
                    }
                }
            }

            if !frame_parsed {
                consecutive_vision_failures += 1;
                // Если бинарник llama-vision не поддерживает формат модели (или нет mmproj) — не ждём 10 кадров, сразу переходим на FFmpeg Scene Detect
                if consecutive_vision_failures >= 2 && candidates.is_empty() {
                    break;
                }
            } else {
                consecutive_vision_failures = 0;
            }
        } else {
            break;
        }
    }

    // 4. Очистка временных файлов
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    // Если Vision-модель не вернула кандидатов — автоматически запускаем надежный двухфакторный алгоритм (FFmpeg Scene Detect + Audio Energy)
    if candidates.is_empty() {
        crate::logger::emit_log(
            &app_handle,
            "Analyzer",
            "info",
            "Переключение на высокоточный двухфакторный анализ сцен (FFmpeg Scene Detect + Audio Energy)...",
        );
        return run_two_factor_scene_analysis(video_path, app_handle, cancel_rx).await;
    }

    let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
        percent: 100.0,
        stage: "complete".to_string(),
        current_frame: sampled_count,
        total_frames: sampled_count,
    });

    Ok(candidates)
}

/// Двухфакторный алгоритм анализа видео-хайлайтов (смена планов FFmpeg + энергетические пики аудиодорожки)
async fn run_two_factor_scene_analysis(
    video_path: PathBuf,
    app_handle: AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<Vec<SceneCandidate>> {
    crate::logger::emit_log(
        &app_handle,
        "Analyzer",
        "info",
        "Запуск двухфакторного алгоритма анализа хайлайтов (смена сцен FFmpeg + звуковая активность)...",
    );

    let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
        percent: 10.0,
        stage: "detecting_scene_cuts".to_string(),
        current_frame: 10,
        total_frames: 100,
    });

    // 1. Получаем реальную длительность видео
    let duration = match crate::ffmpeg::probe_video(video_path.clone(), app_handle.clone()).await {
        Ok(meta) if meta.duration > 3.0 => meta.duration,
        _ => 180.0,
    };

    // 2. Детектируем точки смены сцен через FFmpeg
    let scene_cuts = crate::ffmpeg::detect_scene_changes(
        video_path.clone(),
        app_handle.clone(),
        cancel_rx.resubscribe(),
    ).await.unwrap_or_default();

    let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
        percent: 60.0,
        stage: "detecting_audio_energy".to_string(),
        current_frame: 60,
        total_frames: 100,
    });

    // 3. Детектируем участки с эмоциональной звуковой активностью (поиск речи/всплесков громкости)
    let audio_ranges = crate::ffmpeg::detect_audio_activity(
        video_path.clone(),
        duration,
        app_handle.clone(),
        cancel_rx.resubscribe(),
    ).await.unwrap_or_default();

    let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
        percent: 90.0,
        stage: "ranking_highlights".to_string(),
        current_frame: 90,
        total_frames: 100,
    });

    let mut candidate_scenes = Vec::new();

    // А. Многофакторная оценка динамики сцен (частота монтажных склеек + звуковая энергия + драматургическая позиция)
    for (idx, &scene_t) in scene_cuts.iter().enumerate() {
        if scene_t < 3.0 || scene_t >= duration - 3.0 {
            continue;
        }

        // 1. Плотность монтажных склеек в 15-секундном окне вокруг точки (индикатор экшена и динамики)
        let local_cuts = scene_cuts
            .iter()
            .filter(|&&t| (t - scene_t).abs() <= 7.5)
            .count();
        let cut_density_bonus = ((local_cuts as f64 - 1.0) / 10.0).clamp(0.0, 0.16);

        // 2. Сопоставление со звуковой активностью
        let matching_audio = audio_ranges.iter().find(|(a_start, a_end)| {
            scene_t >= (*a_start - 1.5) && scene_t <= (*a_end + 1.5)
        });

        let is_near_audio = matching_audio.is_some();
        let audio_bonus = if let Some((a_start, a_end)) = matching_audio {
            let seg_len = (a_end - a_start).max(1.0);
            let rel_pos = ((scene_t - a_start) / seg_len).clamp(0.0, 1.0);
            let focus = 1.0 - (rel_pos - 0.35).abs();
            0.12 + (focus * 0.06).clamp(0.0, 0.06)
        } else {
            0.0
        };

        // 3. Защита от перекоса в сторону заставки/опенинга (первые 95с и последние 90с в длинных видео > 10 мин)
        let pos_ratio = (scene_t / duration).clamp(0.0, 1.0);
        let narrative_factor = if duration > 600.0 && (scene_t < 95.0 || scene_t > duration - 90.0) {
            0.85
        } else if pos_ratio >= 0.35 && pos_ratio <= 0.88 {
            1.04
        } else {
            1.0
        };

        let raw_intensity = (0.60 + cut_density_bonus + audio_bonus) * narrative_factor;
        let intensity = raw_intensity.clamp(0.55, 0.94);

        let actions = if is_near_audio && local_cuts >= 4 {
            vec!["экшен/кульминация".to_string(), "эмоциональная речь".to_string()]
        } else if is_near_audio {
            vec!["динамика".to_string(), "реплика/эмоция".to_string()]
        } else {
            vec!["смена плана".to_string(), "движение".to_string()]
        };

        candidate_scenes.push(SceneCandidate {
            timestamp_sec: scene_t,
            description: format!("Сцена #{} (динамика {:.1}с, склеек: {})", idx + 1, scene_t, local_cuts),
            intensity,
            actions,
        });
    }

    // Б. Если смен сцен мало, добавляем центры самых громких участков звука
    if candidate_scenes.len() < 5 {
        for (idx, (a_start, a_end)) in audio_ranges.iter().enumerate() {
            let mid = ((a_start + a_end) / 2.0).min(duration - 2.0);
            if !candidate_scenes.iter().any(|c| (c.timestamp_sec - mid).abs() < 10.0) {
                candidate_scenes.push(SceneCandidate {
                    timestamp_sec: mid,
                    description: format!("Звуковой пик #{} (активная речь {:.1}с)", idx + 1, mid),
                    intensity: 0.78,
                    actions: vec!["диалог".to_string(), "активность".to_string()],
                });
            }
        }
    }

    // В. Сортируем по весу и устраняем скученность (адаптивный шаг для равномерного охвата всего видео)
    candidate_scenes.sort_by(|a, b| b.intensity.partial_cmp(&a.intensity).unwrap_or(std::cmp::Ordering::Equal));

    let min_scene_dist = (duration / 22.0).clamp(15.0, 55.0);
    let mut filtered_scenes = Vec::new();
    for sc in candidate_scenes {
        let too_close = filtered_scenes.iter().any(|f: &SceneCandidate| (f.timestamp_sec - sc.timestamp_sec).abs() < min_scene_dist);
        if !too_close {
            filtered_scenes.push(sc);
        }
        if filtered_scenes.len() >= 15 {
            break;
        }
    }

    // Г. Если всё ещё пусто (крайне тихое и статичное видео) - создаем 3 ключевые точки
    if filtered_scenes.is_empty() {
        let step = duration / 4.0;
        for i in 1..=3 {
            let t = (step * (i as f64)).min(duration - 2.0);
            filtered_scenes.push(SceneCandidate {
                timestamp_sec: t,
                description: format!("Ключевая сцена #{} ({:.0}с)", i, t),
                intensity: 0.70,
                actions: vec!["динамика".to_string()],
            });
        }
    }

    filtered_scenes.sort_by(|a, b| a.timestamp_sec.partial_cmp(&b.timestamp_sec).unwrap_or(std::cmp::Ordering::Equal));

    crate::logger::emit_log(
        &app_handle,
        "Analyzer",
        "info",
        &format!("✓ Двухфакторный анализ завершен: найдено {} качественных сцен-хайлайтов.", filtered_scenes.len()),
    );

    let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
        percent: 100.0,
        stage: "complete".to_string(),
        current_frame: filtered_scenes.len() as u32,
        total_frames: filtered_scenes.len() as u32,
    });

    Ok(filtered_scenes)
}

