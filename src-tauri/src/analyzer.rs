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

    // Если модели нет или sidecar не установлен — запускаем двухфакторный алгоритм (FFmpeg Scene Detect + Audio Energy)
    if !has_vision_model || !has_vision_sidecar {
        crate::logger::emit_log(
            &app_handle,
            "Analyzer",
            "info",
            "Запуск двухфакторного алгоритма анализа хайлайтов (смена сцен FFmpeg + звуковая активность)...",
        );

        let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
            percent: 10.0,
            stage: "detecting_scene_cuts".to_string(),
            current_frame: 0,
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
            cancel_rx.resubscribe()
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
            cancel_rx.resubscribe()
        ).await.unwrap_or_default();

        let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
            percent: 90.0,
            stage: "ranking_highlights".to_string(),
            current_frame: 90,
            total_frames: 100,
        });

        let mut candidate_scenes = Vec::new();

        // А. Сопоставление смен сцен со звуковыми пиками
        for (idx, &scene_t) in scene_cuts.iter().enumerate() {
            if scene_t >= duration - 2.0 {
                continue;
            }
            // Проверяем, попадает ли смена сцены в звуковую активность (или рядом с ней в пределах 3 сек)
            let is_near_audio = audio_ranges.iter().any(|(a_start, a_end)| {
                scene_t >= (a_start - 2.0) && scene_t <= (a_end + 2.0)
            });

            let intensity = if is_near_audio { 0.88 } else { 0.65 };
            let actions = if is_near_audio {
                vec!["динамика".to_string(), "реплика/эмоция".to_string()]
            } else {
                vec!["смена плана".to_string(), "движение".to_string()]
            };

            candidate_scenes.push(SceneCandidate {
                timestamp_sec: scene_t,
                description: format!("Сцена #{} (динамичный момент {:.1}с)", idx + 1, scene_t),
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
                        intensity: 0.82,
                        actions: vec!["диалог".to_string(), "активность".to_string()],
                    });
                }
            }
        }

        // В. Сортируем по весу и устраняем скученность (Non-Maximum Suppression с окном 15с)
        candidate_scenes.sort_by(|a, b| b.intensity.partial_cmp(&a.intensity).unwrap_or(std::cmp::Ordering::Equal));

        let mut filtered_scenes = Vec::new();
        for sc in candidate_scenes {
            let too_close = filtered_scenes.iter().any(|f: &SceneCandidate| (f.timestamp_sec - sc.timestamp_sec).abs() < 15.0);
            if !too_close {
                filtered_scenes.push(sc);
            }
            if filtered_scenes.len() >= 12 {
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

        return Ok(filtered_scenes);
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
        total_frames: 0,
    });

    // 1. Адаптивное извлечение кадров через FFmpeg:
    // Извлекаем не более 1 кадра каждые 20 секунд (для длинных видео и стримов), макс ~20 кадров
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
            "-vf", "fps=1/20,scale=640:-1", // сжатое разрешение для скорости инференса нейросети
            "-vsync", "0",
            "-vframes", "24", // Ограничиваем макс 24 ключевых кадра
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
                log::warn!("Ошибка извлечения кадров: {}, используем fallback", e);
                return Ok(Vec::new());
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
        return Ok(Vec::new());
    }

    let mut candidates = Vec::new();
    let prompt = "Оцени эмоциональную напряженность сцены от 0 до 1 и определи действия: {\"description\": \"...\", \"intensity\": 0.7, \"actions\": [\"dialogue\"]}";

    // 3. Анализ кадров (максимум до 10 самых показательных)
    let sample_step = (total_frames / 10).max(1) as usize;
    let sampled_frames: Vec<_> = frames_with_ts.into_iter().step_by(sample_step).collect();
    let sampled_count = sampled_frames.len() as u32;

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
            Ok(cmd) => cmd.args([
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
                                return Err(AppError::Pipeline("llama error".to_string()));
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

            if let Some(out) = output_json {
                let json_start = out.find('{').unwrap_or(0);
                let json_end = out.rfind('}').unwrap_or(out.len().saturating_sub(1));
                if json_start <= json_end {
                    let clean_json = &out[json_start..=json_end];
                    if let Ok(parsed) = serde_json::from_str::<LlamaOutput>(clean_json) {
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

    let _ = app_handle.emit("analyzer-progress", AnalyzerProgressPayload {
        percent: 100.0,
        stage: "complete".to_string(),
        current_frame: sampled_count,
        total_frames: sampled_count,
    });

    // 4. Очистка временных файлов
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    Ok(candidates)
}

