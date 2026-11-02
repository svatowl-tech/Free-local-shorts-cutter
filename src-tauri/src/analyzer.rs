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

/// Выполняет комплексный мультимодальный анализ видео:
/// 1) Двухфакторный поиск динамичных сцен по всей длительности видео (FFmpeg Scene Detect + Audio Energy).
/// 2) Если установлена совместимая Vision-модель (Moondream 2 / LLaVA Phi-3 Mini + mmproj) —
///    извлекает точные кадры в точках лучших сцен-кандидатов и проводит нейросетевой визуальный анализ каждого кадра через `llama-vision`.
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

    // 1. Сначала выполняем полноразмерный двухфакторный поиск сцен-хайлайтов по всей длине видео (00:00 .. конец серии)
    let mut base_scenes = run_two_factor_scene_analysis(
        video_path.clone(),
        app_handle.clone(),
        cancel_rx.resubscribe(),
    )
    .await?;

    if base_scenes.is_empty() {
        return Ok(base_scenes);
    }

    let shell = app_handle.shell();
    let vision_pair = crate::models::resolve_vision_model_and_projector(
        &app_handle,
        model_path.as_deref().and_then(|p| p.to_str()),
    );
    let has_vision_sidecar = shell.sidecar("llama-vision").is_ok()
        || shell.sidecar("llama-cli").is_ok();

    if vision_pair.is_none() || !has_vision_sidecar {
        if vision_pair.is_none() {
            let pref_str = model_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if pref_str.contains("qwen2-vl") || pref_str.contains("qwen-vl") {
                crate::logger::emit_log(
                    &app_handle,
                    "Analyzer",
                    "warn",
                    "Модель Qwen2-VL использует архитектуру 'qwen2vl', несовместимую с движком llama-vision (llava-cli). Скачайте в Менеджере ИИ-Моделей совместимую Vision-модель 'Moondream 2 Vision' или 'LLaVA Phi-3 Mini'.",
                );
            }
        }
        return Ok(base_scenes);
    }

    let (resolved_model, resolved_mmproj) = vision_pair.unwrap();
    crate::logger::emit_log(
        &app_handle,
        "Analyzer",
        "info",
        &format!(
            "Обнаружена совместимая Vision-модель: '{}' с проектором '{}'. Запуск нейросетевого визуального анализа топ-сцен...",
            resolved_model.file_name().unwrap_or_default().to_string_lossy(),
            resolved_mmproj.file_name().unwrap_or_default().to_string_lossy()
        ),
    );

    // Полный нейросетевой анализ всех найденных сцен-кандидатов через Vision AI (Moondream2 / LLaVA)
    // без искусственных ограничений (если найдено 15 сцен — анализируются все 15)
    let mut top_indices: Vec<usize> = (0..base_scenes.len()).collect();
    top_indices.sort_unstable();

    crate::logger::emit_log(
        &app_handle,
        "Analyzer",
        "info",
        &format!(
            "Полный нейросетевой Vision AI анализ всех {} найденных сцен-кандидатов (без искусственных ограничений)...",
            base_scenes.len()
        ),
    );

    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros();
    let pid = std::process::id();
    let temp_dir = std::env::temp_dir().join(format!("video_cutter_frames_{}_{}", pid, ts));
    let _ = tokio::fs::create_dir_all(&temp_dir).await;

    let sidecar_name = if shell.sidecar("llama-vision").is_ok() {
        "llama-vision"
    } else {
        "llama-cli"
    };

    // Нативный лаконичный VQA-промпт для Moondream 2 и LLaVA (нейтральное описание визуального содержимого кадра)
    let vqa_prompt = "\n\nQuestion: Describe what is visible in this frame in one concise sentence.\n\nAnswer:";
    let total_vision = top_indices.len() as u32;
    let mut enriched_count = 0usize;

    for (step_idx, &scene_idx) in top_indices.iter().enumerate() {
        if cancel_rx.try_recv().is_ok() {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(AppError::Pipeline("Анализ отменен пользователем.".to_string()));
        }

        let scene_ts = base_scenes[scene_idx].timestamp_sec;
        let frame_path = temp_dir.join(format!("scene_{:02}.jpg", step_idx + 1));

        // 1. Быстрое точечное извлечение кадра в точке сцены (-ss перед -i)
        if let Ok(ffmpeg_cmd) = shell.sidecar("ffmpeg") {
            let cmd = ffmpeg_cmd
                .env("PATH", crate::logger::build_sidecar_env_path(&app_handle))
                .args([
                    "-nostdin",
                    "-y",
                    "-ss",
                    &format!("{:.2}", scene_ts),
                    "-i",
                    &video_path.to_string_lossy(),
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=384:-2",
                    "-q:v",
                    "4",
                    &frame_path.to_string_lossy(),
                ]);
            if let Ok((mut rx, _child)) = cmd.spawn() {
                while let Some(ev) = rx.recv().await {
                    if matches!(ev, CommandEvent::Terminated(_)) {
                        break;
                    }
                }
            }
        }

        if !frame_path.exists() {
            continue;
        }

        let current_frame = (step_idx + 1) as u32;
        let percent = 90.0 + (current_frame as f64 / total_vision as f64) * 10.0;
        let _ = app_handle.emit(
            "analyzer-progress",
            AnalyzerProgressPayload {
                percent,
                stage: "analyzing_keyframes".to_string(),
                current_frame,
                total_frames: total_vision,
            },
        );

        let llama_cmd = match shell.sidecar(sidecar_name) {
            Ok(cmd) => cmd
                .env("PATH", crate::logger::build_sidecar_env_path(&app_handle))
                .args([
                    "-m",
                    &resolved_model.to_string_lossy(),
                    "--mmproj",
                    &resolved_mmproj.to_string_lossy(),
                    "--image",
                    &frame_path.to_string_lossy(),
                    "-e",
                    "-p",
                    vqa_prompt,
                    "-ngl",
                    "99",
                    "--temp",
                    "0.1",
                    "-c",
                    "2048",
                    "-n",
                    "64",
                ]),
            Err(_) => break,
        };

        if let Ok((mut rx, child)) = llama_cmd.spawn() {
            let app_handle_clone = app_handle.clone();
            let analyze_future = async {
                let mut stdout_accum = String::new();
                let mut stderr_accum = String::new();
                while let Some(event) = rx.recv().await {
                    match event {
                        CommandEvent::Stdout(bytes) => {
                            stdout_accum.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        CommandEvent::Stderr(bytes) => {
                            crate::logger::emit_stream_lines(&app_handle_clone, "Analyzer", "debug", &bytes);
                            stderr_accum.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        CommandEvent::Terminated(payload) => {
                            if payload.code == Some(0) {
                                return Ok(stdout_accum);
                            } else {
                                let err_msg = format!(
                                    "llama-vision завершился с кодом {:?}: {}",
                                    payload.code,
                                    stderr_accum.lines().last().unwrap_or("")
                                );
                                return Err(AppError::Pipeline(err_msg));
                            }
                        }
                        _ => {}
                    }
                }
                Err(AppError::Pipeline("Процесс анализа кадра прервался".to_string()))
            };

            let output_raw = tokio::select! {
                res = analyze_future => res.ok(),
                _ = cancel_rx.recv() => {
                    let _ = child.kill();
                    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                    return Err(AppError::Pipeline("Анализ отменен пользователем.".to_string()));
                }
            };

            if let Some(out) = output_raw {
                if let Some((desc, vis_bonus)) = parse_vision_description_and_bonus(&out) {
                    enriched_count += 1;
                    let old_int = base_scenes[scene_idx].intensity;
                    let new_int = (old_int + vis_bonus).clamp(0.58, 0.97);
                    base_scenes[scene_idx].intensity = new_int;
                    base_scenes[scene_idx].description = format!(
                        "{} | Vision: {}",
                        base_scenes[scene_idx].description, desc
                    );
                    base_scenes[scene_idx].actions.push("vision_verified".to_string());

                    crate::logger::emit_log(
                        &app_handle,
                        "Analyzer",
                        "info",
                        &format!(
                            "✓ Vision AI [кадр {}/{} @ {:.1}с]: «{}» (рейтинг сцены: {:.2} -> {:.2})",
                            current_frame, total_vision, scene_ts, desc, old_int, new_int
                        ),
                    );
                } else {
                    crate::logger::emit_log(
                        &app_handle,
                        "Analyzer",
                        "debug",
                        &format!(
                            "Vision AI [кадр {}/{} @ {:.1}с]: ответ модели отфильтрован как нерелевантный/отказ («{}»)",
                            current_frame, total_vision, scene_ts, out.trim().replace(['\r', '\n'], " ")
                        ),
                    );
                }
            }
        }
    }

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    if enriched_count > 0 {
        crate::logger::emit_log(
            &app_handle,
            "Analyzer",
            "info",
            &format!(
                "✓ Vision AI успешно проанализировал и обогатил {} ключевых сцен.",
                enriched_count
            ),
        );
    }

    let _ = app_handle.emit(
        "analyzer-progress",
        AnalyzerProgressPayload {
            percent: 100.0,
            stage: "complete".to_string(),
            current_frame: total_vision,
            total_frames: total_vision,
        },
    );

    Ok(base_scenes)
}

/// Извлекает осмысленное визуальное описание кадра из ответа `llama-vision` (Moondream 2 / LLaVA)
/// и рассчитывает бонус визуальной динамики/эмоциональности (`-0.02 .. +0.08`)
fn parse_vision_description_and_bonus(raw_out: &str) -> Option<(String, f64)> {
    let trimmed = raw_out.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. Если модель вернула JSON
    if let (Some(json_start), Some(json_end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if json_start < json_end {
            if let Ok(parsed) = serde_json::from_str::<LlamaOutput>(&trimmed[json_start..=json_end]) {
                if !parsed.description.trim().is_empty() {
                    let bonus = ((parsed.intensity - 0.65) * 0.2).clamp(-0.02, 0.08);
                    return Some((parsed.description.trim().to_string(), bonus));
                }
            }
        }
    }

    // 2. Очищаем текстовый ответ VQA-модели от эха промпта и вводных шаблонов
    let mut clean = trimmed
        .replace(['\r', '\n'], " ")
        .replace("towards the Assistant", "towards the camera")
        .replace("the Assistant", "the viewer");
    if let Some(ans_pos) = clean.rfind("Answer:") {
        clean = clean[ans_pos + 7..].trim().to_string();
    }

    // Очищаем префиксы персонажей вроде 'ASSISTANT: ', 'CHILD: ', 'USER: '
    for prefix in ["ASSISTANT:", "CHILD:", "USER:", "BOT:", "SYSTEM:"] {
        if let Some(pos) = clean.find(prefix) {
            clean = clean[pos + prefix.len()..].trim().to_string();
        }
    }

    // Если модель выдала шаблонный отказ про видео, но в конце описала сам кадр ("...It is a still image of a skull...")
    for marker in [
        "It is a still image of ",
        "The image shows ",
        "In this image, ",
        "This image shows ",
    ] {
        if let Some(pos) = clean.find(marker) {
            clean = clean[pos..].trim().to_string();
            break;
        }
    }

    let clean = clean.trim().trim_matches(['"', '\'', '`', ' ']).to_string();
    let lower = clean.to_lowercase();
    if clean.len() < 6 {
        return None;
    }

    // 3. Строгий фильтр отказов, галлюцинаций и шаблонных ответов VQA-моделей
    let refusal_signatures = [
        "i'm sorry", "i am sorry", "sorry, but", "sorry, i",
        "can't help", "cannot help", "can't assist", "cannot assist",
        "not a real assistant", "computer generated", "i am an ai", "as an ai",
        "i am a language model", "as a language model",
        "don't know what to do", "do not know what to do",
        "i cannot ", "i can't ", "unable to ", "i am unable",
        "i don't have access", "i do not have access",
        "as an assistant", "just a computer",
    ];

    if refusal_signatures.iter().any(|&sig| lower.contains(sig)) {
        return None;
    }

    // Защита от зацикливаний повтора («sorry, sorry...» или «child: child:»)
    if lower.matches("sorry").count() > 1 || lower.matches("assistant").count() > 1 {
        return None;
    }

    // Оцениваем визуальную насыщенность по ключевым признакам экшена, эмоций и крупных планов персонажей
    let high_energy_terms = [
        "fight", "battle", "attack", "explosion", "fire", "glow", "energy", "weapon", "sword",
        "gun", "rifle", "blood", "monster", "kaiju", "skull", "scream", "angry", "shock",
        "intense", "dramatic", "action", "running", "jumping", "laser", "lightning", "smoke",
    ];
    let character_terms = [
        "character", "person", "man", "woman", "girl", "boy", "soldier", "uniform", "face",
        "eyes", "expression", "looking", "holding", "standing", "talking", "close-up", "anime",
    ];

    let action_hits = high_energy_terms.iter().filter(|&&w| lower.contains(w)).count();
    let char_hits = character_terms.iter().filter(|&&w| lower.contains(w)).count();

    let mut bonus: f64 = 0.01;
    if action_hits >= 2 {
        bonus += 0.06;
    } else if action_hits == 1 {
        bonus += 0.04;
    }
    if char_hits >= 1 {
        bonus += 0.02;
    }

    let short_desc: String = clean.chars().take(110).collect();
    Some((short_desc.trim().to_string(), bonus.clamp(0.0, 0.08)))
}

/// Двухфакторный алгоритм анализа видео-хайлайтов (смена планов FFmpeg + энергетические пики аудиодорожки)
async fn run_two_factor_scene_analysis(
    video_path: PathBuf,
    app_handle: AppHandle,
    cancel_rx: Receiver<()>,
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
        let cut_density_bonus = ((local_cuts as f64 - 1.0) / 14.0).clamp(0.0, 0.18);

        // 2. Сопоставление со звуковой активностью
        let matching_audio = audio_ranges.iter().find(|(a_start, a_end)| {
            scene_t >= (*a_start - 1.5) && scene_t <= (*a_end + 1.5)
        });

        let is_near_audio = matching_audio.is_some();
        let audio_bonus = if let Some((a_start, a_end)) = matching_audio {
            let seg_len = (a_end - a_start).max(1.0);
            let rel_pos = ((scene_t - a_start) / seg_len).clamp(0.0, 1.0);
            let focus = 1.0 - (rel_pos - 0.35).abs();
            0.10 + (focus * 0.06).clamp(0.0, 0.06)
        } else {
            0.0
        };

        // 3. Защита от перекоса в сторону начальной заставки и конечных титров в длинных видео > 10 мин
        let narrative_factor = if duration > 600.0 && (scene_t < 60.0 || scene_t > duration - 60.0) {
            0.88
        } else {
            1.0
        };

        let raw_intensity = (0.58 + cut_density_bonus + audio_bonus) * narrative_factor;
        let intensity = raw_intensity.clamp(0.55, 0.95);

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

    let min_scene_dist = (duration / 35.0).clamp(15.0, 42.0);
    let target_scene_limit = (duration / 65.0).clamp(15.0, 30.0).round() as usize;
    let mut filtered_scenes = Vec::new();
    for sc in candidate_scenes {
        let too_close = filtered_scenes.iter().any(|f: &SceneCandidate| (f.timestamp_sec - sc.timestamp_sec).abs() < min_scene_dist);
        if !too_close {
            filtered_scenes.push(sc);
        }
        if filtered_scenes.len() >= target_scene_limit {
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

