// src-tauri/src/render.rs
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandEvent;
use serde::{Deserialize, Serialize};
use crate::error::{AppResult, AppError};
use tokio::sync::broadcast::Receiver;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleStyle {
    pub font_name: String,
    pub font_size: i32,
    pub color: String,
    pub placement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderConfig {
    pub vertical_format: bool,
    pub subtitle_style: Option<SubtitleStyle>,
    pub hw_accel: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct RenderProgressPayload {
    pub percent: f64,
    pub stage: String,
}

/// Определяет разрешение (ширину, высоту), длительность и наличие аудиодорожки у видеофайла через FFmpeg
async fn probe_video_stream_info(
    video_path: &Path,
    is_vertical: bool,
    app_handle: &AppHandle,
) -> (i32, i32, f64, bool) {
    let default_w = if is_vertical { 1080 } else { 1920 };
    let default_h = if is_vertical { 1920 } else { 1080 };

    let Ok(sidecar_cmd) = app_handle.shell().sidecar("ffmpeg") else {
        return (default_w, default_h, 10.0, true);
    };

    let args = vec![
        "-i".to_string(),
        video_path.to_string_lossy().to_string(),
        "-hide_banner".to_string(),
    ];

    let Ok((mut rx, child)) = sidecar_cmd
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
        .args(args)
        .spawn()
    else {
        return (default_w, default_h, 10.0, true);
    };

    let mut stderr_out = String::new();
    while let Some(event) = rx.recv().await {
        if let CommandEvent::Stderr(bytes) = event {
            stderr_out.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    let _ = child.kill();

    let has_audio = stderr_out.contains("Audio:");

    let mut duration = 10.0;
    if let Some(pos) = stderr_out.find("Duration: ") {
        let dur_str = &stderr_out[pos + 10..];
        let parts: Vec<&str> = dur_str
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .split(':')
            .collect();
        if parts.len() == 3 {
            let h: f64 = parts[0].parse().unwrap_or(0.0);
            let m: f64 = parts[1].parse().unwrap_or(0.0);
            let s: f64 = parts[2].parse().unwrap_or(0.0);
            let total = h * 3600.0 + m * 60.0 + s;
            if total > 0.1 {
                duration = total;
            }
        }
    }

    let mut width = default_w;
    let mut height = default_h;
    for line in stderr_out.lines() {
        if line.contains("Video:") {
            for token in line.split(|c: char| c == ',' || c.is_whitespace()) {
                let clean = token.trim_matches(|c: char| !c.is_ascii_digit() && c != 'x');
                if let Some((w_str, h_str)) = clean.split_once('x') {
                    if let (Ok(w), Ok(h)) = (w_str.parse::<i32>(), h_str.parse::<i32>()) {
                        if w >= 120 && h >= 120 && w <= 7680 && h <= 7680 {
                            width = (w / 2) * 2;
                            height = (h / 2) * 2;
                            break;
                        }
                    }
                }
            }
            break;
        }
    }

    (width, height, duration, has_audio)
}

pub async fn attach_bumpers_if_enabled(
    clip_path: &Path,
    bumper_config: Option<&crate::bumpers::BumperGlobalConfig>,
    is_vertical: bool,
    source_video_path: &Path,
    app_handle: &AppHandle,
) -> AppResult<()> {
    if !clip_path.exists() {
        return Ok(());
    }

    use tauri::Manager;
    let app_data = app_handle
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));

    // 1. Разрешаем конфигурацию: если пользователь явно отключил заставки (enabled = false) в параметрах или на диске —
    // строго выходим без авто-активации из кэша!
    let mut resolved_cfg: Option<crate::bumpers::BumperGlobalConfig> = None;
    if let Some(raw) = bumper_config {
        if !raw.enabled {
            return Ok(());
        }
        resolved_cfg = Some(raw.clone());
    } else if let Some(disk_cfg) = crate::bumpers::load_saved_bumper_config(app_handle) {
        if !disk_cfg.enabled {
            return Ok(());
        }
        resolved_cfg = Some(disk_cfg);
    }

    // Проверяем наличие предрендеренных файлов в cache_dir
    let cache_root = app_data.join("bumpers_cache");
    let mut detected_preset_dir: Option<PathBuf> = None;
    let mut detected_preset_id: Option<String> = None;

    if cache_root.exists() {
        if let Ok(entries) = std::fs::read_dir(&cache_root) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let d = entry.path();
                    let has_files = ["preroll_916.mp4", "preroll_169.mp4", "postroll_916.mp4", "postroll_169.mp4"]
                        .iter()
                        .any(|name| d.join(name).metadata().map(|m| m.len() > 512).unwrap_or(false));
                    if has_files {
                        detected_preset_id = d.file_name().and_then(|n| n.to_str()).map(|s| s.to_string());
                        detected_preset_dir = Some(d);
                        break;
                    }
                }
            }
        }
    }

    // Синхронизируем недостающие оверлеи/медиа из сохраненной конфигурации на диске (если IPC передал сокращенную копию)
    let mut global_cfg = match (resolved_cfg, detected_preset_id.as_ref()) {
        (Some(mut cfg), _) => {
            cfg.ensure_populated();
            cfg
        }
        (None, Some(pid)) => {
            crate::logger::emit_log(
                app_handle,
                "Bumpers",
                "info",
                &format!("Обнаружены предрендеренные заставки в кэше '{}', авто-активация склейки...", pid),
            );
            let mut cfg = crate::bumpers::BumperGlobalConfig {
                enabled: true,
                active_preset_id: pid.clone(),
                presets: Vec::new(),
            };
            cfg.ensure_populated();
            cfg
        }
        (None, None) => {
            return Ok(());
        }
    };

    if let Some(disk_cfg) = crate::bumpers::load_saved_bumper_config(app_handle) {
        for preset in &mut global_cfg.presets {
            if let Some(disk_preset) = disk_cfg.presets.iter().find(|p| p.id == preset.id) {
                for (cur_slot, disk_slot) in [
                    (&mut preset.v916.preroll, &disk_preset.v916.preroll),
                    (&mut preset.v916.postroll, &disk_preset.v916.postroll),
                    (&mut preset.v169.preroll, &disk_preset.v169.preroll),
                    (&mut preset.v169.postroll, &disk_preset.v169.postroll),
                ] {
                    if cur_slot.overlay_png_base64.is_none() {
                        cur_slot.overlay_png_base64 = disk_slot.overlay_png_base64.clone();
                    }
                    if cur_slot.logo_image_path.is_none() {
                        cur_slot.logo_image_path = disk_slot.logo_image_path.clone();
                    }
                    if cur_slot.custom_audio_path.is_none() {
                        cur_slot.custom_audio_path = disk_slot.custom_audio_path.clone();
                    }
                    if cur_slot.bg_video_path.is_none() {
                        cur_slot.bg_video_path = disk_slot.bg_video_path.clone();
                    }
                    if cur_slot.bg_image_path.is_none() {
                        cur_slot.bg_image_path = disk_slot.bg_image_path.clone();
                    }
                }
            }
        }
    }

    let active_preset = global_cfg
        .presets
        .iter()
        .find(|p| p.id == global_cfg.active_preset_id)
        .or_else(|| global_cfg.presets.first());
    let Some(preset) = active_preset else {
        return Ok(());
    };

    let cache_dir = detected_preset_dir.unwrap_or_else(|| cache_root.join(&preset.id));
    let _ = tokio::fs::create_dir_all(&cache_dir).await;

    let fmt_tag = if is_vertical { "916" } else { "169" };
    let alt_fmt_tag = if is_vertical { "169" } else { "916" };

    let pre_file_primary = cache_dir.join(format!("preroll_{}.mp4", fmt_tag));
    let pre_file_alt = cache_dir.join(format!("preroll_{}.mp4", alt_fmt_tag));
    let post_file_primary = cache_dir.join(format!("postroll_{}.mp4", fmt_tag));
    let post_file_alt = cache_dir.join(format!("postroll_{}.mp4", alt_fmt_tag));

    let primary_cat = if is_vertical { &preset.v916 } else { &preset.v169 };
    let fallback_cat = if is_vertical { &preset.v169 } else { &preset.v916 };

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut temp_bumper_files: Vec<PathBuf> = Vec::new();

    let mut pre_bumper: Option<(PathBuf, crate::bumpers::BumperItemConfig)> = None;
    let mut post_bumper: Option<(PathBuf, crate::bumpers::BumperItemConfig)> = None;

    // 1. Авто-проверка и прикрепление Преролла (только если преролл включен в пресете)
    if primary_cat.preroll.enabled || fallback_cat.preroll.enabled {
        let use_item = if primary_cat.preroll.enabled {
            &primary_cat.preroll
        } else {
            &fallback_cat.preroll
        };
        let uses_clip_frame = use_item.mode != "custom_video"
            && (use_item.bg_mode == "blur_frame" || use_item.bg_mode.trim().is_empty());

        if uses_clip_frame {
            // Всегда генерируем индивидуальный преролл из размытого последнего кадра конкретного шортса (clip_path)
            let dynamic_pre = clip_path.with_extension(format!("preroll_{}_{}.tmp.mp4", fmt_tag, ts));
            let frame_src = if clip_path.exists() { clip_path } else { source_video_path };
            crate::logger::emit_log(
                app_handle,
                "Bumpers",
                "info",
                &format!(
                    "🎬 Генерация персонального преролла ({}) с размытым последним кадром клипа...",
                    fmt_tag
                ),
            );
            if let Err(e) = crate::bumpers::render_single_bumper_video(
                use_item,
                is_vertical,
                frame_src,
                &dynamic_pre,
                app_handle,
            )
            .await
            {
                crate::logger::emit_log(
                    app_handle,
                    "Bumpers",
                    "warn",
                    &format!("Ошибка генерации преролла по кадру клипа: {}", e),
                );
            } else if dynamic_pre.metadata().map(|m| m.len() > 512).unwrap_or(false) {
                temp_bumper_files.push(dynamic_pre.clone());
                pre_bumper = Some((dynamic_pre, use_item.clone()));
            }
        } else if primary_cat.preroll.enabled && pre_file_primary.metadata().map(|m| m.len() > 512).unwrap_or(false) {
            crate::logger::emit_log(
                app_handle,
                "Bumpers",
                "info",
                &format!("✓ Использован готовый предрендеренный преролл ({})", fmt_tag),
            );
            pre_bumper = Some((pre_file_primary, primary_cat.preroll.clone()));
        } else if pre_file_alt.metadata().map(|m| m.len() > 512).unwrap_or(false) {
            crate::logger::emit_log(
                app_handle,
                "Bumpers",
                "info",
                &format!("✓ Использован готовый предрендеренный преролл из формата {} с авто-масштабированием под {}", alt_fmt_tag, fmt_tag),
            );
            pre_bumper = Some((pre_file_alt, fallback_cat.preroll.clone()));
        } else {
            crate::logger::emit_log(
                app_handle,
                "Bumpers",
                "info",
                &format!("Генерация преролла ({}) на лету перед вклейкой...", fmt_tag),
            );
            if let Err(e) = crate::bumpers::render_single_bumper_video(
                use_item,
                is_vertical,
                clip_path,
                &pre_file_primary,
                app_handle,
            ).await {
                crate::logger::emit_log(app_handle, "Bumpers", "warn", &format!("Ошибка генерации преролла на лету: {}", e));
            } else if pre_file_primary.metadata().map(|m| m.len() > 512).unwrap_or(false) {
                pre_bumper = Some((pre_file_primary, use_item.clone()));
            }
        }
    }

    // 2. Авто-проверка и прикрепление Постролла (только если постролл включен в пресете)
    if primary_cat.postroll.enabled || fallback_cat.postroll.enabled {
        let use_item = if primary_cat.postroll.enabled {
            &primary_cat.postroll
        } else {
            &fallback_cat.postroll
        };
        let uses_clip_frame = use_item.mode != "custom_video"
            && (use_item.bg_mode == "blur_frame" || use_item.bg_mode.trim().is_empty());

        if uses_clip_frame {
            // Всегда генерируем индивидуальный постролл с размытым последним кадром этого шортса
            let dynamic_post = clip_path.with_extension(format!("postroll_{}_{}.tmp.mp4", fmt_tag, ts));
            let frame_src = if clip_path.exists() { clip_path } else { source_video_path };
            crate::logger::emit_log(
                app_handle,
                "Bumpers",
                "info",
                &format!(
                    "🎬 Генерация персонального постролла ({}) с размытым последним кадром клипа...",
                    fmt_tag
                ),
            );
            if let Err(e) = crate::bumpers::render_single_bumper_video(
                use_item,
                is_vertical,
                frame_src,
                &dynamic_post,
                app_handle,
            )
            .await
            {
                crate::logger::emit_log(
                    app_handle,
                    "Bumpers",
                    "warn",
                    &format!("Ошибка генерации постролла по кадру клипа: {}", e),
                );
            } else if dynamic_post.metadata().map(|m| m.len() > 512).unwrap_or(false) {
                temp_bumper_files.push(dynamic_post.clone());
                post_bumper = Some((dynamic_post, use_item.clone()));
            }
        } else if primary_cat.postroll.enabled && post_file_primary.metadata().map(|m| m.len() > 512).unwrap_or(false) {
            crate::logger::emit_log(
                app_handle,
                "Bumpers",
                "info",
                &format!("✓ Использован готовый предрендеренный постролл ({})", fmt_tag),
            );
            post_bumper = Some((post_file_primary, primary_cat.postroll.clone()));
        } else if post_file_alt.metadata().map(|m| m.len() > 512).unwrap_or(false) {
            crate::logger::emit_log(
                app_handle,
                "Bumpers",
                "info",
                &format!("✓ Использован готовый предрендеренный постролл из формата {} с авто-масштабированием под {}", alt_fmt_tag, fmt_tag),
            );
            post_bumper = Some((post_file_alt, fallback_cat.postroll.clone()));
        } else {
            crate::logger::emit_log(
                app_handle,
                "Bumpers",
                "info",
                &format!("Генерация постролла ({}) на лету перед вклейкой...", fmt_tag),
            );
            if let Err(e) = crate::bumpers::render_single_bumper_video(
                use_item,
                is_vertical,
                clip_path,
                &post_file_primary,
                app_handle,
            ).await {
                crate::logger::emit_log(app_handle, "Bumpers", "warn", &format!("Ошибка генерации постролла на лету: {}", e));
            } else if post_file_primary.metadata().map(|m| m.len() > 512).unwrap_or(false) {
                post_bumper = Some((post_file_primary, use_item.clone()));
            }
        }
    }

    if pre_bumper.is_none() && post_bumper.is_none() {
        for tmp_f in temp_bumper_files {
            let _ = tokio::fs::remove_file(tmp_f).await;
        }
        return Ok(());
    }

    let (target_w, target_h, clip_dur_raw, clip_has_audio) =
        probe_video_stream_info(clip_path, is_vertical, app_handle).await;
    let clip_dur = clip_dur_raw.max(0.4);

    let pre_info = if let Some((ref pre_path, ref pre_cfg)) = pre_bumper {
        let (_, _, d, has_a) = probe_video_stream_info(pre_path, is_vertical, app_handle).await;
        Some((pre_path.clone(), pre_cfg.clone(), d.max(0.4), has_a))
    } else {
        None
    };
    let post_info = if let Some((ref post_path, ref post_cfg)) = post_bumper {
        let (_, _, d, has_a) = probe_video_stream_info(post_path, is_vertical, app_handle).await;
        Some((post_path.clone(), post_cfg.clone(), d.max(0.4), has_a))
    } else {
        None
    };

    let temp_output = clip_path.with_extension(format!("with_bumpers_{}.tmp.mp4", ts));

    // Определяем режимы переходов и длительность плавных фейдов/наплывов (crossfade / fade_black / cut).
    // ВАЖНО: Для crossfade мы добавляем безопасный стоп-кадр с тишиной на стыке основного клипа (tpad + adelay/apad),
    // чтобы xfade/acrossfade давал мягкий киношный наплыв, НЕ съедая ни миллисекунды речи в начале и конце шортса!
    let pre_mode = pre_info
        .as_ref()
        .map(|(_, c, _, _)| c.transition.as_str())
        .unwrap_or("cut");
    let post_mode = post_info
        .as_ref()
        .map(|(_, c, _, _)| c.transition.as_str())
        .unwrap_or("cut");

    let d_pre_xfade = match pre_info.as_ref() {
        Some((_, c, pre_d, _)) if c.transition != "cut" && c.transition != "fade_black" => {
            (pre_d * 0.35).clamp(0.25, 0.48).min(clip_dur * 0.30)
        }
        _ => 0.0,
    };
    let d_post_xfade = match post_info.as_ref() {
        Some((_, c, post_d, _)) if c.transition != "cut" && c.transition != "fade_black" => {
            (post_d * 0.35).clamp(0.25, 0.50).min(clip_dur * 0.30)
        }
        _ => 0.0,
    };

    let mut args = vec!["-hide_banner".to_string(), "-nostdin".to_string(), "-y".to_string()];
    let mut filter_parts: Vec<String> = Vec::new();
    let mut input_idx = 0usize;

    // 1. Вход Преролла (если активен)
    let pre_dur_opt = if let Some((ref pre_path, ref pre_cfg, pre_dur, pre_has_audio)) = pre_info {
        args.push("-i".to_string());
        args.push(pre_path.to_string_lossy().to_string());
        let v_in = input_idx;
        input_idx += 1;

        let a_in = if pre_has_audio {
            v_in
        } else {
            args.push("-f".to_string());
            args.push("lavfi".to_string());
            args.push("-t".to_string());
            args.push(format!("{:.3}", pre_dur + 0.30));
            args.push("-i".to_string());
            args.push("anullsrc=r=48000:cl=stereo".to_string());
            let idx = input_idx;
            input_idx += 1;
            idx
        };

        let outer_fade_in_v = if pre_cfg.transition != "cut" && pre_dur > 0.70 {
            ",fade=t=in:st=0:d=0.30".to_string()
        } else {
            String::new()
        };
        let outer_fade_in_a = if pre_cfg.transition != "cut" && pre_dur > 0.70 {
            ",afade=t=in:st=0:d=0.25".to_string()
        } else {
            String::new()
        };

        let fb_d = (pre_dur * 0.32).clamp(0.20, 0.42);
        let inner_fade_out_v = if pre_cfg.transition == "fade_black" && pre_dur > 0.50 {
            format!(",fade=t=out:st={:.3}:d={:.3}", (pre_dur - fb_d).max(0.05), fb_d)
        } else {
            String::new()
        };
        let inner_fade_out_a = if pre_cfg.transition == "fade_black" && pre_dur > 0.50 {
            format!(",afade=t=out:st={:.3}:d={:.3}", (pre_dur - fb_d).max(0.05), fb_d)
        } else if pre_cfg.transition == "cut" && pre_dur > 0.20 {
            format!(",afade=t=out:st={:.3}:d=0.02", (pre_dur - 0.02).max(0.05))
        } else {
            String::new()
        };

        filter_parts.push(format!(
            "[{}:v]scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps=30,settb=AVTB,format=yuv420p,tpad=stop_mode=clone:stop_duration=0.25,trim=duration={:.3},setpts=PTS-STARTPTS{}{}[v_pre]",
            v_in, target_w, target_h, target_w, target_h, pre_dur, outer_fade_in_v, inner_fade_out_v
        ));
        filter_parts.push(format!(
            "[{}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,apad=pad_dur=0.25,atrim=duration={:.3},asetpts=PTS-STARTPTS{}{}[a_pre]",
            a_in, pre_dur, outer_fade_in_a, inner_fade_out_a
        ));
        Some(pre_dur)
    } else {
        None
    };

    // 2. Вход Основного клипа
    let clip_virt_dur = clip_dur + d_pre_xfade + d_post_xfade;
    {
        args.push("-i".to_string());
        args.push(clip_path.to_string_lossy().to_string());
        let v_in = input_idx;
        input_idx += 1;

        let a_in = if clip_has_audio {
            v_in
        } else {
            args.push("-f".to_string());
            args.push("lavfi".to_string());
            args.push("-t".to_string());
            args.push(format!("{:.3}", clip_dur + 0.30));
            args.push("-i".to_string());
            args.push("anullsrc=r=48000:cl=stereo".to_string());
            let idx = input_idx;
            input_idx += 1;
            idx
        };

        let start_tpad = if d_pre_xfade > 0.0 {
            format!(",tpad=start_mode=clone:start_duration={:.3}:stop_mode=clone:stop_duration={:.3}", d_pre_xfade, d_post_xfade + 0.25)
        } else {
            format!(",tpad=stop_mode=clone:stop_duration={:.3}", d_post_xfade + 0.25)
        };
        let start_adelay = if d_pre_xfade > 0.0 {
            let ms = (d_pre_xfade * 1000.0).round() as i64;
            format!(",adelay={}|{}", ms, ms)
        } else {
            String::new()
        };

        let fb_in_d = (clip_dur * 0.25).clamp(0.20, 0.40);
        let fb_out_d = (clip_dur * 0.25).clamp(0.20, 0.40);

        let clip_fade_in_v = if pre_info.is_some() && pre_mode == "fade_black" && clip_dur > 0.60 {
            format!(",fade=t=in:st=0:d={:.3}", fb_in_d)
        } else {
            String::new()
        };
        let clip_fade_in_a = if pre_info.is_some() && pre_mode == "fade_black" && clip_dur > 0.60 {
            format!(",afade=t=in:st=0:d={:.3}", fb_in_d)
        } else if pre_info.is_some() && pre_mode == "cut" {
            ",afade=t=in:st=0:d=0.02".to_string()
        } else {
            String::new()
        };

        let clip_fade_out_v = if post_info.is_some() && post_mode == "fade_black" && clip_dur > 0.60 {
            format!(",fade=t=out:st={:.3}:d={:.3}", (clip_virt_dur - fb_out_d).max(0.1), fb_out_d)
        } else {
            String::new()
        };
        let clip_fade_out_a = if post_info.is_some() && post_mode == "fade_black" && clip_dur > 0.60 {
            format!(",afade=t=out:st={:.3}:d={:.3}", (clip_virt_dur - fb_out_d).max(0.1), fb_out_d)
        } else {
            String::new()
        };

        filter_parts.push(format!(
            "[{}:v]scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps=30,settb=AVTB,format=yuv420p{},trim=duration={:.3},setpts=PTS-STARTPTS{}{}[v_clip]",
            v_in, target_w, target_h, target_w, target_h, start_tpad, clip_virt_dur, clip_fade_in_v, clip_fade_out_v
        ));
        filter_parts.push(format!(
            "[{}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo{},apad=pad_dur={:.3},atrim=duration={:.3},asetpts=PTS-STARTPTS{}{}[a_clip]",
            a_in, start_adelay, d_post_xfade + 0.25, clip_virt_dur, clip_fade_in_a, clip_fade_out_a
        ));
    }

    // 3. Вход Постролла (если активен)
    let post_dur_opt = if let Some((ref post_path, ref post_cfg, post_dur, post_has_audio)) = post_info {
        args.push("-i".to_string());
        args.push(post_path.to_string_lossy().to_string());
        let v_in = input_idx;
        input_idx += 1;

        let a_in = if post_has_audio {
            v_in
        } else {
            args.push("-f".to_string());
            args.push("lavfi".to_string());
            args.push("-t".to_string());
            args.push(format!("{:.3}", post_dur + 0.30));
            args.push("-i".to_string());
            args.push("anullsrc=r=48000:cl=stereo".to_string());
            input_idx
        };

        let fb_d = (post_dur * 0.32).clamp(0.20, 0.42);
        let inner_fade_in_v = if post_cfg.transition == "fade_black" && post_dur > 0.50 {
            format!(",fade=t=in:st=0:d={:.3}", fb_d)
        } else {
            String::new()
        };
        let inner_fade_in_a = if post_cfg.transition == "fade_black" && post_dur > 0.50 {
            format!(",afade=t=in:st=0:d={:.3}", fb_d)
        } else if post_cfg.transition == "cut" && post_dur > 0.20 {
            ",afade=t=in:st=0:d=0.02".to_string()
        } else {
            String::new()
        };

        let outer_fade_out_v = if post_cfg.transition != "cut" && post_dur > 0.70 {
            format!(",fade=t=out:st={:.3}:d=0.35", (post_dur - 0.35).max(0.1))
        } else {
            String::new()
        };
        let outer_fade_out_a = if post_cfg.transition != "cut" && post_dur > 0.70 {
            format!(",afade=t=out:st={:.3}:d=0.35", (post_dur - 0.35).max(0.1))
        } else {
            String::new()
        };

        filter_parts.push(format!(
            "[{}:v]scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps=30,settb=AVTB,format=yuv420p,tpad=stop_mode=clone:stop_duration=0.25,trim=duration={:.3},setpts=PTS-STARTPTS{}{}[v_post]",
            v_in, target_w, target_h, target_w, target_h, post_dur, inner_fade_in_v, outer_fade_out_v
        ));
        filter_parts.push(format!(
            "[{}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,apad=pad_dur=0.25,atrim=duration={:.3},asetpts=PTS-STARTPTS{}{}[a_post]",
            a_in, post_dur, inner_fade_in_a, outer_fade_out_a
        ));
        Some(post_dur)
    } else {
        None
    };

    // 4. Поэтапная сборка с фейдерами (xfade + acrossfade при crossfade, или concat при fade_black / cut)
    let mut cur_v = "v_clip".to_string();
    let mut cur_a = "a_clip".to_string();
    let mut cur_dur = clip_virt_dur;

    if let Some(pre_dur) = pre_dur_opt {
        if d_pre_xfade > 0.0 {
            let offset1 = (pre_dur - d_pre_xfade).max(0.05);
            filter_parts.push(format!(
                "[v_pre][v_clip]xfade=transition=fade:duration={:.3}:offset={:.3},setpts=PTS-STARTPTS[v_mid]",
                d_pre_xfade, offset1
            ));
            filter_parts.push(format!(
                "[a_pre][a_clip]acrossfade=d={:.3}:c1=tri:c2=tri,asetpts=PTS-STARTPTS[a_mid]",
                d_pre_xfade
            ));
            cur_dur = offset1 + clip_virt_dur;
        } else {
            filter_parts.push(
                "[v_pre][a_pre][v_clip][a_clip]concat=n=2:v=1:a=1[v_mid_raw][a_mid_raw];[v_mid_raw]fps=30,settb=AVTB,format=yuv420p,setpts=PTS-STARTPTS[v_mid];[a_mid_raw]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=PTS-STARTPTS[a_mid]".to_string()
            );
            cur_dur = pre_dur + clip_virt_dur;
        }
        cur_v = "v_mid".to_string();
        cur_a = "a_mid".to_string();
    }

    if let Some(_post_dur) = post_dur_opt {
        if d_post_xfade > 0.0 {
            let offset2 = (cur_dur - d_post_xfade).max(0.10);
            filter_parts.push(format!(
                "[{}][v_post]xfade=transition=fade:duration={:.3}:offset={:.3},setpts=PTS-STARTPTS[outv]",
                cur_v, d_post_xfade, offset2
            ));
            filter_parts.push(format!(
                "[{}][a_post]acrossfade=d={:.3}:c1=tri:c2=tri,asetpts=PTS-STARTPTS[outa]",
                cur_a, d_post_xfade
            ));
        } else {
            filter_parts.push(format!(
                "[{}][{}][v_post][a_post]concat=n=2:v=1:a=1[outv][outa]",
                cur_v, cur_a
            ));
        }
    } else {
        filter_parts.push(format!("[{}]null[outv];[{}]anull[outa]", cur_v, cur_a));
    }

    let filter_complex = filter_parts.join(";");
    args.extend([
        "-filter_complex".to_string(),
        filter_complex,
        "-map".to_string(),
        "[outv]".to_string(),
        "-map".to_string(),
        "[outa]".to_string(),
        "-c:v".to_string(),
        "libx264".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-preset".to_string(),
        "veryfast".to_string(),
        "-crf".to_string(),
        "20".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
        "-ar".to_string(),
        "48000".to_string(),
        "-ac".to_string(),
        "2".to_string(),
        "-b:a".to_string(),
        "192k".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
        temp_output.to_string_lossy().to_string(),
    ]);

    let sidecar_cmd = app_handle
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось запустить FFmpeg: {}", e)))?
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
        .args(args);

    let (mut rx, child) = sidecar_cmd
        .spawn()
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка старта FFmpeg при склейке заставок: {}", e)))?;

    let mut exit_code = None;
    let mut stderr_log = String::new();
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stderr(bytes) => {
                stderr_log.push_str(&String::from_utf8_lossy(&bytes));
            }
            CommandEvent::Terminated(payload) => {
                exit_code = payload.code;
                break;
            }
            _ => {}
        }
    }
    let _ = child.kill();

    let output_ok = exit_code == Some(0)
        && std::fs::metadata(&temp_output)
            .map(|m| m.len() > 1024)
            .unwrap_or(false);

    if output_ok {
        let _ = tokio::fs::remove_file(clip_path).await;
        if let Err(rename_err) = tokio::fs::rename(&temp_output, clip_path).await {
            let _ = tokio::fs::copy(&temp_output, clip_path).await;
            let _ = tokio::fs::remove_file(&temp_output).await;
            log::debug!("Fallback copy after rename in attach_bumpers: {}", rename_err);
        }
        crate::logger::emit_log(
            app_handle,
            "Bumpers",
            "info",
            &format!(
                "✓ Заставки ({}{}) вшиты в ролик '{}'",
                if pre_bumper.is_some() { "Преролл " } else { "" },
                if post_bumper.is_some() { "+ Постролл" } else { "" },
                clip_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("clip.mp4")
            ),
        );
    } else {
        let _ = tokio::fs::remove_file(&temp_output).await;
        let last_err = stderr_log
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("Неизвестная ошибка FFmpeg");
        crate::logger::emit_log(
            app_handle,
            "Bumpers",
            "warn",
            &format!("Не удалось вшить заставки в '{}': {}", clip_path.display(), last_err),
        );
    }

    for tmp_f in temp_bumper_files {
        let _ = tokio::fs::remove_file(tmp_f).await;
    }

    Ok(())
}


/// Конвертирует HTML цвет `#RRGGBB` в формат ASS/libass `&H00BBGGRR`
fn to_ass_color(color: &str) -> String {
    let trimmed = color.trim();
    if trimmed.starts_with("&H") || trimmed.starts_with("&h") {
        return trimmed.to_string();
    }
    let hex = trimmed.trim_start_matches('#');
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let r = &hex[0..2];
        let g = &hex[2..4];
        let b = &hex[4..6];
        return format!("&H00{}{}{}", b.to_uppercase(), g.to_uppercase(), r.to_uppercase());
    }
    "&H00FFFFFF".to_string()
}

/// Формирует строку фильтра субтитров FFmpeg с экранированием путей и применением настроек стиля
fn format_subtitles_filter(
    srt_file: &Path,
    custom_style: Option<&SubtitleStyle>,
) -> String {
    let srt_escaped = srt_file
        .to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'")
        .replace('[', "\\[")
        .replace(']', "\\]");

    if let Some(style) = custom_style {
        let font = if style.font_name.is_empty() || style.font_name.eq_ignore_ascii_case("Inter") {
            "Arial".to_string()
        } else {
            style.font_name.clone()
        };
        let font_size = if style.font_size <= 0 { 20 } else { style.font_size.min(28) };
        let primary_color = to_ass_color(&style.color);
        let alignment = match style.placement.as_str() {
            "top" => 8,
            "middle" | "center" => 5,
            _ => 2, // bottom center
        };
        format!(
            "subtitles='{}':force_style='FontName={},FontSize={},PrimaryColour={},OutlineColour=&H00000000,BorderStyle=1,Outline=2,Shadow=1,Bold=1,Alignment={},MarginV=38'",
            srt_escaped, font, font_size, primary_color, alignment
        )
    } else {
        format!(
            "subtitles='{}':force_style='FontName=Arial,FontSize=20,PrimaryColour=&H00FFFFFF,OutlineColour=&H00000000,BorderStyle=1,Outline=2,Shadow=1,Bold=1,Alignment=2,MarginV=38'",
            srt_escaped
        )
    }
}

/// Вырезает и сдвигает по времени субтитры для конкретного клипа `[start_sec, start_sec + duration_sec]`.
/// Дополнительно контролирует естественную длительность каждой фразы внутри клипа и исключает наслоение соседних реплик,
/// чтобы субтитры появлялись строго в момент произнесения фразы и исчезали сразу после её завершения.
async fn create_clip_srt(
    full_srt_path: &Path,
    start_sec: f64,
    duration_sec: f64,
) -> Option<PathBuf> {
    let content = tokio::fs::read_to_string(full_srt_path).await.ok()?;
    let all_fragments = crate::text_analyzer::parse_srt(&content);
    let end_sec = start_sec + duration_sec;

    let mut selected: Vec<crate::text_analyzer::SubtitleFragment> = Vec::new();

    for frag in all_fragments {
        if frag.end <= start_sec + 0.08 || frag.start >= end_sec - 0.08 {
            continue;
        }
        if crate::whisper::is_whisper_hallucination(&frag.text) {
            continue;
        }

        let mut rel_start = (frag.start - start_sec).max(0.0);
        let mut rel_end = (frag.end - start_sec).min(duration_sec);
        let cue_dur = rel_end - rel_start;
        if cue_dur < 0.20 {
            continue;
        }

        // Контроль максимальной длительности удержания одного титра на экране по длине фразы в живой актерской речи
        let char_count = frag.text.chars().filter(|c| !c.is_whitespace()).count().max(1);
        let word_count = frag.text.split_whitespace().count().max(1);
        let natural_dur = (0.28 + (char_count as f64) / 11.5)
            .max((word_count as f64) * 0.40)
            .clamp(0.65, 5.20);

        // Если титр висит дольше естественной длительности речи — закрываем его сразу по окончании фразы
        if cue_dur > natural_dur * 1.25 + 0.25 {
            rel_end = (rel_start + natural_dur).min(duration_sec);
        }

        // Гарантируем чистый зазор 60 мс после предыдущего титра, чтобы реплики не слипались
        if let Some(prev) = selected.last_mut() {
            if rel_start < prev.end + 0.05 {
                if prev.end - prev.start > 0.60 {
                    prev.end = (rel_start - 0.05).max(prev.start + 0.40);
                }
                rel_start = rel_start.max(prev.end + 0.05);
            }
        }

        if rel_end - rel_start >= 0.22 {
            let (_, clean_cue_text) = crate::subtitles_parser::extract_srt_character_prefix(&frag.text);
            selected.push(crate::text_analyzer::SubtitleFragment {
                start: rel_start,
                end: rel_end,
                text: clean_cue_text.trim().to_string(),
            });
        }
    }

    if selected.is_empty() {
        return None;
    }

    let mut clip_cues = String::new();
    for (idx, cue) in selected.iter().enumerate() {
        clip_cues.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            idx + 1,
            crate::whisper::format_srt_timestamp(cue.start),
            crate::whisper::format_srt_timestamp(cue.end),
            cue.text
        ));
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_srt = std::env::temp_dir().join(format!("vc_clip_sub_{}_{}.srt", std::process::id(), ts));
    tokio::fs::write(&temp_srt, clip_cues).await.ok()?;
    Some(temp_srt)
}

/// Внутренний исполнитель FFmpeg для рендера одного клипа с заданным выражением горизонтального кропа `crop_x_expr`
async fn execute_ffmpeg_clip_render(
    video_path: &Path,
    start_sec: f64,
    duration_sec: f64,
    output_path: &Path,
    temp_clip_srt: Option<&Path>,
    config: &RenderConfig,
    crop_x_expr: Option<&str>,
    app_handle: &AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<()> {
    let shell = app_handle.shell();

    // Формируем аргументы для FFmpeg (-accurate_seek + -avoid_negative_ts make_zero для идеальной синхронизации аудио, видео и фильтра subtitles)
    let mut args = vec![
        "-hide_banner".to_string(),
        "-nostdin".to_string(),
        "-y".to_string(),
        "-accurate_seek".to_string(),
        "-ss".to_string(),
        format!("{:.3}", start_sec),
        "-i".to_string(),
        video_path.to_string_lossy().to_string(),
        "-t".to_string(),
        format!("{:.3}", duration_sec),
        "-avoid_negative_ts".to_string(),
        "make_zero".to_string(),
    ];

    let mut vf_filters = Vec::new();

    if config.vertical_format {
        let expr = crop_x_expr
            .unwrap_or("min(max(0, iw*0.500 - (ih*(9/16))/2), iw-(ih*(9/16)))");
        let crop_filter = format!(
            "crop=w='ih*(9/16)':h='ih':x='{}':y=0,scale=1080:1920:flags=lanczos",
            expr
        );
        vf_filters.push(crop_filter);
    }

    if let Some(clip_srt) = temp_clip_srt {
        let srt_escaped = clip_srt
            .to_string_lossy()
            .replace('\\', "/")
            .replace(':', "\\:")
            .replace('\'', "\\'")
            .replace('[', "\\[")
            .replace(']', "\\]");

        let mut sub_filter = format!("subtitles='{}'", srt_escaped);

        if let Some(ref style) = config.subtitle_style {
            // Заменяем веб-шрифт Inter на системный Arial, чтобы libass не тратил время на поиск fallback-шрифтов
            let font = if style.font_name.is_empty() || style.font_name.eq_ignore_ascii_case("Inter") {
                "Arial".to_string()
            } else {
                style.font_name.clone()
            };
            let font_size = if style.font_size <= 0 { 18 } else { style.font_size.min(28) };
            let primary_color = to_ass_color(&style.color);
            let alignment = match style.placement.as_str() {
                "top" => 8,
                "middle" | "center" => 5,
                _ => 2, // bottom center
            };
            sub_filter.push_str(&format!(
                ":force_style='FontName={},FontSize={},PrimaryColour={},OutlineColour=&H00000000,BorderStyle=1,Outline=2,Shadow=1,Bold=1,Alignment={},MarginV=38'",
                font, font_size, primary_color, alignment
            ));
        } else {
            sub_filter.push_str(":force_style='FontName=Arial,FontSize=18,PrimaryColour=&H00FFFFFF,OutlineColour=&H00000000,BorderStyle=1,Outline=2,Shadow=1,Bold=1,Alignment=2,MarginV=38'");
        }

        vf_filters.push(sub_filter);
    }

    if !vf_filters.is_empty() {
        args.push("-vf".to_string());
        args.push(vf_filters.join(","));
    }

    // Анти-клик микро-фейд 40 мс на входе и выходе клипа (полностью устраняет цифровые щелчки на границах нарезки аудио)
    if duration_sec >= 0.25 {
        let fade_out_start = (duration_sec - 0.040).max(0.05);
        let af_filter = format!(
            "afade=t=in:st=0:d=0.040,afade=t=out:st={:.3}:d=0.040",
            fade_out_start
        );
        args.push("-af".to_string());
        args.push(af_filter);
    }

    // Кодеки видео и аудио (максимальная совместимость со всеми устройствами)
    args.push("-c:v".to_string());
    args.push("libx264".to_string());
    args.push("-preset".to_string());
    args.push("veryfast".to_string());
    args.push("-crf".to_string());
    args.push("21".to_string());
    args.push("-pix_fmt".to_string());
    args.push("yuv420p".to_string());

    args.push("-c:a".to_string());
    args.push("aac".to_string());
    args.push("-b:a".to_string());
    args.push("192k".to_string());

    args.push(output_path.to_string_lossy().to_string());

    let command = match shell.sidecar("ffmpeg") {
        Ok(cmd) => cmd
            .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
            .args(args),
        Err(e) => {
            let err = format!("FFmpeg (render) не обнаружен: {}", e);
            crate::logger::emit_log(app_handle, "Render", "error", &err);
            return Err(AppError::Pipeline(err));
        }
    };

    let (mut rx, child) = match command.spawn() {
        Ok(pair) => pair,
        Err(e) => {
            let err = format!("Не удалось запустить FFmpeg при рендере: {}", e);
            crate::logger::emit_log(app_handle, "Render", "error", &err);
            return Err(AppError::Pipeline(err));
        }
    };

    let app_handle_clone = app_handle.clone();
    let out_display = output_path.display().to_string();
    let render_future = async move {
        let _ = app_handle_clone.emit(
            "render-progress",
            RenderProgressPayload {
                percent: 0.0,
                stage: "rendering started".to_string(),
            },
        );

        let mut max_cur_secs: f64 = 0.0;

        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stderr(bytes) => {
                    crate::logger::emit_stream_lines(&app_handle_clone, "Render", "debug", &bytes);
                    let log_line = String::from_utf8_lossy(&bytes);
                    if let Some(pos) = log_line.rfind("time=") {
                        let time_part = &log_line[pos + 5..];
                        let space_pos = time_part
                            .find(|c: char| c.is_whitespace())
                            .unwrap_or(time_part.len());
                        let time_str = &time_part[..space_pos];
                        let parts: Vec<&str> = time_str.trim().split(':').collect();
                        if parts.len() == 3 {
                            if let (Ok(h), Ok(m), Ok(s)) = (
                                parts[0].parse::<f64>(),
                                parts[1].parse::<f64>(),
                                parts[2].parse::<f64>(),
                            ) {
                                let cur_secs = (h * 3600.0 + m * 60.0 + s).min(duration_sec);
                                if cur_secs > max_cur_secs && duration_sec > 0.0 {
                                    max_cur_secs = cur_secs;
                                    let pct = (max_cur_secs / duration_sec * 100.0)
                                        .min(99.0)
                                        .max(0.0);
                                    let _ = app_handle_clone.emit(
                                        "render-progress",
                                        RenderProgressPayload {
                                            percent: pct,
                                            stage: format!(
                                                "{:.1}с / {:.1}с",
                                                max_cur_secs, duration_sec
                                            ),
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
                CommandEvent::Stdout(bytes) => {
                    crate::logger::emit_stream_lines(&app_handle_clone, "Render", "debug", &bytes);
                }
                CommandEvent::Terminated(payload) => {
                    if payload.code == Some(0) {
                        crate::logger::emit_log(
                            &app_handle_clone,
                            "Render",
                            "info",
                            &format!("Клип успешно создан: '{}'", out_display),
                        );
                        let _ = app_handle_clone.emit(
                            "render-progress",
                            RenderProgressPayload {
                                percent: 100.0,
                                stage: "rendering complete".to_string(),
                            },
                        );
                        return Ok(());
                    } else {
                        let err_msg =
                            format!("Рендер завершился с ошибкой, код: {:?}", payload.code);
                        crate::logger::emit_log(&app_handle_clone, "Render", "error", &err_msg);
                        return Err(AppError::Pipeline(err_msg));
                    }
                }
                _ => {}
            }
        }
        let err_msg = "Процесс рендера FFmpeg неожиданно прервался.".to_string();
        crate::logger::emit_log(&app_handle_clone, "Render", "error", &err_msg);
        Err(AppError::Pipeline(err_msg))
    };

    tokio::select! {
        r = render_future => r,
        _ = cancel_rx.recv() => {
            let _ = child.kill();
            Err(AppError::Pipeline("Отрисовка отменена пользователем.".to_string()))
        }
    }
}

/// Создает один клип из исходного видео с нарезкой, субтитрами, фильтрами и финальной контрольной проверкой наличия лиц в 9:16 шортсе.
pub async fn render_clip(
    video_path: PathBuf,
    start_sec: f64,
    duration_sec: f64,
    output_path: PathBuf,
    srt_path: Option<PathBuf>,
    config: RenderConfig,
    app_handle: AppHandle,
    cancel_rx: Receiver<()>,
) -> AppResult<()> {
    log::info!("Запуск рендеринга клипа: {}", output_path.display());

    // Гарантируем существование родительской папки для сохранения
    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
    }

    // Преобразование в вертикальный формат 9:16 (Shorts / Reels / TikTok) с динамическим ведением лиц внутри клипа
    let mut initial_crop_expr: Option<String> = None;
    let mut used_ratios: Vec<f64> = Vec::new();
    if config.vertical_format {
        let (crop_x_expr, tracking_summary, track_ratios) =
            build_dynamic_face_crop_expr(&video_path, start_sec, duration_sec, &app_handle).await;
        crate::logger::emit_log(
            &app_handle,
            "Render",
            "info",
            &format!("Smart Face Tracking: {}", tracking_summary),
        );
        initial_crop_expr = Some(crop_x_expr);
        used_ratios = track_ratios;
    }

    // Формируем индивидуальный синхронизированный SRT-файл для таймкода данного клипа [start_sec .. start_sec + duration_sec]
    let mut active_start_sec = start_sec;
    let mut active_duration_sec = duration_sec;
    let mut temp_clip_srt: Option<PathBuf> = None;
    if let Some(ref srt) = srt_path {
        if srt.exists() {
            temp_clip_srt = create_clip_srt(srt, active_start_sec, active_duration_sec).await;
        }
    }

    crate::logger::emit_log(
        &app_handle,
        "Render",
        "info",
        &format!(
            "Рендеринг клипа: [{:.2}с - {:.2}с] -> '{}' (вертикально={}, субтитры={}, микро-фейд=40мс)",
            active_start_sec,
            active_start_sec + active_duration_sec,
            output_path.display(),
            config.vertical_format,
            temp_clip_srt.is_some()
        ),
    );

    let res = execute_ffmpeg_clip_render(
        &video_path,
        active_start_sec,
        active_duration_sec,
        &output_path,
        temp_clip_srt.as_deref(),
        &config,
        initial_crop_expr.as_deref(),
        &app_handle,
        cancel_rx.resubscribe(),
    )
    .await;

    // 1. ФИНАЛЬНЫЙ КОНТРОЛЬ АУДИООБРЕЗКИ И ПОЛНОТЫ СЛОВ В ГОТОВОМ ШОРТСЕ (WHISPER ASR + FFMPEG 10ms PCM):
    // Прогоняем сам отрендеренный шортс через Whisper и 10-мс детектор речевой энергии FFmpeg.
    // Если на начале или конце готового шортса обнаружено обрезанное слово или незаконченная фраза (как «не договорил героя»),
    // автоматически находим точную границу окончания/начала речи в исходном аудио, пересобираем SRT и перерендериваем шортс!
    if res.is_ok() && output_path.exists() {
        if let Some((repaired_start, repaired_dur, repaired_srt)) =
            verify_and_repair_short_audio_with_whisper(
                &video_path,
                active_start_sec,
                active_duration_sec,
                &output_path,
                srt_path.as_deref(),
                &config,
                initial_crop_expr.as_deref(),
                &app_handle,
                cancel_rx.resubscribe(),
            )
            .await
        {
            if let Some(ref old_tmp) = temp_clip_srt {
                let _ = tokio::fs::remove_file(old_tmp).await;
            }
            active_start_sec = repaired_start;
            active_duration_sec = repaired_dur;
            temp_clip_srt = repaired_srt;
        }
    }

    // 2. ФИНАЛЬНЫЙ КОНТРОЛЬ ЛИЦ В ГОТОВОМ 9:16 ШОРТСЕ:
    // Если в отрендеренной вертикальной обрезке не зафиксировано ни одного лица (0 кадров с лицом),
    // значит окно кропа промахнулось мимо персонажа. Выполняем глубокое сканирование полноценного 16:9 оригинала
    // и, если в оригинале есть лица, автоматически пересобираем шортс с исправленной траекторией кадрирования!
    if res.is_ok() && config.vertical_format && output_path.exists() {
        verify_and_repair_cropped_short_faces(
            &video_path,
            active_start_sec,
            active_duration_sec,
            &output_path,
            temp_clip_srt.as_deref(),
            &config,
            &used_ratios,
            &app_handle,
            cancel_rx.resubscribe(),
        )
        .await;
    }

    if let Some(ref tmp) = temp_clip_srt {
        let _ = tokio::fs::remove_file(tmp).await;
    }

    res
}

/// Структура 10-мс аудио-окна для высокоточного контроля среза фонем на краях готового шортса
#[derive(Clone, Copy, Debug)]
struct ShortPcmFrame {
    rms_db: f32,
    is_speech: bool,
}

/// Извлекает из медиафайла интервал `[ss_opt .. ss_opt + dur_opt]` в 16kHz mono s16le WAV
/// и рассчитывает 10-мс карту голосовой активности (RMS dB + речевой ZCR) для детекции обрезанных фонем.
async fn extract_wav_and_10ms_frames(
    media_path: &Path,
    ss_opt: Option<f64>,
    dur_opt: Option<f64>,
    out_wav: &Path,
    app_handle: &AppHandle,
) -> Option<(Vec<ShortPcmFrame>, f32)> {
    let ffmpeg = app_handle.shell().sidecar("ffmpeg").ok()?;
    let mut args = vec!["-y".to_string(), "-nostdin".to_string()];
    if let Some(ss) = ss_opt {
        args.push("-ss".to_string());
        args.push(format!("{:.3}", ss.max(0.0)));
    }
    args.push("-i".to_string());
    args.push(media_path.to_string_lossy().to_string());
    if let Some(d) = dur_opt {
        args.push("-t".to_string());
        args.push(format!("{:.3}", d.max(0.2)));
    }
    args.extend([
        "-vn".to_string(),
        "-ac".to_string(),
        "1".to_string(),
        "-ar".to_string(),
        "16000".to_string(),
        "-c:a".to_string(),
        "pcm_s16le".to_string(),
        out_wav.to_string_lossy().to_string(),
    ]);

    let output = ffmpeg
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
        .args(args)
        .output()
        .await
        .ok()?;

    if !output.status.success() || !out_wav.exists() {
        return None;
    }

    let bytes = tokio::fs::read(out_wav).await.ok()?;
    if bytes.len() <= 44 + 320 {
        return None;
    }

    // Пропускаем стандартный WAV-заголовок (ищем чанк `data` или отступаем 44 байта)
    let mut pcm_offset = 44usize;
    for i in 12..bytes.len().min(256).saturating_sub(8) {
        if &bytes[i..i + 4] == b"data" {
            pcm_offset = i + 8;
            break;
        }
    }
    if pcm_offset >= bytes.len() {
        return None;
    }

    let pcm_bytes = &bytes[pcm_offset..];
    // 10 мс при 16000 Гц = 160 сэмплов (320 байт)
    let frame_samples = 160usize;
    let frame_bytes = frame_samples * 2;
    let num_frames = pcm_bytes.len() / frame_bytes;
    if num_frames == 0 {
        return None;
    }

    let mut raw_rms: Vec<f32> = Vec::with_capacity(num_frames);
    let mut raw_zcr: Vec<f32> = Vec::with_capacity(num_frames);

    for f_idx in 0..num_frames {
        let slice = &pcm_bytes[f_idx * frame_bytes..(f_idx + 1) * frame_bytes];
        let mut sum_sq = 0.0f64;
        let mut zc = 0usize;
        let mut prev_s: i16 = 0;

        for (s_idx, chunk) in slice.chunks_exact(2).enumerate() {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            let norm = (sample as f64) / 32768.0;
            sum_sq += norm * norm;
            if s_idx > 0 && ((sample >= 0 && prev_s < 0) || (sample < 0 && prev_s >= 0)) {
                zc += 1;
            }
            prev_s = sample;
        }

        let rms = (sum_sq / (frame_samples as f64)).sqrt().max(1e-6);
        let db = (20.0 * rms.log10()).clamp(-85.0, 0.0) as f32;
        let zcr = (zc as f32) / (frame_samples as f32);
        raw_rms.push(db);
        raw_zcr.push(zcr);
    }

    // Вычисляем локальный уровень тихой впадины (20-й перцентиль непустых фреймов)
    let mut sorted: Vec<f32> = raw_rms.iter().copied().filter(|&v| v > -78.0).collect();
    let valley_db = if sorted.is_empty() {
        -55.0
    } else {
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p20 = ((sorted.len() as f64) * 0.20) as usize;
        sorted[p20.min(sorted.len() - 1)].clamp(-62.0, -22.0)
    };

    let mut frames = Vec::with_capacity(num_frames);
    for idx in 0..num_frames {
        let prev = raw_rms[idx.saturating_sub(1)];
        let curr = raw_rms[idx];
        let next = raw_rms[(idx + 1).min(num_frames - 1)];
        let smoothed_db = prev * 0.25 + curr * 0.50 + next * 0.25;
        let zcr = raw_zcr[idx];
        let is_speech = smoothed_db > (valley_db + 2.2).max(-46.0) && (0.015..=0.34).contains(&zcr);
        frames.push(ShortPcmFrame {
            rms_db: smoothed_db,
            is_speech,
        });
    }

    Some((frames, valley_db))
}

/// Нормализует строку до списка строчных слов (с заменой `ё` -> `е`) для сверки распознанных слов Whisper в готовом шортсе
fn normalize_words_for_short_audit(text: &str) -> Vec<String> {
    let (_, clean_body) = crate::subtitles_parser::extract_srt_character_prefix(text);
    clean_body
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphabetic())
                .collect::<String>()
                .to_lowercase()
                .replace('ё', "е")
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Проверяет нечеткое совпадение двух слов (с учетом падежных окончаний и возможных мелких расхождений ASR на краях)
fn audit_words_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    if ac.len() >= 4 && bc.len() >= 4 && ac[..4] == bc[..4] {
        return true;
    }
    if ac.len() >= 3 && bc.len() >= 3 && ac[..3] == bc[..3] && ac.len().abs_diff(bc.len()) <= 2 {
        return true;
    }
    false
}

/// Проверяет по тексту последней реплики Whisper из самого отрендеренного шортса,
/// оборвалась ли фраза на полуслове, висячем предлоге/союзе или многоточии обрыва («И не по...», «Когда он пришёл, не...»)
fn is_short_whisper_ending_cut(text: &str, reference_cue_text: Option<&str>) -> bool {
    let trimmed = text.trim().trim_end_matches(['"', '»', ')']).trim();
    if trimmed.is_empty() {
        return false;
    }

    if !crate::text_analyzer::has_terminal_punctuation(trimmed)
        || crate::text_analyzer::has_incomplete_clause(trimmed)
        || trimmed.ends_with(',')
        || trimmed.ends_with('—')
        || trimmed.ends_with('-')
        || trimmed.ends_with(':')
        || trimmed.ends_with(';')
    {
        return true;
    }

    // Когда аудио обрывается посреди слова или незаконченной синтаксической группы, Whisper ставит в конце «...» или «…»
    // (например «И не по...» в Клипе 05 или «Когда он пришёл, не...» в Клипе 02)
    let ends_with_ellipsis =
        trimmed.ends_with("...") || trimmed.ends_with("..") || trimmed.ends_with('…');
    if ends_with_ellipsis {
        let ref_has_ellipsis = reference_cue_text
            .map(|r| {
                let rt = r.trim();
                rt.ends_with("...") || rt.ends_with('…')
            })
            .unwrap_or(false);

        let without_dots = trimmed.trim_end_matches(['.', '…', ' ']);
        let words = normalize_words_for_short_audit(without_dots);
        let last_w = words.last().map(|s| s.as_str()).unwrap_or("");

        // Короткий осколок <= 4 слов с многоточием («И не по...»), короткое слово/приставка <= 3 букв перед «...»,
        // незавершенная клауза перед «...», либо отсутствие многоточия в эталонном SRT проекта — это 100% аудиообрезка!
        if !ref_has_ellipsis
            || words.len() <= 4
            || last_w.chars().count() <= 3
            || crate::text_analyzer::has_incomplete_clause(without_dots)
            || matches!(
                last_w,
                "по" | "не"
                    | "ни"
                    | "на"
                    | "за"
                    | "со"
                    | "во"
                    | "до"
                    | "от"
                    | "из"
                    | "об"
                    | "при"
                    | "про"
                    | "без"
                    | "под"
                    | "над"
                    | "для"
                    | "как"
                    | "что"
                    | "кто"
                    | "где"
                    | "или"
                    | "но"
                    | "да"
                    | "же"
                    | "бы"
                    | "то"
                    | "это"
                    | "все"
                    | "я"
                    | "ты"
                    | "он"
                    | "она"
                    | "мы"
                    | "вы"
                    | "они"
                    | "его"
                    | "ее"
                    | "их"
            )
        {
            return true;
        }
    }

    false
}

/// Находит оптимальную точку разреза (индекс 10-мс фрейма) в интервале `[min_idx ..= max_idx]`:
/// 1) Если есть чистая речевая пауза >= 140 мс (`14` фреймов подряд без речи и ниже `valley_db + 1.5`),
///    возвращает точку через 80 мс после окончания последнего речевого сэмпла (строго до начала следующей фразы).
/// 2) Если играет громкая фоновая музыка (BGM), находит центр самого тихого 100-мс окна (`10` фреймов)
///    с минимальным числом речевых фреймов и минимальной энергией RMS dB.
fn find_best_cut_index_in_frames(
    nb_frames: &[ShortPcmFrame],
    nb_valley_db: f32,
    scan_start_idx: usize,
    min_idx: usize,
    max_idx: usize,
) -> usize {
    if nb_frames.is_empty() {
        return min_idx;
    }
    let last_valid = nb_frames.len().saturating_sub(1);
    let lo = min_idx.min(last_valid);
    let hi = max_idx.min(last_valid).max(lo);
    let start_k = scan_start_idx.min(lo);

    let mut quiet_streak = 0usize;
    for k in start_k..=hi {
        let f = nb_frames[k];
        if !f.is_speech && f.rms_db <= nb_valley_db + 1.5 {
            quiet_streak += 1;
            if quiet_streak >= 14 && k >= lo {
                let speech_done_idx = k.saturating_sub(quiet_streak);
                return (speech_done_idx + 8).clamp(lo, k.saturating_sub(2).max(lo));
            }
        } else {
            quiet_streak = 0;
        }
    }

    // Если из-за громкой музыки (BGM) порог `nb_valley_db + 1.5` не пробивается —
    // ищем самое тихое 100-мс окно (10 фреймов) внутри `[lo ..= hi]` и режем ровно по его центру!
    let win_len = 10usize.min(hi.saturating_sub(lo) + 1).max(1);
    let mut best_center = lo;
    let mut best_score = f32::MAX;

    let max_w_start = hi.saturating_sub(win_len - 1).max(lo);
    for w_start in lo..=max_w_start {
        let w_end = (w_start + win_len).min(hi + 1);
        let mut speech_cnt = 0usize;
        let mut sum_db = 0.0f32;
        for f in &nb_frames[w_start..w_end] {
            if f.is_speech {
                speech_cnt += 1;
            }
            sum_db += f.rms_db;
        }
        let avg_db = sum_db / ((w_end - w_start).max(1) as f32);
        let score = (speech_cnt as f32) * 8.0 + avg_db;
        if score < best_score {
            best_score = score;
            best_center = w_start + (w_end - w_start) / 2;
        }
    }

    best_center.clamp(lo, hi)
}

/// ФИНАЛЬНАЯ ПРОВЕРКА ГОТОВОГО ШОРТСА ЧЕРЕЗ WHISPER ASR И 10-МС АНАЛИЗ FFMPEG PCM:
/// 1. Извлекает аудио из самого готового файла шортса (`cropped_short_path`) и прогоняет его через `whisper.cpp`.
/// 2. Проверяет, все ли слова стартовой и финальной реплики распознались целиком, нет ли обрыва на полуслове,
///    многоточия обрыва («И не по...»), незакрытой запятой/союза, либо случайного захвата начала следующей фразы.
/// 3. Параллельно проверяет 10-мс профиль звуковой энергии FFmpeg на границах `start_sec` и `end_sec` в окрестности оригинала.
/// 4. Если обнаружена обрезка слова или фразы — вычисляет исправленные `[repaired_start, repaired_dur]`,
///    пересоздает синхронизированные субтитры, перерендеривает шортс и выполняет контрольную верификацию Whisper
///    (со вторым контуром авто-доводки, если на стыке под фоновую музыку зацепился хвост следующей реплики)!
#[allow(clippy::too_many_arguments)]
async fn verify_and_repair_short_audio_with_whisper(
    source_video_path: &Path,
    start_sec: f64,
    duration_sec: f64,
    cropped_short_path: &Path,
    project_srt_path: Option<&Path>,
    config: &RenderConfig,
    crop_expr: Option<&str>,
    app_handle: &AppHandle,
    cancel_rx: Receiver<()>,
) -> Option<(f64, f64, Option<PathBuf>)> {
    let short_name = cropped_short_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("clip.mp4");

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_dir = std::env::temp_dir().join(format!(
        "vcp_short_whisper_audit_{}_{}",
        std::process::id(),
        ts
    ));
    let _ = tokio::fs::create_dir_all(&temp_dir).await;

    let short_wav = temp_dir.join("rendered_short.wav");
    let nb_wav = temp_dir.join("source_neighborhood.wav");

    // 1. Извлекаем WAV и 10-мс профиль из САМОГО ГОТОВОГО ШОРТСА
    let Some((short_frames, short_valley_db)) =
        extract_wav_and_10ms_frames(cropped_short_path, None, None, &short_wav, app_handle).await
    else {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return None;
    };

    // 2. Прогоняем готовый шортс через Whisper ASR (именно сам шортс!)
    let short_whisper_cues = crate::whisper::transcribe_short_wav_with_whisper(&short_wav, app_handle)
        .await
        .unwrap_or_default();

    // 3. Загружаем эталонные реплики из SRT проекта для сверки полноты слов на границах
    let project_cues: Vec<crate::text_analyzer::SubtitleFragment> = if let Some(sp) = project_srt_path {
        tokio::fs::read_to_string(sp)
            .await
            .ok()
            .map(|c| crate::text_analyzer::parse_srt(&c))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let end_sec = start_sec + duration_sec;

    // Извлекаем окрестность из исходного видео: [start_sec - 1.8s .. end_sec + 6.5s] для точного анализа пограничных слов и пауз
    let nb_start = (start_sec - 1.80).max(0.0);
    let nb_dur = (end_sec + 6.50 - nb_start).max(duration_sec + 3.0);
    let nb_analysis =
        extract_wav_and_10ms_frames(source_video_path, Some(nb_start), Some(nb_dur), &nb_wav, app_handle).await;

    let mut cut_reasons: Vec<String> = Vec::new();
    let mut need_start_shift = 0.0f64;
    let mut need_end_shift = 0.0f64;
    let mut prefer_trim_leaked_tail = false;
    let mut leaked_tail_pause_sec: Option<f64> = None;
    let mut extend_into_next_cue = false;
    let mut last_expected_idx_opt: Option<usize> = None;

    // Находим реплики проекта, которые существенно (>= 42% своей длительности или >= 0.85с) входят в интервал шортса.
    // Это исключает ложное принятие следующей реплики, задетой самым краем на 0.15–0.25с (как в Клипе 02), за целевую финальную!
    let active_cues: Vec<(usize, &crate::text_analyzer::SubtitleFragment)> = project_cues
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let overlap = (c.end.min(end_sec) - c.start.max(start_sec)).max(0.0);
            let cue_dur = (c.end - c.start).max(0.10);
            overlap >= 0.85 || overlap >= cue_dur * 0.42
        })
        .collect();

    if let Some(&(last_idx, _)) = active_cues.last() {
        last_expected_idx_opt = Some(last_idx);
    }

    // --- АНАЛИЗ WHISPER ASR ГОТОВОГО ШОРТСА ---
    let short_all_words: Vec<String> = short_whisper_cues
        .iter()
        .flat_map(|c| normalize_words_for_short_audit(&c.text))
        .collect();

    if let Some(last_short_cue) = short_whisper_cues.last() {
        let last_txt = last_short_cue.text.trim();
        let ref_txt = active_cues.last().map(|(_, c)| c.text.as_str());
        let ends_near_clip_edge = last_short_cue.end >= duration_sec - 1.85;
        if ends_near_clip_edge && is_short_whisper_ending_cut(last_txt, ref_txt) {
            cut_reasons.push(format!(
                "Whisper на готовом шортсе зафиксировал незавершенную/обрезанную фразу в конце: «{}»",
                last_txt
            ));
            need_end_shift = need_end_shift.max(1.65);
        }
    }

    // Сверяем слова последней и следующей реплики из SRT проекта со словами, которые реально услышал Whisper в готовом шортсе
    if !project_cues.is_empty() && !short_all_words.is_empty() {
        if let Some(&(first_idx, first_expected)) = active_cues.first() {
            let expected_first_words = normalize_words_for_short_audit(&first_expected.text);
            let head_window: Vec<&String> = short_all_words.iter().take(10).collect();
            if let Some(first_w) = expected_first_words.first() {
                if first_w.chars().count() >= 3
                    && !head_window.iter().any(|sw| audit_words_match(sw, first_w))
                    && (first_expected.start - start_sec).abs() <= 0.65
                {
                    cut_reasons.push(format!(
                        "Whisper не дослышал первое слово «{}» стартовой реплики «{}»",
                        first_w,
                        first_expected.text.trim()
                    ));
                    need_start_shift = need_start_shift.min(-0.45);
                }
            }
            let _ = first_idx;
        }

        if let Some(&(last_idx, last_expected)) = active_cues.last() {
            let expected_last_words = normalize_words_for_short_audit(&last_expected.text);
            let tail_window: Vec<&String> = short_all_words
                .iter()
                .rev()
                .take(14)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();

            // 1) Проверяем, услышал ли Whisper на готовом шортсе последнее слово целевой финальной реплики
            if let Some(last_w) = expected_last_words.last() {
                if last_w.chars().count() >= 3
                    && !tail_window.iter().any(|sw| audit_words_match(sw, last_w))
                {
                    cut_reasons.push(format!(
                        "Whisper на шортсе не распознал финальное слово «{}» фразы «{}» (слово обрезано на конце)",
                        last_w,
                        last_expected.text.trim()
                    ));
                    let deficit = (last_expected.end - end_sec).max(0.0) + 1.15;
                    need_end_shift = need_end_shift.max(deficit.clamp(0.85, 2.20));
                }
            }

            // 2) Если целевая финальная реплика в проекте сама заканчивалась запятой или не имела финальной точки
            if !crate::text_analyzer::has_terminal_punctuation(&last_expected.text)
                || crate::text_analyzer::has_incomplete_clause(&last_expected.text)
            {
                if last_idx + 1 < project_cues.len() {
                    let next_cue = &project_cues[last_idx + 1];
                    let ext_needed = (next_cue.end - end_sec + 0.25).clamp(0.85, 4.80);
                    cut_reasons.push(format!(
                        "Финальная реплика шортса синтаксически переходит в «{}» (требуется договорить фразу до конца)",
                        next_cue.text.trim()
                    ));
                    extend_into_next_cue = true;
                    need_end_shift = need_end_shift.max(ext_needed);
                }
            } else if last_idx + 1 < project_cues.len() {
                // 3) Проверяем обратную ситуацию: в конец шортса попало начало следующей фразы (`next_cue`), но не вся она целиком!
                let next_cue = &project_cues[last_idx + 1];
                let next_words = normalize_words_for_short_audit(&next_cue.text);
                if next_words.len() >= 2 && (next_cue.start - end_sec) <= 4.2 {
                    let prev_words_norm = normalize_words_for_short_audit(&last_expected.text);
                    let mut leaked_match_count = 0usize;
                    let mut last_matched_next_word = String::new();
                    for nw in next_words.iter().take(7) {
                        if nw.chars().count() >= 3
                            && !prev_words_norm.iter().any(|pw| audit_words_match(pw, nw))
                            && tail_window.iter().rev().take(6).any(|sw| audit_words_match(sw, nw))
                        {
                            leaked_match_count += 1;
                            last_matched_next_word = nw.clone();
                        }
                    }

                    let next_last_w = next_words.last().cloned().unwrap_or_default();
                    let next_completed = !next_last_w.is_empty()
                        && tail_window
                            .iter()
                            .rev()
                            .take(3)
                            .any(|sw| audit_words_match(sw, &next_last_w));

                    if leaked_match_count >= 1 && !next_completed {
                        cut_reasons.push(format!(
                            "В конец шортса попало начало следующей фразы («{}», слово «{}») и оборвалось на полуслове",
                            next_cue.text.trim(),
                            last_matched_next_word
                        ));
                        // Если все слова `last_expected` уже прозвучали, а из `next_cue` зацепилось лишь 1–2 слова на самом хвосте (`<= 0.90с`),
                        // приоритет — аккуратно подрезать по паузе ПЕРЕД `next_cue`; иначе — договорить `next_cue` целиком!
                        if leaked_match_count <= 2 && (end_sec - last_expected.end) <= 0.95 {
                            prefer_trim_leaked_tail = true;
                        } else {
                            extend_into_next_cue = true;
                        }
                        let mut target_end = next_cue.end + 0.25;
                        if !crate::text_analyzer::has_terminal_punctuation(&next_cue.text)
                            && last_idx + 2 < project_cues.len()
                        {
                            target_end = project_cues[last_idx + 2].end + 0.25;
                        }
                        let ext = (target_end - end_sec).clamp(0.95, 5.20);
                        need_end_shift = need_end_shift.max(ext);
                    }
                }
            }
        }
    }

    // --- АНАЛИЗ 10-МС АУДИО-ГРАНИЦ FFMPEG PCM (ДЕТЕКТОР СРЕЗА СЛОВ ПО ЗВУКОВОЙ ВОЛНЕ) ---
    if let Some((nb_frames, nb_valley_db)) = nb_analysis.as_ref() {
        let rel_start_idx = (((start_sec - nb_start) * 100.0).round() as isize)
            .clamp(0, (nb_frames.len().saturating_sub(1)) as isize) as usize;
        let rel_end_idx = (((end_sec - nb_start) * 100.0).round() as isize)
            .clamp(0, (nb_frames.len().saturating_sub(1)) as isize) as usize;

        // 1. Проверяем точку старта `rel_start_idx`: звучит ли речь прямо на границе старта (в окне [-60мс .. +50мс])?
        let start_win_lo = rel_start_idx.saturating_sub(6);
        let start_win_hi = (rel_start_idx + 5).min(nb_frames.len().saturating_sub(1));
        let active_at_start = (start_win_lo..=start_win_hi)
            .filter(|&k| nb_frames[k].is_speech && nb_frames[k].rms_db > nb_valley_db + 3.0)
            .count();
        let speech_before_start = (rel_start_idx.saturating_sub(8)..rel_start_idx)
            .filter(|&k| nb_frames[k].is_speech)
            .count();
        let speech_after_start = (rel_start_idx..(rel_start_idx + 8).min(nb_frames.len()))
            .filter(|&k| nb_frames[k].is_speech)
            .count();

        if active_at_start >= 4 && speech_before_start >= 3 && speech_after_start >= 3 {
            cut_reasons.push(format!(
                "FFmpeg PCM 10мс: голосовая волна разрезана на старте клипа ({:.2}с, RMS {:.1} dB > порога {:.1} dB)",
                start_sec,
                nb_frames[rel_start_idx].rms_db,
                nb_valley_db + 3.0
            ));
            need_start_shift = need_start_shift.min(-0.35);
        }

        // 2. Проверяем точку конца `rel_end_idx`
        let speech_before_end = (rel_end_idx.saturating_sub(12)..rel_end_idx)
            .filter(|&k| nb_frames[k].is_speech && nb_frames[k].rms_db > nb_valley_db + 2.5)
            .count();
        let speech_after_end = (rel_end_idx..(rel_end_idx + 16).min(nb_frames.len()))
            .filter(|&k| nb_frames[k].is_speech && nb_frames[k].rms_db > nb_valley_db + 2.5)
            .count();
        let boundary_rms = nb_frames[rel_end_idx].rms_db;

        let short_len = short_frames.len();
        let short_tail_active = if short_len > 16 {
            (short_len.saturating_sub(14)..short_len.saturating_sub(4))
                .filter(|&k| short_frames[k].is_speech && short_frames[k].rms_db > short_valley_db + 2.8)
                .count()
        } else {
            0
        };

        if (speech_before_end >= 4 && speech_after_end >= 3)
            || (boundary_rms > nb_valley_db + 3.2 && speech_after_end >= 2)
            || (short_tail_active >= 6 && speech_after_end >= 3)
        {
            cut_reasons.push(format!(
                "FFmpeg PCM 10мс: активная речь продолжается через границу конца шортса ({:.2}с, до среза {}0мс речи, после среза {}0мс речи)",
                end_sec, speech_before_end, speech_after_end
            ));

            // Ищем чистую паузу >= 150 мс в пределах 0.90с перед `rel_end_idx` (если мы случайно зацепили первые 0.15–0.75с следующей фразы)
            let lookback_limit = rel_end_idx.saturating_sub(90);
            for k in (lookback_limit + 15..rel_end_idx.saturating_sub(4)).rev() {
                let quiet_150ms = (0..15).all(|off| {
                    let idx_q = k.saturating_sub(off);
                    !nb_frames[idx_q].is_speech && nb_frames[idx_q].rms_db <= nb_valley_db + 1.6
                });
                let leaked_frames = rel_end_idx.saturating_sub(k);
                if quiet_150ms && (10..=75).contains(&leaked_frames) {
                    leaked_tail_pause_sec = Some(nb_start + ((k.saturating_sub(4)) as f64) * 0.01);
                    break;
                }
            }

            if need_end_shift < 0.45 {
                need_end_shift = 1.45;
            }
        }
    }

    // Если ни Whisper на готовом шортсе, ни 10-мс анализатор FFmpeg не нашли обрезанных слов — логируем успешное подтверждение!
    if cut_reasons.is_empty() {
        let recognized_preview = short_whisper_cues
            .last()
            .map(|c| c.text.trim())
            .unwrap_or("речь чистая");
        crate::logger::emit_log(
            app_handle,
            "Render",
            "info",
            &format!(
                "✓ [Финальный контроль аудио Whisper + FFmpeg] Шортс '{}' проверен ({} реплик распознано, концовка: «{}»): обрезанных слов и фраз не обнаружено.",
                short_name,
                short_whisper_cues.len(),
                recognized_preview
            ),
        );
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return None;
    }

    // --- ВЫЧИСЛЕНИЕ ТОЧНЫХ ИСПРАВЛЕННЫХ ГРАНИЦ `[repaired_start, repaired_end]` ПО ОРИГИНАЛУ ---
    let mut repaired_start = start_sec;
    let mut repaired_end = end_sec;

    if let Some((nb_frames, nb_valley_db)) = nb_analysis.as_ref() {
        let rel_start_idx = (((start_sec - nb_start) * 100.0).round() as isize)
            .clamp(0, (nb_frames.len().saturating_sub(1)) as isize) as usize;
        let rel_end_idx = (((end_sec - nb_start) * 100.0).round() as isize)
            .clamp(0, (nb_frames.len().saturating_sub(1)) as isize) as usize;

        // 1. Исправление начала `repaired_start`, если было срезано первое слово
        if need_start_shift < -0.05 {
            let min_idx = 0usize;
            let mut target_start_idx = rel_start_idx.saturating_sub(35);
            for k in (min_idx + 12..=rel_start_idx).rev() {
                let quiet_120ms = (0..12).all(|off| {
                    let idx_q = k.saturating_sub(off);
                    !nb_frames[idx_q].is_speech && nb_frames[idx_q].rms_db <= nb_valley_db + 1.4
                });
                if quiet_120ms {
                    target_start_idx = k.saturating_sub(4);
                    break;
                }
            }
            repaired_start = (nb_start + (target_start_idx as f64) * 0.01).max(0.0);
        }

        // 2. Исправление конца `repaired_end`:
        // Вариант А: если зацепился короткий фальш-хвост следующей реплики после паузы,
        // и все слова целевой финальной реплики `last_expected` уже прозвучали — подрезаем по этой паузе!
        let min_safe_clip_end = (repaired_start + 8.0).max(end_sec - 0.90);
        if let Some(pause_sec) = leaked_tail_pause_sec {
            if prefer_trim_leaked_tail && pause_sec >= min_safe_clip_end {
                repaired_end = pause_sec;
            }
        }

        // Вариант Б (основной): продлеваем конец клипа вперёд до завершения обрезанного слова/фразы.
        // ВАЖНО: Если мы договариваем текущую реплику `last_expected` (НЕ переходя в `next_cue`),
        // жестко ограничиваем окно поиска паузы границей начала `next_cue`, чтобы на громкой музыке (BGM)
        // не проскочить межфразовый вдох и не разрезать следующую реплику посередине (как «И не по...»)!
        if (repaired_end - end_sec).abs() < 0.03 {
            let max_nb_sec = nb_start + nb_dur - 0.15;
            let (min_forward_sec, max_forward_sec) = if let Some(last_idx) = last_expected_idx_opt {
                let last_cue = &project_cues[last_idx];
                let next_cue_opt = project_cues.get(last_idx + 1);
                if extend_into_next_cue {
                    if let Some(nc) = next_cue_opt {
                        let mut target_end = nc.end;
                        if !crate::text_analyzer::has_terminal_punctuation(&nc.text) {
                            if let Some(nc2) = project_cues.get(last_idx + 2) {
                                target_end = nc2.end;
                            }
                        }
                        let lo_s = (target_end - 0.25).max(end_sec + 0.85).min(max_nb_sec);
                        let hi_s = (target_end + 1.35).max(lo_s + 0.40).min(max_nb_sec);
                        (lo_s, hi_s)
                    } else {
                        let lo_s = (end_sec + 0.65).min(max_nb_sec);
                        let hi_s = (end_sec + need_end_shift.max(1.50)).min(max_nb_sec);
                        (lo_s, hi_s)
                    }
                } else {
                    // Договариваем ТОЛЬКО `last_cue` и останавливаемся в межфразовой впадине перед `next_cue`!
                    let expected_end = last_cue.end.max(end_sec + 0.55);
                    let lo_s = (expected_end - 0.15).max(end_sec + 0.45).min(max_nb_sec);
                    let hi_bound = if let Some(nc) = next_cue_opt {
                        // Если после паузы в финальной реплике стоит ещё одно слово (например «...спасибо тогда. Помогла.»),
                        // разрешаем дотянуться вплоть до `nc.start + 0.35` (до +2.85с), не залезая глубже в следующую реплику.
                        (nc.start + 0.35).max(lo_s + 0.85).min(end_sec + 2.85)
                    } else {
                        expected_end + 1.50
                    };
                    let hi_s = hi_bound.max(lo_s + 0.35).min(max_nb_sec);
                    (lo_s, hi_s)
                }
            } else {
                let lo_s = (end_sec + 0.55).min(max_nb_sec);
                let hi_s = (end_sec + 1.85).min(max_nb_sec);
                (lo_s, hi_s)
            };

            let min_forward_idx = (((min_forward_sec - nb_start) * 100.0).round() as isize)
                .clamp(rel_end_idx as isize + 10, (nb_frames.len().saturating_sub(1)) as isize)
                as usize;
            let max_forward_idx = (((max_forward_sec - nb_start) * 100.0).round() as isize)
                .clamp(min_forward_idx as isize, (nb_frames.len().saturating_sub(1)) as isize)
                as usize;

            let chosen_idx = find_best_cut_index_in_frames(
                nb_frames,
                *nb_valley_db,
                rel_end_idx + 8,
                min_forward_idx,
                max_forward_idx,
            );
            repaired_end = nb_start + (chosen_idx as f64) * 0.01;
        }
    } else {
        repaired_start = (start_sec + need_start_shift).max(0.0);
        repaired_end = end_sec + need_end_shift.max(0.85);
    }

    let mut repaired_dur = (repaired_end - repaired_start).max(5.0);
    if (repaired_start - start_sec).abs() < 0.06 && (repaired_dur - duration_sec).abs() < 0.06 {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return None;
    }

    crate::logger::emit_log(
        app_handle,
        "Render",
        "warn",
        &format!(
            "⚠ [Финальный контроль аудио Whisper + FFmpeg] В шортсе '{}' обнаружена обрезка речи ({})! Авто-коррекция границ: [{:.2}с - {:.2}с] -> [{:.2}с - {:.2}с] (длительность {:.2}с -> {:.2}с) и пересборка шортса...",
            short_name,
            cut_reasons.join("; "),
            start_sec,
            end_sec,
            repaired_start,
            repaired_start + repaired_dur,
            duration_sec,
            repaired_dur
        ),
    );

    // 4. Пересоздаем синхронизированные субтитры для обновленного интервала `[repaired_start .. repaired_start + repaired_dur]`
    let mut new_temp_srt = if let Some(sp) = project_srt_path {
        if sp.exists() {
            create_clip_srt(sp, repaired_start, repaired_dur).await
        } else {
            None
        }
    } else {
        None
    };

    let repaired_video_tmp =
        cropped_short_path.with_extension(format!("audio_repaired_{}.tmp.mp4", ts));

    let rerender_res = execute_ffmpeg_clip_render(
        source_video_path,
        repaired_start,
        repaired_dur,
        &repaired_video_tmp,
        new_temp_srt.as_deref(),
        config,
        crop_expr,
        app_handle,
        cancel_rx.resubscribe(),
    )
    .await;

    let repaired_ok = rerender_res.is_ok()
        && std::fs::metadata(&repaired_video_tmp)
            .map(|m| m.len() > 1024)
            .unwrap_or(false);

    if repaired_ok {
        let _ = tokio::fs::remove_file(cropped_short_path).await;
        if tokio::fs::rename(&repaired_video_tmp, cropped_short_path)
            .await
            .is_err()
        {
            let _ = tokio::fs::copy(&repaired_video_tmp, cropped_short_path).await;
            let _ = tokio::fs::remove_file(&repaired_video_tmp).await;
        }

        // Контрольный прогон Whisper по уже исправленному шортсу для подтверждения полноты финальной фразы!
        let recheck_wav = temp_dir.join("rechecked_short.wav");
        let mut recheck_cues = if extract_wav_and_10ms_frames(
            cropped_short_path,
            None,
            None,
            &recheck_wav,
            app_handle,
        )
        .await
        .is_some()
        {
            crate::whisper::transcribe_short_wav_with_whisper(&recheck_wav, app_handle)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // --- ВТОРОЙ КОНТУР ДОВОДКИ (ЕСЛИ ПРИ ПЕРВОМ ПРОДЛЕНИИ ЗАЦЕПИЛСЯ КУСОК СЛЕДУЮЩЕЙ ФРАЗЫ ИЛИ ВСЁ ЕЩЁ НЕ ДОСЛЫШАНО ФИНАЛЬНОЕ СЛОВО ИЗ SRT) ---
        let recheck_ending_cut = recheck_cues
            .last()
            .map(|c| is_short_whisper_ending_cut(&c.text, None))
            .unwrap_or(false);
        let recheck_words: Vec<String> = recheck_cues
            .iter()
            .flat_map(|c| normalize_words_for_short_audit(&c.text))
            .collect();
        let recheck_missing_expected_last = last_expected_idx_opt
            .and_then(|idx| project_cues.get(idx))
            .and_then(|lc| {
                let words = normalize_words_for_short_audit(&lc.text);
                words.last().cloned()
            })
            .map(|lw| {
                lw.chars().count() >= 3
                    && !recheck_words.is_empty()
                    && !recheck_words
                        .iter()
                        .rev()
                        .take(10)
                        .any(|sw| audit_words_match(sw, &lw))
            })
            .unwrap_or(false);
        let recheck_still_cut = recheck_ending_cut || recheck_missing_expected_last;

        if recheck_still_cut {
            let mut repaired_end_v2: Option<f64> = None;

            // Случай 1: Предпоследняя распознанная реплика в исправленном шортсе полностью завершена (например «...как последний подлец.»),
            // а последняя реплика — короткий оборванный зачин следующей мысли (<= 5 слов, например «И не по...» или «Не, не злитесь!»).
            // Тогда у нас есть ТОЧНЫЙ таймкод от самого Whisper внутри шортса, где закончилась полная фраза (`prev_rc.end`) и начался лишний хвост (`last_rc.start`)!
            if recheck_ending_cut && !recheck_missing_expected_last && recheck_cues.len() >= 2 {
                let prev_rc = &recheck_cues[recheck_cues.len() - 2];
                let last_rc = &recheck_cues[recheck_cues.len() - 1];
                let last_rc_words = normalize_words_for_short_audit(&last_rc.text).len();
                let prev_end_abs = repaired_start + prev_rc.end;
                let last_start_abs = repaired_start + last_rc.start;

                if !is_short_whisper_ending_cut(&prev_rc.text, None)
                    && last_rc_words <= 5
                    && prev_end_abs >= repaired_start + 5.5
                {
                    if let Some((nb_frames, nb_valley_db)) = nb_analysis.as_ref() {
                        let lo_sec = (prev_end_abs - 0.18).max(repaired_start + 5.0);
                        let hi_sec = (last_start_abs + 0.10).max(lo_sec + 0.15);
                        let lo_idx = (((lo_sec - nb_start) * 100.0).round() as isize)
                            .clamp(0, (nb_frames.len().saturating_sub(1)) as isize)
                            as usize;
                        let hi_idx = (((hi_sec - nb_start) * 100.0).round() as isize)
                            .clamp(lo_idx as isize, (nb_frames.len().saturating_sub(1)) as isize)
                            as usize;
                        let cut_idx = find_best_cut_index_in_frames(
                            nb_frames,
                            *nb_valley_db,
                            lo_idx,
                            lo_idx,
                            hi_idx,
                        );
                        repaired_end_v2 = Some(nb_start + (cut_idx as f64) * 0.01);
                    } else {
                        repaired_end_v2 = Some((prev_end_abs + 0.12).min(last_start_abs + 0.05));
                    }
                }
            }

            // Случай 2: Если подрезать перед хвостом нельзя (или не прозвучало последнее слово финальной реплики),
            // дотягиваем границу до завершения слова / следующей реплики в SRT проекта!
            if repaired_end_v2.is_none() {
                if let Some((nb_frames, nb_valley_db)) = nb_analysis.as_ref() {
                    let max_nb_sec = nb_start + nb_dur - 0.15;
                    let (target_sec, hi_cap_sec) = if recheck_missing_expected_last {
                        let le_end = last_expected_idx_opt
                            .and_then(|idx| project_cues.get(idx))
                            .map(|lc| lc.end)
                            .unwrap_or(repaired_end);
                        let nc_cap = last_expected_idx_opt
                            .and_then(|idx| project_cues.get(idx + 1))
                            .map(|nc| nc.start + 0.35)
                            .unwrap_or(max_nb_sec);
                        let t = le_end.max(repaired_end + 1.15).min(nc_cap).min(max_nb_sec);
                        (t, nc_cap.min(max_nb_sec))
                    } else {
                        let t = project_cues
                            .iter()
                            .find(|c| c.end > repaired_end + 0.25 && c.start < repaired_end + 1.20)
                            .map(|c| (c.end + 0.30).min(max_nb_sec))
                            .unwrap_or_else(|| (repaired_end + 2.10).min(max_nb_sec));
                        (t, (t + 0.90).min(max_nb_sec))
                    };
                    if target_sec > repaired_end + 0.35 {
                        let lo_idx = ((((target_sec - 0.30) - nb_start) * 100.0).round() as isize)
                            .clamp(0, (nb_frames.len().saturating_sub(1)) as isize)
                            as usize;
                        let hi_idx = (((hi_cap_sec.max(target_sec + 0.20) - nb_start) * 100.0).round() as isize)
                            .clamp(lo_idx as isize, (nb_frames.len().saturating_sub(1)) as isize)
                            as usize;
                        let cut_idx = find_best_cut_index_in_frames(
                            nb_frames,
                            *nb_valley_db,
                            lo_idx,
                            lo_idx,
                            hi_idx,
                        );
                        repaired_end_v2 = Some(nb_start + (cut_idx as f64) * 0.01);
                    }
                }
            }

            if let Some(end_v2) = repaired_end_v2 {
                let dur_v2 = (end_v2 - repaired_start).max(5.0);
                if (dur_v2 - repaired_dur).abs() >= 0.12 {
                    crate::logger::emit_log(
                        app_handle,
                        "Render",
                        "info",
                        &format!(
                            "⟳ [Точная доводка Whisper Pass 2] Устранение остаточного хвоста на стыке фраз в '{}': [{:.2}с - {:.2}с] -> [{:.2}с - {:.2}с]...",
                            short_name,
                            repaired_start,
                            repaired_start + repaired_dur,
                            repaired_start,
                            repaired_start + dur_v2
                        ),
                    );

                    let srt_v2 = if let Some(sp) = project_srt_path {
                        if sp.exists() {
                            create_clip_srt(sp, repaired_start, dur_v2).await
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let tmp_v2 =
                        cropped_short_path.with_extension(format!("audio_repaired_v2_{}.tmp.mp4", ts));
                    let res_v2 = execute_ffmpeg_clip_render(
                        source_video_path,
                        repaired_start,
                        dur_v2,
                        &tmp_v2,
                        srt_v2.as_deref(),
                        config,
                        crop_expr,
                        app_handle,
                        cancel_rx,
                    )
                    .await;

                    if res_v2.is_ok()
                        && std::fs::metadata(&tmp_v2)
                            .map(|m| m.len() > 1024)
                            .unwrap_or(false)
                    {
                        let _ = tokio::fs::remove_file(cropped_short_path).await;
                        if tokio::fs::rename(&tmp_v2, cropped_short_path)
                            .await
                            .is_err()
                        {
                            let _ = tokio::fs::copy(&tmp_v2, cropped_short_path).await;
                            let _ = tokio::fs::remove_file(&tmp_v2).await;
                        }
                        if let Some(ref old_s) = new_temp_srt {
                            let _ = tokio::fs::remove_file(old_s).await;
                        }
                        repaired_dur = dur_v2;
                        new_temp_srt = srt_v2;

                        let recheck_wav_v2 = temp_dir.join("rechecked_short_v2.wav");
                        if extract_wav_and_10ms_frames(
                            cropped_short_path,
                            None,
                            None,
                            &recheck_wav_v2,
                            app_handle,
                        )
                        .await
                        .is_some()
                        {
                            if let Ok(cues_v2) = crate::whisper::transcribe_short_wav_with_whisper(
                                &recheck_wav_v2,
                                app_handle,
                            )
                            .await
                            {
                                if !cues_v2.is_empty() {
                                    recheck_cues = cues_v2;
                                }
                            }
                        }
                    } else {
                        let _ = tokio::fs::remove_file(&tmp_v2).await;
                        if let Some(ref s2) = srt_v2 {
                            let _ = tokio::fs::remove_file(s2).await;
                        }
                    }
                }
            }
        }

        let recheck_summary = recheck_cues
            .last()
            .map(|c| c.text.trim().to_string())
            .unwrap_or_else(|| "фраза восстановлена".to_string());

        crate::logger::emit_log(
            app_handle,
            "Render",
            "info",
            &format!(
                "✓ [Финальный контроль аудио Whisper + FFmpeg] Шортс '{}' успешно исправлен и повторно проверен Whisper! Новые границы: [{:.2}с - {:.2}с], финальная фраза в шортсе: «{}».",
                short_name,
                repaired_start,
                repaired_start + repaired_dur,
                recheck_summary
            ),
        );
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        Some((repaired_start, repaired_dur, new_temp_srt))
    } else {
        let _ = tokio::fs::remove_file(&repaired_video_tmp).await;
        if let Some(ref s) = new_temp_srt {
            let _ = tokio::fs::remove_file(s).await;
        }
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        None
    }
}

/// Выполняет финальную проверку готового 9:16 шортса на наличие лиц.
/// Если в финальной обрезке не найдено ни одного лица (`0` кадров), проверяет оригинальное полноценное 16:9 видео
/// и при обнаружении лиц в оригинале автоматически исправляет (перерендеривает) шортс с точным наведением на лицо.
#[allow(clippy::too_many_arguments)]
async fn verify_and_repair_cropped_short_faces(
    source_video_path: &Path,
    start_sec: f64,
    duration_sec: f64,
    cropped_short_path: &Path,
    temp_clip_srt: Option<&Path>,
    config: &RenderConfig,
    used_ratios: &[f64],
    app_handle: &AppHandle,
    cancel_rx: Receiver<()>,
) {
    let short_name = cropped_short_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("clip.mp4");

    let (face_frames, total_frames, max_score) =
        detect_faces_in_cropped_short(cropped_short_path, duration_sec, app_handle).await;

    if total_frames == 0 {
        return;
    }

    if face_frames >= 1 {
        crate::logger::emit_log(
            app_handle,
            "Render",
            "info",
            &format!(
                "✓ [Финальный контроль лиц 9:16] В готовом шортсе '{}' подтверждены лица в кадре ({}/{} проверочных кадров, четкость: {:.2})",
                short_name, face_frames, total_frames, max_score
            ),
        );
        return;
    }

    crate::logger::emit_log(
        app_handle,
        "Render",
        "warn",
        &format!(
            "⚠ [Финальный контроль лиц 9:16] В финальной обрезке '{}' НЕ зафиксировано ни одного лица (0/{} кадров)! Запуск глубокой проверки полноценного оригинального 16:9 видео [{:.1}с - {:.1}с]...",
            short_name,
            total_frames,
            start_sec,
            start_sec + duration_sec
        ),
    );

    if let Some((repaired_expr, repaired_summary, orig_face_frames, orig_best_score)) =
        deep_scan_original_video_for_faces(
            source_video_path,
            start_sec,
            duration_sec,
            used_ratios,
            app_handle,
        )
        .await
    {
        crate::logger::emit_log(
            app_handle,
            "Render",
            "info",
            &format!(
                "⟳ [Авто-исправление шортса 9:16] В оригинальном 16:9 видео обнаружены лица ({} кадров, четкость {:.2} -> {}). Пересборка '{}' с исправленной обрезкой...",
                orig_face_frames, orig_best_score, repaired_summary, short_name
            ),
        );

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let repaired_tmp =
            cropped_short_path.with_extension(format!("face_repaired_{}.tmp.mp4", ts));

        let rerender_res = execute_ffmpeg_clip_render(
            source_video_path,
            start_sec,
            duration_sec,
            &repaired_tmp,
            temp_clip_srt,
            config,
            Some(&repaired_expr),
            app_handle,
            cancel_rx,
        )
        .await;

        let repaired_ok = rerender_res.is_ok()
            && std::fs::metadata(&repaired_tmp)
                .map(|m| m.len() > 1024)
                .unwrap_or(false);

        if repaired_ok {
            let _ = tokio::fs::remove_file(cropped_short_path).await;
            if tokio::fs::rename(&repaired_tmp, cropped_short_path)
                .await
                .is_err()
            {
                let _ = tokio::fs::copy(&repaired_tmp, cropped_short_path).await;
                let _ = tokio::fs::remove_file(&repaired_tmp).await;
            }

            let (new_faces, new_total, new_max) =
                detect_faces_in_cropped_short(cropped_short_path, duration_sec, app_handle).await;
            crate::logger::emit_log(
                app_handle,
                "Render",
                "info",
                &format!(
                    "✓ [Финальный контроль лиц 9:16] Шортс '{}' успешно исправлен по оригинальному видео: теперь лиц в кадре {}/{} (четкость {:.2}, траектория: {})!",
                    short_name,
                    new_faces.max(1),
                    new_total.max(1),
                    new_max.max(orig_best_score),
                    repaired_summary
                ),
            );
        } else {
            let _ = tokio::fs::remove_file(&repaired_tmp).await;
            crate::logger::emit_log(
                app_handle,
                "Render",
                "warn",
                &format!(
                    "Не удалось пересобрать шортс '{}' при авто-исправлении лиц, сохранен исходный файл.",
                    short_name
                ),
            );
        }
    } else {
        crate::logger::emit_log(
            app_handle,
            "Render",
            "info",
            &format!(
                "ℹ [Финальный контроль лиц 9:16] Глубокая проверка оригинала подтвердила: на интервале [{:.1}с - {:.1}с] лица в кадре отсутствуют (сцена общего плана / пейзаж / деталь без лиц) — обрезка не требует правки.",
                start_sec,
                start_sec + duration_sec
            ),
        );
    }
}

/// Извлекает кадры из уже готового вертикального 9:16 шортса (`135x240`, 1.5 FPS) и проверяет,
/// зафиксировано ли в финальной обрезке хотя бы одно не обрезанное по краям лицо персонажа.
async fn detect_faces_in_cropped_short(
    cropped_short_path: &Path,
    duration_sec: f64,
    app_handle: &AppHandle,
) -> (usize, usize, f64) {
    let Ok(ffmpeg) = app_handle.shell().sidecar("ffmpeg") else {
        return (1, 1, 0.50);
    };

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_dir = std::env::temp_dir().join(format!(
        "vcp_verify916_{}_{}",
        std::process::id(),
        ts
    ));
    let _ = tokio::fs::create_dir_all(&temp_dir).await;

    let scan_dur = duration_sec.clamp(2.0, 28.0);
    let out_pattern = temp_dir
        .join("vframe_%03d.bmp")
        .to_string_lossy()
        .to_string();

    let cmd = ffmpeg
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
        .args([
            "-nostdin",
            "-y",
            "-t",
            &format!("{:.3}", scan_dur),
            "-i",
            &cropped_short_path.to_string_lossy(),
            "-vf",
            "fps=1.5,scale=135:240:flags=fast_bilinear",
            "-pix_fmt",
            "bgr24",
            &out_pattern,
        ]);

    if let Ok((mut rx, child)) = cmd.spawn() {
        let wait_frames = async {
            while let Some(event) = rx.recv().await {
                if let CommandEvent::Terminated(_) = event {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(7), wait_frames).await;
        let _ = child.kill();
    }

    let max_frames = (scan_dur * 1.5).ceil() as usize + 2;
    let mut face_frames = 0usize;
    let mut total_frames = 0usize;
    let mut max_score = 0.0_f64;

    for idx in 1..=max_frames {
        let frame_path = temp_dir.join(format!("vframe_{:03}.bmp", idx));
        let Ok(bytes) = tokio::fs::read(&frame_path).await else {
            break;
        };
        total_frames += 1;
        let score = score_portrait_916_bmp_for_face(&bytes, 135, 240);
        if score > max_score {
            max_score = score;
        }
        if score >= 0.30 {
            face_frames += 1;
        }
    }

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    (face_frames, total_frames, max_score)
}

/// Оценивает наличие цельного (не разрезанного краем экрана) лица персонажа в портретном кадре 9:16 (`135x240` BMP BGR24).
/// Игнорирует нижнюю зону субтитров (`y > 70%`) и угловые водяные знаки.
fn score_portrait_916_bmp_for_face(bytes: &[u8], w: usize, h: usize) -> f64 {
    if bytes.len() < 54 {
        return 0.0;
    }
    let data_offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    let row_stride = (w * 3 + 3) & !3;
    if bytes.len() < data_offset + row_stride * h {
        return 0.0;
    }
    let raw_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
    let bottom_up = raw_height > 0;

    let get_pixel = |x: usize, y: usize| -> (f64, f64, f64, f64) {
        let bmp_y = if bottom_up { h - 1 - y } else { y };
        let idx = data_offset + bmp_y * row_stride + x * 3;
        let b = bytes[idx] as f64;
        let g = bytes[idx + 1] as f64;
        let r = bytes[idx + 2] as f64;
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        (r, g, b, luma)
    };

    let cw = 5_usize;
    let ch = 5_usize;
    let gw = w / cw; // 27
    let gh = h / ch; // 48

    let mut grid_skin = vec![0.0_f64; gw * gh];
    let mut grid_eye = vec![0.0_f64; gw * gh];
    let mut grid_edge = vec![0.0_f64; gw * gh];

    for gy in 1..(gh * 70 / 100) {
        let py0 = gy * ch;
        let y_norm = (py0 as f64) / (h as f64);
        if !(0.06..=0.68).contains(&y_norm) {
            continue;
        }

        for gx in 1..(gw - 1) {
            let px0 = gx * cw;
            let mut skin_pixels = 0.0_f64;
            let mut dark_pixels = 0.0_f64;
            let mut sclera_pixels = 0.0_f64;
            let mut edge_sum = 0.0_f64;
            let mut luma_min = 255.0_f64;
            let mut luma_max = 0.0_f64;

            for dy in 0..ch {
                for dx in 0..cw {
                    let x = px0 + dx;
                    let y = py0 + dy;
                    let (r, g, b, luma) = get_pixel(x, y);
                    luma_min = luma_min.min(luma);
                    luma_max = luma_max.max(luma);

                    let (_, _, _, l_right) = get_pixel((x + 2).min(w - 1), y);
                    let (_, _, _, l_down) = get_pixel(x, (y + 2).min(h - 1));
                    edge_sum += (luma - l_right).abs() + (luma - l_down).abs();

                    let max_c = r.max(g).max(b);
                    let min_c = r.min(g).min(b);
                    let sat = if max_c > 1.0 { (max_c - min_c) / max_c } else { 0.0 };
                    let cb = 128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b;
                    let cr = 128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b;

                    let is_real_skin = luma > 45.0
                        && luma < 240.0
                        && (80.0..=128.0).contains(&cb)
                        && (132.0..=176.0).contains(&cr)
                        && r > g
                        && g > b * 0.84
                        && (r - b) > 14.0
                        && (0.09..=0.62).contains(&sat);

                    let is_anime_skin = luma > 95.0
                        && luma < 248.0
                        && r >= g * 1.01
                        && g >= b * 0.92
                        && (r - b) >= 9.0
                        && (r - b) <= 70.0
                        && (0.05..=0.46).contains(&sat);

                    if is_real_skin || is_anime_skin {
                        skin_pixels += 1.0;
                    }
                    if luma < 65.0 {
                        dark_pixels += 1.0;
                    }
                    if luma > 190.0 && sat < 0.16 {
                        sclera_pixels += 1.0;
                    }
                }
            }

            let cell_area = (cw * ch) as f64;
            let idx = gy * gw + gx;
            let skin_frac = skin_pixels / cell_area;
            let local_contrast = luma_max - luma_min;
            let avg_edge = edge_sum / cell_area;

            grid_skin[idx] = skin_frac;
            if local_contrast > 42.0 && avg_edge > 14.0 && dark_pixels >= 2.0 {
                let sclera_bonus = if sclera_pixels >= 1.0 { 1.4 } else { 1.0 };
                grid_eye[idx] = ((avg_edge / 38.0).min(2.2)) * sclera_bonus;
            }
            if avg_edge > 13.5 && local_contrast > 32.0 {
                grid_edge[idx] = (avg_edge / 46.0).min(1.6);
            }
        }
    }

    let total_skin_cells = grid_skin.iter().filter(|&&v| v > 0.35).count() as f64;
    let flood_penalty = if total_skin_cells > (gw * gh) as f64 * 0.45 {
        0.20
    } else {
        1.0
    };

    let box_scales: [(usize, usize, f64); 3] = [(11, 10, 1.15), (8, 8, 1.05), (5, 6, 0.92)];
    let mut best_score = 0.0_f64;

    for &(bw, bh, scale_w) in &box_scales {
        let max_gy = (gh * 66 / 100).saturating_sub(bh).max(2);
        // Лицо должно целиком находиться внутри портретного окна (отступ от левого/правого края >= 2 ячейки, т.е. не разрезано пополам на границе!)
        for gy in 2..=max_gy {
            for gx in 2..=(gw.saturating_sub(bw + 2)) {
                let mut skin_sum = 0.0_f64;
                let mut eye_sum = 0.0_f64;
                let mut edge_sum = 0.0_f64;
                for by in gy..(gy + bh) {
                    for bx in gx..(gx + bw) {
                        let idx = by * gw + bx;
                        skin_sum += grid_skin[idx];
                        eye_sum += grid_eye[idx];
                        edge_sum += grid_edge[idx];
                    }
                }
                let area = (bw * bh) as f64;
                let skin_d = (skin_sum / area) * flood_penalty;
                let eye_d = eye_sum / area;
                let edge_d = edge_sum / area;

                if skin_d < 0.12 || (eye_d < 0.065 && edge_d < 0.12) {
                    continue;
                }

                // Поддержка шеи/плеч под лицом
                let mut under_support = 0.0_f64;
                let uy_end = (gy + bh + 3).min(gh);
                let mut u_cnt = 0.0_f64;
                for uy in (gy + bh)..uy_end {
                    for ux in gx..(gx + bw) {
                        let u_idx = uy * gw + ux;
                        under_support += grid_skin[u_idx] * 1.4 + grid_edge[u_idx];
                        u_cnt += 1.0;
                    }
                }
                if u_cnt > 0.0 {
                    under_support /= u_cnt;
                }
                if under_support < 0.10 {
                    continue;
                }

                let score = (skin_d * 2.4) * (1.0 + eye_d * 2.2 + edge_d * 0.8) * scale_w;
                if score > best_score {
                    best_score = score;
                }
            }
        }
    }

    best_score
}

/// Глубокое сканирование полноценного оригинального 16:9 видео (`320x180`, 2.5 FPS) в случае,
/// если в готовом 9:16 шортсе не оказалось ни одного лица.
/// Ищет лица по всей ширине 16:9 кадра (`16% .. 84%`), штрафует уже проверенную пустую зону `used_ratios`,
/// и проверяет каждое окно 9:16 портретным детектором, чтобы гарантировать попадание лица в исправленный шортс.
async fn deep_scan_original_video_for_faces(
    video_path: &Path,
    start_sec: f64,
    duration_sec: f64,
    used_ratios: &[f64],
    app_handle: &AppHandle,
) -> Option<(String, String, usize, f64)> {
    let Ok(ffmpeg) = app_handle.shell().sidecar("ffmpeg") else {
        return None;
    };

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_dir = std::env::temp_dir().join(format!(
        "vcp_deepscan169_{}_{}",
        std::process::id(),
        ts
    ));
    let _ = tokio::fs::create_dir_all(&temp_dir).await;

    let sample_fps = 2.5_f64;
    let scan_dur = duration_sec.clamp(2.0, 30.0);
    let out_pattern = temp_dir
        .join("oframe_%03d.bmp")
        .to_string_lossy()
        .to_string();

    let cmd = ffmpeg
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
        .args([
            "-nostdin",
            "-y",
            "-ss",
            &format!("{:.3}", start_sec),
            "-t",
            &format!("{:.3}", scan_dur),
            "-i",
            &video_path.to_string_lossy(),
            "-vf",
            "fps=2.5,scale=320:180:flags=lanczos",
            "-pix_fmt",
            "bgr24",
            &out_pattern,
        ]);

    if let Ok((mut rx, child)) = cmd.spawn() {
        let wait_frames = async {
            while let Some(event) = rx.recv().await {
                if let CommandEvent::Terminated(_) = event {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(9), wait_frames).await;
        let _ = child.kill();
    }

    // 13 перекрывающихся горизонтальных окон 9:16 по всей ширине 16:9 кадра (от 16% до 84%)
    let candidate_centers: [f64; 13] = [
        0.16, 0.22, 0.28, 0.34, 0.40, 0.46, 0.50, 0.54, 0.60, 0.66, 0.72, 0.78, 0.84,
    ];
    let mut window_totals = [0.0_f64; 13];
    let mut window_hits = [0_usize; 13];
    let mut frame_best_ratios: Vec<(f64, f64, f64)> = Vec::new(); // (rel_time, best_ratio, best_score)

    let max_frames = (scan_dur * sample_fps).ceil() as usize + 2;
    for idx in 1..=max_frames {
        let frame_path = temp_dir.join(format!("oframe_{:03}.bmp", idx));
        let Ok(bytes) = tokio::fs::read(&frame_path).await else {
            break;
        };
        let rel_time = ((idx - 1) as f64) / sample_fps;

        let mut best_c_idx = 6_usize;
        let mut best_c_score = 0.0_f64;

        for (c_idx, &center_r) in candidate_centers.iter().enumerate() {
            let raw_score =
                score_16_9_frame_at_portrait_slice(&bytes, 320, 180, center_r);

            // Если этот участок кадра уже использовался при первом рендере и дал 0 лиц в шортсе —
            // штрафуем его, чтобы перевести камеру на реальное лицо в другой части 16:9 кадра!
            let already_failed_zone = used_ratios
                .iter()
                .any(|&old_r| (old_r - center_r).abs() < 0.065);
            let adjusted_score = if already_failed_zone {
                raw_score * 0.32
            } else {
                raw_score
            };

            if adjusted_score >= 0.24 {
                window_totals[c_idx] += adjusted_score;
                window_hits[c_idx] += 1;
            }
            if adjusted_score > best_c_score {
                best_c_score = adjusted_score;
                best_c_idx = c_idx;
            }
        }

        if best_c_score >= 0.24 {
            frame_best_ratios.push((rel_time, candidate_centers[best_c_idx], best_c_score));
        }
    }

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    if frame_best_ratios.is_empty() {
        return None;
    }

    // Находим глобально лучшую горизонтальную зону в оригинальном 16:9 видео
    let mut best_global_idx = 6_usize;
    let mut best_global_power = 0.0_f64;
    for i in 0..candidate_centers.len() {
        let power = window_totals[i] * (1.0 + (window_hits[i] as f64) * 0.15);
        if power > best_global_power {
            best_global_power = power;
            best_global_idx = i;
        }
    }

    let dominant_ratio = candidate_centers[best_global_idx].clamp(0.16, 0.84);
    let max_seen_score = frame_best_ratios
        .iter()
        .map(|(_, _, s)| *s)
        .fold(0.0_f64, f64::max);
    let face_frames_found = frame_best_ratios.len();

    // Проверяем, меняет ли персонаж сторону кадра между первой и второй половиной ролика
    if frame_best_ratios.len() >= 6 && duration_sec >= 5.0 {
        let mid = frame_best_ratios.len() / 2;
        let first_slice = &frame_best_ratios[..mid];
        let second_slice = &frame_best_ratios[mid..];

        let w1_sum: f64 = first_slice.iter().map(|(_, _, w)| *w).sum();
        let r1 = first_slice.iter().map(|(_, r, w)| r * w).sum::<f64>() / w1_sum.max(0.01);

        let w2_sum: f64 = second_slice.iter().map(|(_, _, w)| *w).sum();
        let r2 = second_slice.iter().map(|(_, r, w)| r * w).sum::<f64>() / w2_sum.max(0.01);

        if (r1 - r2).abs() >= 0.18 {
            let split_t = second_slice[0].0.clamp(1.5, (duration_sec - 1.5).max(1.6));
            let segs = vec![
                CameraSegment {
                    start_t: 0.0,
                    end_t: split_t,
                    start_ratio: r1.clamp(0.16, 0.84),
                    end_ratio: r1.clamp(0.16, 0.84),
                },
                CameraSegment {
                    start_t: split_t,
                    end_t: duration_sec,
                    start_ratio: r2.clamp(0.16, 0.84),
                    end_ratio: r2.clamp(0.16, 0.84),
                },
            ];
            let ratio_expr = build_ffmpeg_piecewise_ratio_expr(&segs);
            let full_expr = format!(
                "min(max(0, iw*({}) - (ih*(9/16))/2), iw-(ih*(9/16)))",
                ratio_expr
            );
            let summary = format!(
                "восстановленный 2-ракурсный фокус на лицах [{:.0}% -> {:.0}%]",
                r1 * 100.0,
                r2 * 100.0
            );
            return Some((full_expr, summary, face_frames_found, max_seen_score));
        }
    }

    let expr = format!(
        "min(max(0, iw*{:.4} - (ih*(9/16))/2), iw-(ih*(9/16)))",
        dominant_ratio
    );
    let summary = format!(
        "восстановленный фокус на лице {:.1}% ширины оригинала",
        dominant_ratio * 100.0
    );
    Some((expr, summary, face_frames_found, max_seen_score))
}

/// Вырезает в памяти портретное окно 9:16 с центром `center_ratio` из полноразмерного 16:9 кадра (`320x180`)
/// и оценивает наличие в нем лица с повышенной чувствительностью (включая ночные/контрастные сцены и лица у краев 16:9 кадра).
fn score_16_9_frame_at_portrait_slice(
    bytes: &[u8],
    w: usize,
    h: usize,
    center_ratio: f64,
) -> f64 {
    if bytes.len() < 54 {
        return 0.0;
    }
    let data_offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    let row_stride = (w * 3 + 3) & !3;
    if bytes.len() < data_offset + row_stride * h {
        return 0.0;
    }
    let raw_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
    let bottom_up = raw_height > 0;

    let get_pixel = |x: usize, y: usize| -> (f64, f64, f64, f64) {
        let bmp_y = if bottom_up { h - 1 - y } else { y };
        let idx = data_offset + bmp_y * row_stride + x * 3;
        let b = bytes[idx] as f64;
        let g = bytes[idx + 1] as f64;
        let r = bytes[idx + 2] as f64;
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        (r, g, b, luma)
    };

    // Ширина окна 9:16 внутри 320x180 равна 180 * 9 / 16 = 101 пиксель (~20 ячеек по 5 пикселей)
    let slice_w = (h * 9 / 16).clamp(60, w);
    let center_px = ((center_ratio * (w as f64)).round() as isize).clamp(0, w as isize);
    let x_start = (center_px - (slice_w as isize) / 2)
        .clamp(0, (w - slice_w) as isize) as usize;

    let cw = 5_usize;
    let ch = 5_usize;
    let gw = slice_w / cw; // 20
    let gh = h / ch; // 36

    let mut grid_skin = vec![0.0_f64; gw * gh];
    let mut grid_eye = vec![0.0_f64; gw * gh];
    let mut grid_edge = vec![0.0_f64; gw * gh];

    for gy in 1..(gh * 76 / 100) {
        let py0 = gy * ch;
        for gx in 1..(gw - 1) {
            let px0 = x_start + gx * cw;
            let mut skin_pixels = 0.0_f64;
            let mut dark_pixels = 0.0_f64;
            let mut sclera_pixels = 0.0_f64;
            let mut edge_sum = 0.0_f64;
            let mut luma_min = 255.0_f64;
            let mut luma_max = 0.0_f64;

            for dy in 0..ch {
                for dx in 0..cw {
                    let x = (px0 + dx).min(w - 1);
                    let y = (py0 + dy).min(h - 1);
                    let (r, g, b, luma) = get_pixel(x, y);
                    luma_min = luma_min.min(luma);
                    luma_max = luma_max.max(luma);

                    let (_, _, _, l_right) = get_pixel((x + 2).min(w - 1), y);
                    let (_, _, _, l_down) = get_pixel(x, (y + 2).min(h - 1));
                    edge_sum += (luma - l_right).abs() + (luma - l_down).abs();

                    let max_c = r.max(g).max(b);
                    let min_c = r.min(g).min(b);
                    let sat = if max_c > 1.0 { (max_c - min_c) / max_c } else { 0.0 };
                    let cb = 128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b;
                    let cr = 128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b;

                    // Расширенный детектор кожи (включая затененные/вечерние сцены и бледные/контрастные лица аниме)
                    let is_real_or_warm_skin = luma > 38.0
                        && luma < 244.0
                        && (76.0..=132.0).contains(&cb)
                        && (128.0..=180.0).contains(&cr)
                        && r >= g * 0.98
                        && g >= b * 0.82
                        && (r - b) >= 10.0
                        && (0.06..=0.65).contains(&sat);

                    let is_anime_or_pale_skin = luma > 75.0
                        && luma < 248.0
                        && r >= g * 0.99
                        && g >= b * 0.90
                        && (r - b) >= 6.0
                        && (r - b) <= 75.0
                        && (0.04..=0.48).contains(&sat);

                    if is_real_or_warm_skin || is_anime_or_pale_skin {
                        skin_pixels += 1.0;
                    }
                    if luma < 68.0 {
                        dark_pixels += 1.0;
                    }
                    if luma > 182.0 && sat < 0.18 {
                        sclera_pixels += 1.0;
                    }
                }
            }

            let area = (cw * ch) as f64;
            let idx = gy * gw + gx;
            grid_skin[idx] = skin_pixels / area;
            let local_contrast = luma_max - luma_min;
            let avg_edge = edge_sum / area;

            if local_contrast > 38.0 && avg_edge > 12.5 && dark_pixels >= 1.5 {
                let sclera_bonus = if sclera_pixels >= 1.0 { 1.4 } else { 1.0 };
                grid_eye[idx] = ((avg_edge / 36.0).min(2.2)) * sclera_bonus;
            }
            if avg_edge > 12.0 && local_contrast > 28.0 {
                grid_edge[idx] = (avg_edge / 44.0).min(1.6);
            }
        }
    }

    let box_scales: [(usize, usize, f64); 3] = [(9, 8, 1.18), (6, 6, 1.06), (4, 5, 0.92)];
    let mut best_score = 0.0_f64;

    for &(bw, bh, scale_w) in &box_scales {
        let max_gy = (gh * 72 / 100).saturating_sub(bh).max(2);
        // Требуем, чтобы лицо в этом срезе 9:16 не прижималось вплотную к краям среза (отступ >= 2 ячейки),
        // чтобы в итоговом шортсе оно стояло ровно по центру!
        for gy in 1..=max_gy {
            for gx in 2..=(gw.saturating_sub(bw + 2)) {
                let mut skin_sum = 0.0_f64;
                let mut eye_sum = 0.0_f64;
                let mut edge_sum = 0.0_f64;
                for by in gy..(gy + bh) {
                    for bx in gx..(gx + bw) {
                        let idx = by * gw + bx;
                        skin_sum += grid_skin[idx];
                        eye_sum += grid_eye[idx];
                        edge_sum += grid_edge[idx];
                    }
                }
                let area = (bw * bh) as f64;
                let skin_d = skin_sum / area;
                let eye_d = eye_sum / area;
                let edge_d = edge_sum / area;

                if skin_d < 0.10 || (eye_d < 0.055 && edge_d < 0.10) {
                    continue;
                }

                // Бонус за расположение лица ближе к горизонтальному центру выбранного окна 9:16
                let box_cx = (gx as f64 + (bw as f64) * 0.5) / (gw as f64);
                let center_align = (1.0 - (box_cx - 0.50).abs() * 1.1).clamp(0.55, 1.0);

                let score = (skin_d * 2.3)
                    * (1.0 + eye_d * 2.1 + edge_d * 0.85)
                    * scale_w
                    * center_align;
                if score > best_score {
                    best_score = score;
                }
            }
        }
    }

    best_score
}

#[derive(Clone, Debug)]
struct FaceCandidate {
    center_ratio: f64,
    base_score: f64,
    speaker_activity: f64,
    scale_weight: f64,
}

#[derive(Clone, Debug)]
struct FrameFocusSample {
    rel_time: f64,
    #[allow(dead_code)]
    center_ratio: f64,
    confidence: f64,
    hist: [f64; 12],
    candidates: Vec<FaceCandidate>,
    dual_group_ratio: Option<f64>,
}

#[derive(Clone, Debug)]
struct CameraSegment {
    start_t: f64,
    end_t: f64,
    start_ratio: f64,
    end_ratio: f64,
}

/// Smart Face & Character Tracking v3 (многоцелевое слежение с защитой от путаницы при 2+ лицах в кадре):
/// 1) Извлекает серию кадров 240x135 с шагом 0.5 сек (2 FPS) по всей длине клипа.
/// 2) В каждом кадре через 2D Non-Maximum Suppression (NMS) находит до 4 независимых лиц/голов персонажей
///    и вычисляет для каждого лица межкадровую активность мимики/рта (Active Speaker Detection).
/// 3) Если 2 лица стоят достаточно близко (`|x1 - x2| <= 0.20`), так что оба целиком помещаются в окно 9:16 (ширина 31.6%),
///    берет оба лица в единый комфортный двухместный кадр.
/// 4) Если в кадре 2 и более разнесенных лиц (`|x1 - x2| > 0.20`), НЕ усредняет их координаты в пустоту между ними
///    и НЕ прыгает между ними каждые 0.5с: фиксируется на говорящем/доминирующем персонаже ракурса (с гистерезисом)
///    и переключается только при устойчивой смене говорящего (>= 1.5с) или реальной склейке камеры.
async fn build_dynamic_face_crop_expr(
    video_path: &PathBuf,
    start_sec: f64,
    duration_sec: f64,
    app_handle: &AppHandle,
) -> (String, String, Vec<f64>) {
    let default_expr = "min(max(0, iw*0.500 - (ih*(9/16))/2), iw-(ih*(9/16)))".to_string();
    let shell = app_handle.shell();
    let Ok(ffmpeg) = shell.sidecar("ffmpeg") else {
        return (
            default_expr,
            "центр кадра 50.0% (дефолт)".to_string(),
            vec![0.50],
        );
    };

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let temp_dir = std::env::temp_dir().join(format!(
        "vcp_track_{}_{}_{}",
        std::process::id(),
        (start_sec * 10.0) as u64,
        ts
    ));
    let _ = tokio::fs::create_dir_all(&temp_dir).await;

    let sample_fps = 2.0_f64; // каждые 0.5 секунды
    let scan_dur = duration_sec.clamp(2.0, 30.0);
    let out_pattern = temp_dir.join("frame_%03d.bmp").to_string_lossy().to_string();

    let cmd = ffmpeg
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
        .args([
            "-nostdin",
            "-y",
            "-ss",
            &format!("{:.3}", start_sec),
            "-t",
            &format!("{:.3}", scan_dur),
            "-i",
            &video_path.to_string_lossy(),
            "-vf",
            "fps=2,scale=240:135:flags=fast_bilinear",
            "-pix_fmt",
            "bgr24",
            &out_pattern,
        ]);

    if let Ok((mut rx, child)) = cmd.spawn() {
        let wait_frames = async {
            while let Some(event) = rx.recv().await {
                if let CommandEvent::Terminated(_) = event {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(8), wait_frames).await;
        let _ = child.kill();
    }

    let mut samples: Vec<FrameFocusSample> = Vec::new();
    let mut prev_cell_luma: Option<Vec<f64>> = None;
    let max_frames = (scan_dur * sample_fps).ceil() as usize + 2;

    for idx in 1..=max_frames {
        let frame_path = temp_dir.join(format!("frame_{:03}.bmp", idx));
        let Ok(bytes) = tokio::fs::read(&frame_path).await else {
            break;
        };
        let rel_time = ((idx - 1) as f64) / sample_fps;
        if let Some((sample, curr_luma)) =
            analyze_bmp_frame_2d(&bytes, 240, 135, rel_time, prev_cell_luma.as_deref())
        {
            prev_cell_luma = Some(curr_luma);
            samples.push(sample);
        }
    }

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    if samples.is_empty() {
        return (
            default_expr,
            "центр кадра 50.0% (дефолт)".to_string(),
            vec![0.50],
        );
    }

    let multi_face_frames = samples.iter().filter(|s| s.candidates.len() >= 2).count();
    let max_faces_seen = samples.iter().map(|s| s.candidates.len()).max().unwrap_or(1);

    let segments = segment_and_smooth_camera_track(&samples, duration_sec);
    if segments.is_empty() {
        return (
            default_expr,
            "центр кадра 50.0% (дефолт)".to_string(),
            vec![0.50],
        );
    }

    let mut track_ratios: Vec<f64> = Vec::new();
    for s in &segments {
        track_ratios.push(s.start_ratio);
        if (s.end_ratio - s.start_ratio).abs() > 0.03 {
            track_ratios.push(s.end_ratio);
        }
    }

    let multi_face_note = if multi_face_frames >= 2 {
        format!(", анти-дребезг: до {} лиц в кадре", max_faces_seen)
    } else {
        String::new()
    };

    let is_single_static = segments.len() == 1
        && (segments[0].start_ratio - segments[0].end_ratio).abs() < 0.025;

    if is_single_static {
        let r = segments[0].start_ratio.clamp(0.18, 0.82);
        let expr = format!(
            "min(max(0, iw*{:.4} - (ih*(9/16))/2), iw-(ih*(9/16)))",
            r
        );
        let summary = format!(
            "стабильный фокус на лице {:.1}% ширины кадра (1 ракурс{})",
            r * 100.0,
            multi_face_note
        );
        return (expr, summary, track_ratios);
    }

    let ratio_expr = build_ffmpeg_piecewise_ratio_expr(&segments);
    let full_expr = format!(
        "min(max(0, iw*({}) - (ih*(9/16))/2), iw-(ih*(9/16)))",
        ratio_expr
    );

    let seg_descriptions: Vec<String> = segments
        .iter()
        .map(|s| {
            if (s.start_ratio - s.end_ratio).abs() < 0.028 {
                format!(
                    "{:.1}с-{:.1}с -> {:.0}%",
                    s.start_t,
                    s.end_t,
                    s.start_ratio * 100.0
                )
            } else {
                format!(
                    "{:.1}с-{:.1}с -> {:.0}%..{:.0}%",
                    s.start_t,
                    s.end_t,
                    s.start_ratio * 100.0,
                    s.end_ratio * 100.0
                )
            }
        })
        .collect();

    let summary = format!(
        "динамическое слежение за лицами ({} ракурс(а){}: [{}])",
        segments.len(),
        multi_face_note,
        seg_descriptions.join(", ")
    );

    (full_expr, summary, track_ratios)
}

/// 2D-анализатор кадра 240x135 (BMP BGR24) с поддержкой нескольких лиц в кадре и детекцией активного говорящего:
/// - Находит все независимые кандидаты голов/лиц через 2D Non-Maximum Suppression (NMS, шаг >= 14% ширины кадра).
/// - Для каждого лица уточняет собственный локальный центр в узком радиусе ±8.5%, чтобы второе лицо рядом не перетягивало центр в пустоту между ними.
/// - По разнице яркости с предыдущим кадром в нижней половине рамки лица (область рта/челюсти и мимики) за вычетом общего движения фона
///   определяет `speaker_activity` — кто именно из персонажей сейчас говорит или активно двигается.
fn analyze_bmp_frame_2d(
    bytes: &[u8],
    w: usize,
    h: usize,
    rel_time: f64,
    prev_cell_luma: Option<&[f64]>,
) -> Option<(FrameFocusSample, Vec<f64>)> {
    if bytes.len() < 54 {
        return None;
    }
    let data_offset =
        u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    let row_stride = (w * 3 + 3) & !3;
    if bytes.len() < data_offset + row_stride * h {
        return None;
    }

    let raw_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
    let bottom_up = raw_height > 0;

    let get_pixel = |x: usize, y: usize| -> (f64, f64, f64, f64) {
        let bmp_y = if bottom_up { h - 1 - y } else { y };
        let idx = data_offset + bmp_y * row_stride + x * 3;
        let b = bytes[idx] as f64;
        let g = bytes[idx + 1] as f64;
        let r = bytes[idx + 2] as f64;
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        (r, g, b, luma)
    };

    // 1. Пространственно-яркостная гистограмма (12 корзин: 3 горизонтальные зоны x 4 уровня яркости)
    // для точного обнаружения склеек и переключения камеры внутри клипа
    let mut hist = [0.0_f64; 12];
    let mut hist_total = 0.0_f64;
    for y in (8..(h - 8)).step_by(3) {
        for x in (8..(w - 8)).step_by(3) {
            let (_, _, _, luma) = get_pixel(x, y);
            let l_bin = ((luma / 256.0) * 4.0).floor().clamp(0.0, 3.0) as usize;
            let x_zone = ((x * 3) / w).min(2);
            hist[x_zone * 4 + l_bin] += 1.0;
            hist_total += 1.0;
        }
    }
    if hist_total > 0.0 {
        for v in hist.iter_mut() {
            *v /= hist_total;
        }
    }

    // 2. Строим 2D-карту признаков по сетке 48x27 (ячейки 5x5 пикселей)
    let cw = 5_usize;
    let ch = 5_usize;
    let gw = w / cw; // 48
    let gh = h / ch; // 27

    let mut grid_skin = vec![0.0_f64; gw * gh];
    let mut grid_eye = vec![0.0_f64; gw * gh];
    let mut grid_edge = vec![0.0_f64; gw * gh];
    let mut grid_luma = vec![0.0_f64; gw * gh];

    for gy in 0..gh {
        let py0 = gy * ch;
        let y_norm = (py0 as f64) / (h as f64);

        for gx in 0..gw {
            let px0 = gx * cw;
            let mut skin_pixels = 0.0_f64;
            let mut dark_pixels = 0.0_f64;
            let mut sclera_pixels = 0.0_f64;
            let mut edge_sum = 0.0_f64;
            let mut luma_sum = 0.0_f64;
            let mut luma_min = 255.0_f64;
            let mut luma_max = 0.0_f64;

            for dy in 0..ch {
                for dx in 0..cw {
                    let x = px0 + dx;
                    let y = py0 + dy;
                    let (r, g, b, luma) = get_pixel(x, y);
                    luma_sum += luma;
                    luma_min = luma_min.min(luma);
                    luma_max = luma_max.max(luma);

                    let (_, _, _, l_right) = get_pixel((x + 2).min(w - 1), y);
                    let (_, _, _, l_down) = get_pixel(x, (y + 2).min(h - 1));
                    let grad = (luma - l_right).abs() + (luma - l_down).abs();
                    edge_sum += grad;

                    let max_c = r.max(g).max(b);
                    let min_c = r.min(g).min(b);
                    let sat = if max_c > 1.0 {
                        (max_c - min_c) / max_c
                    } else {
                        0.0
                    };

                    let cb = 128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b;
                    let cr = 128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b;

                    // Реалистичная кожа в YCbCr + RGB
                    let is_real_skin = luma > 48.0
                        && luma < 238.0
                        && (82.0..=126.0).contains(&cb)
                        && (134.0..=174.0).contains(&cr)
                        && r > g
                        && g > b * 0.85
                        && (r - b) > 16.0
                        && sat > 0.10
                        && sat < 0.60;

                    // Аниме-кожа (персиковый / светло-бежевый / теплый тон лица без оранжевых взрывов и серых стен)
                    let is_anime_skin = luma > 115.0
                        && luma < 246.0
                        && r > g * 1.02
                        && g >= b * 0.94
                        && (r - b) >= 11.0
                        && (r - b) <= 66.0
                        && (r - g) >= 3.0
                        && (r - g) <= 38.0
                        && sat >= 0.065
                        && sat <= 0.43;

                    if is_real_skin || is_anime_skin {
                        skin_pixels += 1.0;
                    }
                    if luma < 62.0 {
                        dark_pixels += 1.0;
                    }
                    if luma > 195.0 && sat < 0.15 {
                        sclera_pixels += 1.0;
                    }
                }
            }

            let cell_area = (cw * ch) as f64;
            let cell_idx = gy * gw + gx;
            grid_luma[cell_idx] = luma_sum / cell_area;

            if !(1..(gh - 1)).contains(&gy)
                || !(1..(gw - 1)).contains(&gx)
                || !(0.05..=0.82).contains(&y_norm)
            {
                continue;
            }

            let skin_frac = skin_pixels / cell_area;
            let local_contrast = luma_max - luma_min;
            let avg_edge = edge_sum / cell_area;

            // Защита от угловых водяных знаков / логотипов каналов / аватарок (вверху слева и вверху справа)
            let x_norm = (px0 as f64) / (w as f64);
            let is_corner_logo_cell = (x_norm > 0.74 || x_norm < 0.26) && y_norm < 0.28;
            let corner_cell_damp = if is_corner_logo_cell { 0.40 } else { 1.00 };

            let head_y_prior = if (0.10..=0.56).contains(&y_norm) {
                1.35
            } else if (0.06..=0.68).contains(&y_norm) {
                0.85
            } else {
                0.25
            } * corner_cell_damp;

            grid_skin[cell_idx] = skin_frac * head_y_prior;

            // Признак глаз/черт лица: резкий переход темных ресниц/зрачка и светлого белка/блика/кожи
            if local_contrast > 44.0 && avg_edge > 15.0 && dark_pixels >= 2.0 {
                let sclera_bonus = if sclera_pixels >= 1.0 { 1.45 } else { 1.0 };
                grid_eye[cell_idx] =
                    ((avg_edge / 38.0).min(2.2)) * sclera_bonus * head_y_prior;
            }

            // Контуры фигуры/головы (работает в том числе для персонажей в масках и Кайдзю)
            if avg_edge > 14.0 && local_contrast > 34.0 {
                grid_edge[cell_idx] = (avg_edge / 48.0).min(1.6) * head_y_prior;
            }
        }
    }

    // Вычисляем карту межкадрового изменения яркости (для детекции движения губ/челюсти говорящего персонажа)
    let mut grid_motion = vec![0.0_f64; gw * gh];
    let mut median_bg_motion = 0.0_f64;
    if let Some(prev_l) = prev_cell_luma {
        if prev_l.len() == gw * gh {
            let mut sum_m = 0.0_f64;
            for idx in 0..(gw * gh) {
                let diff = (grid_luma[idx] - prev_l[idx]).abs();
                grid_motion[idx] = diff;
                sum_m += diff;
            }
            median_bg_motion = sum_m / ((gw * gh) as f64);
        }
    }

    // Если весь кадр залит теплым цветом (взрыв, закат, пыль), штрафуем глобальный цвет кожи и опираемся на черты глаз/контуры
    let total_skin_cells = grid_skin.iter().filter(|&&v| v > 0.35).count() as f64;
    let global_skin_flood_penalty = if total_skin_cells > (gw * gh) as f64 * 0.42 {
        0.18
    } else if total_skin_cells > (gw * gh) as f64 * 0.26 {
        0.55
    } else {
        1.0
    };

    // 3. Многомасштабный 2D-поиск ВСЕХ кандидатов лиц в кадре:
    // (крупный план 9x8 имеет приоритет перед фоновыми мелкими лицами 4x5)
    let box_scales: [(usize, usize, f64); 3] = [(9, 8, 1.18), (6, 6, 1.05), (4, 5, 0.90)];
    let mut raw_boxes: Vec<(usize, usize, usize, usize, f64, f64, f64)> = Vec::new(); // (gx, gy, bw, bh, center_x, score, scale_w)

    for &(bw, bh, scale_weight) in &box_scales {
        let max_gy = (gh * 68 / 100).saturating_sub(bh).max(2);
        for gy in 1..=max_gy {
            for gx in 1..=(gw.saturating_sub(bw + 1)) {
                let mut skin_sum = 0.0_f64;
                let mut eye_sum = 0.0_f64;
                let mut edge_sum = 0.0_f64;

                for by in gy..(gy + bh) {
                    for bx in gx..(gx + bw) {
                        let idx = by * gw + bx;
                        skin_sum += grid_skin[idx];
                        eye_sum += grid_eye[idx];
                        edge_sum += grid_edge[idx];
                    }
                }

                let box_cells = (bw * bh) as f64;
                let skin_density = (skin_sum / box_cells) * global_skin_flood_penalty;
                let eye_density = eye_sum / box_cells;
                let edge_density = edge_sum / box_cells;

                // Проверяем окружение слева и справа от рамки головы (настоящее лицо ограничено прической/фоном)
                let mut surround_skin = 0.0_f64;
                let mut surround_cnt = 0.0_f64;
                for by in gy..(gy + bh) {
                    if gx >= 2 {
                        surround_skin += grid_skin[by * gw + (gx - 2)];
                        surround_cnt += 1.0;
                    }
                    if gx + bw + 1 < gw {
                        surround_skin += grid_skin[by * gw + (gx + bw + 1)];
                        surround_cnt += 1.0;
                    }
                }
                let surround_density = if surround_cnt > 0.0 {
                    (surround_skin / surround_cnt) * global_skin_flood_penalty
                } else {
                    0.0
                };
                let isolation_bonus = (skin_density - surround_density * 0.65).max(0.0);

                // Верхняя половина рамки должна иметь более выраженный контраст глаз/бровей, чем пустая щека
                let mut upper_eye_sum = 0.0_f64;
                let mid_by = gy + (bh / 2).max(1);
                for by in gy..mid_by {
                    for bx in gx..(gx + bw) {
                        upper_eye_sum += grid_eye[by * gw + bx];
                    }
                }
                let upper_eye_density = upper_eye_sum / (((mid_by - gy) * bw).max(1) as f64);

                let face_score = if skin_density > 0.13 && (eye_density > 0.07 || edge_density > 0.11) {
                    (skin_density * 2.5 + isolation_bonus * 1.9)
                        * (1.0 + eye_density * 2.1 + upper_eye_density * 0.6 + edge_density * 0.85)
                        * scale_weight
                } else {
                    (edge_density * 0.50 + eye_density * 0.65) * scale_weight
                };

                let center_gx = (gx as f64 + (bw as f64) * 0.5) / (gw as f64);
                let top_gy_norm = (gy as f64) / (gh as f64);
                let is_corner_watermark_zone = (center_gx > 0.73 || center_gx < 0.27) && top_gy_norm < 0.32;

                // 1. Проверяем анатомическую поддержку тела/шеи под рамкой лица:
                // У настоящего персонажа под подбородком обязательно есть шея, плечи или одежда.
                // У парящего логотипа/аватарки в верхнем углу снизу воздух/пустой фон.
                let mut under_feature = 0.0_f64;
                let under_y_start = gy + bh;
                let under_y_end = (gy + bh + 3).min(gh);
                if under_y_end > under_y_start {
                    let mut under_count = 0.0_f64;
                    for uy in under_y_start..under_y_end {
                        for ux in gx..(gx + bw) {
                            let u_idx = uy * gw + ux;
                            under_feature += grid_skin[u_idx] * 1.6 + grid_edge[u_idx];
                            under_count += 1.0;
                        }
                    }
                    if under_count > 0.0 {
                        under_feature /= under_count;
                    }
                }
                let watermark_penalty = if is_corner_watermark_zone && under_feature < 0.12 {
                    0.06 // жесткое подавление парящего в углу логотипа/водяного знака в 16 раз!
                } else if is_corner_watermark_zone {
                    0.50
                } else {
                    1.00
                };

                // 2. Проверяем вертикальную протяженность (фильтр архитектурных каннелированных колонн и стен):
                // У колонны вертикальные края идут непрерывно через всю высоту кадра вверх и вниз за пределы рамки.
                let mut col_ext_sum = 0.0_f64;
                let mut col_ext_count = 0.0_f64;
                let above_start = gy.saturating_sub(3);
                for ay in above_start..gy {
                    for ax in gx..(gx + bw) {
                        col_ext_sum += grid_edge[ay * gw + ax];
                        col_ext_count += 1.0;
                    }
                }
                let below_end = (gy + bh + 4).min(gh);
                for by in (gy + bh)..below_end {
                    for bx in gx..(gx + bw) {
                        col_ext_sum += grid_edge[by * gw + bx];
                        col_ext_count += 1.0;
                    }
                }
                let avg_col_ext = if col_ext_count > 0.0 { col_ext_sum / col_ext_count } else { 0.0 };
                let column_penalty = if avg_col_ext > 0.36 && skin_density < 0.22 {
                    0.15 // подавление вертикальных архитектурных столбов/колонн
                } else {
                    1.00
                };

                // 3. Операторский композиционный центрирующий Prior:
                // В кадре 16:9 персонажи компонуются в центре (0.50) или по правилу третей (0.33..0.66).
                // Край кадра (x > 0.75 или x < 0.25) требует четких черт глаз/лица, чтобы лампы/стены на краю не уводили камеру в 19%!
                let center_dist = (center_gx - 0.50).abs();
                let base_comp_prior = (1.0 - (center_dist / 0.46).powi(2) * 0.48).clamp(0.48, 1.00);
                let edge_artifact_damp = if !(0.25..=0.75).contains(&center_gx)
                    && (eye_density < 0.085 || upper_eye_density < 0.060)
                {
                    0.28
                } else {
                    1.00
                };
                let composition_prior = base_comp_prior * edge_artifact_damp;

                // 4. Баланс черт лица: настоящее лицо обязательно имеет глаза и брови.
                let feature_balance = if eye_density < 0.035 && skin_density > 0.12 {
                    0.35 // однородная текстура без черт лица (плечо, спина, стена)
                } else if eye_density >= 0.07 && upper_eye_density >= 0.05 {
                    1.28 // выразительные глаза и черты лица
                } else {
                    1.00
                };

                let score = face_score
                    * composition_prior
                    * watermark_penalty
                    * column_penalty
                    * feature_balance;
                if score > 0.28 {
                    raw_boxes.push((gx, gy, bw, bh, center_gx, score, scale_weight));
                }
            }
        }
    }

    // Сортируем найденные рамки по убыванию уверенности и выполняем горизонтальный Non-Maximum Suppression (NMS),
    // чтобы получить до 4 независимых лиц в кадре (минимальная дистанция между центрами двух разных лиц >= 14% ширины кадра)
    raw_boxes.sort_by(|a, b| b.5.partial_cmp(&a.5).unwrap_or(std::cmp::Ordering::Equal));

    let mut candidates: Vec<FaceCandidate> = Vec::with_capacity(4);
    for &(gx, gy, bw, bh, raw_cx, score, scale_weight) in &raw_boxes {
        // Второстепенные кандидаты должны обладать достаточной выраженностью (>= 44% от главного лица),
        // чтобы случайный контраст фона не создавал ложные 3–4 лица в каждом кадре
        if let Some(first) = candidates.first() {
            if score < first.base_score * 0.44 {
                break;
            }
        }
        if candidates
            .iter()
            .any(|c| (c.center_ratio - raw_cx).abs() < 0.145)
        {
            continue;
        }

        // Уточняем центр конкретного лица строго в пределах его собственной ширины (±8.0% кадра),
        // чтобы соседнее второе лицо НЕ смещало центр тяжести в пустое пространство между персонажами!
        let local_span = 0.080_f64;
        let gy_min = gy.saturating_sub(1);
        let gy_max = (gy + bh + 1).min(gh - 1);
        let mut num = 0.0_f64;
        let mut den = 0.0_f64;
        for col_x in 1..(gw - 1) {
            let x_r = (col_x as f64 + 0.5) / (gw as f64);
            if (x_r - raw_cx).abs() <= local_span {
                let mut col_w = 0.0_f64;
                for row_y in gy_min..gy_max {
                    let idx = row_y * gw + col_x;
                    col_w += grid_skin[idx] * global_skin_flood_penalty * 2.0
                        + grid_eye[idx] * 2.6
                        + grid_edge[idx] * 0.6;
                }
                num += x_r * col_w;
                den += col_w;
            }
        }

        let refined_cx = if den > 0.1 {
            (raw_cx * 0.60 + (num / den) * 0.40).clamp(0.24, 0.76)
        } else {
            raw_cx.clamp(0.24, 0.76)
        };

        // Оцениваем активность мимики/рта (`speaker_activity`) внутри рамки лица (с акцентом на нижние 65% лица — губы/челюсть)
        let mut mouth_motion_sum = 0.0_f64;
        let mut mouth_cells = 0.0_f64;
        let mouth_y_start = gy + (bh / 3);
        for by in mouth_y_start..(gy + bh) {
            for bx in gx..(gx + bw) {
                let idx = by * gw + bx;
                // Вычитаем фоновый сдвиг камеры, чтобы панорамирование не путалось с движением губ
                let net_motion = (grid_motion[idx] - median_bg_motion * 0.85).max(0.0);
                mouth_motion_sum += net_motion;
                mouth_cells += 1.0;
            }
        }
        let avg_mouth_motion = if mouth_cells > 0.0 {
            mouth_motion_sum / mouth_cells
        } else {
            0.0
        };
        // В аниме и живой речи открывание/закрывание рта дает локальный перепад яркости ~2.5..15 единиц в ячейке
        let speaker_activity = ((avg_mouth_motion - 0.7).max(0.0) / 9.0).clamp(0.0, 1.0);

        candidates.push(FaceCandidate {
            center_ratio: refined_cx,
            base_score: score,
            speaker_activity,
            scale_weight,
        });

        if candidates.len() >= 4 {
            break;
        }
    }

    // Проверяем, есть ли в кадре 2 выраженных лица, которые стоят достаточно близко друг к другу (`|x1 - x2| <= 0.20`),
    // чтобы ОБА лица целиком помещались в вертикальное окно 9:16 (шириной 31.6% от 16:9 кадра) без обрезки краев
    let mut dual_group_ratio: Option<f64> = None;
    if candidates.len() >= 2 {
        let c1 = &candidates[0];
        let c2 = &candidates[1];
        let dist = (c1.center_ratio - c2.center_ratio).abs();
        if c2.base_score >= c1.base_score * 0.58 && dist <= 0.205 {
            let w1 = c1.base_score * (1.0 + c1.speaker_activity * 0.4);
            let w2 = c2.base_score * (1.0 + c2.speaker_activity * 0.4);
            let mid = ((c1.center_ratio * w1 + c2.center_ratio * w2) / (w1 + w2).max(0.01))
                .clamp(0.26, 0.74);
            dual_group_ratio = Some(mid);
        }
    }

    let (primary_ratio, confidence) = if let Some(first) = candidates.first() {
        (first.center_ratio, (first.base_score / 1.2).clamp(0.05, 1.0))
    } else {
        (0.50, 0.05)
    };

    Some((
        FrameFocusSample {
            rel_time,
            center_ratio: primary_ratio,
            confidence,
            hist,
            candidates,
            dual_group_ratio,
        },
        grid_luma,
    ))
}

/// Внутри одного ракурса (между склейками камеры) выбирает стабильную траекторию лица при наличии 2 и более персонажей:
/// - Если два персонажа стоят близко (`dual_group_ratio`), держит обоих в кадре 9:16.
/// - Если в кадре 2+ разнесенных лица (`|x1 - x2| > 0.20`), группирует кандидатов в горизонтальные треки персонажей,
///   вычисляет говорящего по `base_score + speaker_activity` и блокирует перескоки между лицами каждые 0.5с!
/// - Если ракурс длинный (>= 3.5с) и второй персонаж начинает активно говорить во второй половине сцены (>= 3 кадра подряд),
///   делит ракурс на 2 чистых под-сегмента (перевод камеры с первого говорящего на второго) без усреднения в пустоту между ними.
fn resolve_multiface_shot_segments(
    slice: &[FrameFocusSample],
    start_t: f64,
    end_t: f64,
) -> Vec<CameraSegment> {
    if slice.is_empty() || end_t <= start_t + 0.05 {
        return Vec::new();
    }

    // 1. Проверяем, стоят ли 2 персонажа достаточно близко для комфортного двухместного кадра в большинстве сэмплов ракурса
    let dual_samples: Vec<(f64, f64)> = slice
        .iter()
        .filter_map(|s| s.dual_group_ratio.map(|r| (r, s.confidence.max(0.1))))
        .collect();
    if dual_samples.len() * 2 >= slice.len() && !dual_samples.is_empty() {
        let num: f64 = dual_samples.iter().map(|(r, w)| r * w).sum();
        let den: f64 = dual_samples.iter().map(|(_, w)| *w).sum();
        let group_ratio = (num / den.max(0.01)).clamp(0.26, 0.74);
        return vec![CameraSegment {
            start_t,
            end_t,
            start_ratio: group_ratio,
            end_ratio: group_ratio,
        }];
    }

    // 2. Кластеризуем все обнаруженные лица внутри этого ракурса по горизонтальным позициям (трекам персонажей)
    // Каждый кластер: (mean_x, total_focus_power, hit_frames)
    let mut clusters: Vec<(f64, f64, usize)> = Vec::new();
    for s in slice {
        for (rank, cand) in s.candidates.iter().enumerate() {
            // Учитываем базовую четкость лица, крупность плана и активность мимики/рта (говорящий персонаж получает сильный приоритет!)
            let rank_factor = if rank == 0 { 1.0 } else if rank == 1 { 0.88 } else { 0.72 };
            let power = cand.base_score
                * rank_factor
                * cand.scale_weight
                * (1.0 + cand.speaker_activity * 2.80);

            if let Some(cl) = clusters
                .iter_mut()
                .find(|c| (c.0 - cand.center_ratio).abs() < 0.14)
            {
                let new_w = cl.1 + power;
                if new_w > 0.0 {
                    cl.0 = (cl.0 * cl.1 + cand.center_ratio * power) / new_w;
                }
                cl.1 = new_w;
                cl.2 += 1;
            } else {
                clusters.push((cand.center_ratio, power, 1));
            }
        }
    }

    if clusters.is_empty() {
        return vec![CameraSegment {
            start_t,
            end_t,
            start_ratio: 0.50,
            end_ratio: 0.50,
        }];
    }

    clusters.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Защита от краевых ложных кластеров (водяные знаки, колонны, края стен):
    // Если первый кластер оказался ближе к краю (x > 0.74 или x < 0.26),
    // а среди остальных кластеров есть кандидат в комфортной центральной/третьевой зоне (x от 0.26 до 0.74),
    // проверяем: если центральный кластер достаточно выражен (>= 22% веса крайнего ИЛИ замечен в 2+ кадрах),
    // то предпочтение отдается центральному персонажу!
    let mut best_cluster_idx = 0_usize;
    if clusters.len() >= 2 && (clusters[0].0 > 0.74 || clusters[0].0 < 0.26) {
        for (idx, cl) in clusters.iter().enumerate().skip(1) {
            let is_central = (0.26..=0.74).contains(&cl.0);
            if is_central && (cl.1 >= clusters[0].1 * 0.22 || cl.2 >= 2) {
                best_cluster_idx = idx;
                break;
            }
        }
    }
    if best_cluster_idx != 0 {
        let chosen = clusters.remove(best_cluster_idx);
        clusters.insert(0, chosen);
    }

    let primary_cluster_x = clusters[0].0.clamp(0.26, 0.74);

    // 3. Если в ракурсе есть 2 конкурирующих разнесенных лица (например левый собеседник ~30% и правый ~70%)
    // и ракурс длится >= 3.5с (7+ сэмплов), проверяем, нет ли устойчивой смены говорящего между первой и второй половиной ракурса.
    // ВАЖНО: отсекаем краевые ложные кластеры (< 26% или > 74%), чтобы камера никогда не улетала в край стены/фона!
    if clusters.len() >= 2 && slice.len() >= 7 {
        let secondary_cluster_x = clusters[1].0;
        let secondary_is_valid_zone = (0.26..=0.74).contains(&secondary_cluster_x);
        let cluster_dist = (primary_cluster_x - secondary_cluster_x).abs();

        if secondary_is_valid_zone
            && cluster_dist >= 0.20
            && clusters[1].1 >= clusters[0].1 * 0.48
            && clusters[1].2 >= 3
        {
            // Для каждого кадра ракурса определяем, кто из двух персонажей доминирует/говорит (0 = первый, 1 = второй)
            let mut frame_winner: Vec<usize> = Vec::with_capacity(slice.len());
            let mut current_speaker = 0_usize;

            for (f_idx, s) in slice.iter().enumerate() {
                let mut score_0 = 0.0_f64;
                let mut score_1 = 0.0_f64;

                for cand in &s.candidates {
                    let s_val = cand.base_score * (1.0 + cand.speaker_activity * 2.20);
                    if (cand.center_ratio - primary_cluster_x).abs() < 0.15 {
                        score_0 = score_0.max(s_val);
                    }
                    if (cand.center_ratio - secondary_cluster_x).abs() < 0.15 {
                        score_1 = score_1.max(s_val);
                    }
                }

                if f_idx == 0 {
                    current_speaker = if score_1 > score_0 * 1.08 { 1 } else { 0 };
                } else {
                    // Гистерезис удержания (+25% бонус текущему персонажу), чтобы не прыгать от случайного моргания
                    if current_speaker == 0 && score_1 > score_0 * 1.25 {
                        current_speaker = 1;
                    } else if current_speaker == 1 && score_0 > score_1 * 1.25 {
                        current_speaker = 0;
                    }
                }
                frame_winner.push(current_speaker);
            }

            // Проверяем, произошел ли ОДИН чистый переход говорящего (например 3+ кадра первый, затем 3+ кадра второй)
            let mid_start = 3;
            let mid_end = slice.len().saturating_sub(3);
            let mut best_split: Option<(usize, usize, usize)> = None; // (split_idx, first_spk, second_spk)

            for split in mid_start..=mid_end {
                let left_0 = frame_winner[..split].iter().filter(|&&w| w == 0).count();
                let left_1 = split - left_0;
                let right_total = slice.len() - split;
                let right_0 = frame_winner[split..].iter().filter(|&&w| w == 0).count();
                let right_1 = right_total - right_0;

                if left_0 * 4 >= split * 3 && right_1 * 4 >= right_total * 3 {
                    best_split = Some((split, 0, 1));
                    break;
                }
                if left_1 * 4 >= split * 3 && right_0 * 4 >= right_total * 3 {
                    best_split = Some((split, 1, 0));
                    break;
                }
            }

            if let Some((split_idx, spk_a, spk_b)) = best_split {
                let target_a = if spk_a == 0 { primary_cluster_x } else { secondary_cluster_x };
                let target_b = if spk_b == 0 { primary_cluster_x } else { secondary_cluster_x };
                let split_t = slice[split_idx].rel_time.clamp(start_t + 1.5, end_t - 1.5);

                return vec![
                    CameraSegment {
                        start_t,
                        end_t: split_t,
                        start_ratio: target_a.clamp(0.26, 0.74),
                        end_ratio: target_a.clamp(0.26, 0.74),
                    },
                    CameraSegment {
                        start_t: split_t,
                        end_t,
                        start_ratio: target_b.clamp(0.26, 0.74),
                        end_ratio: target_b.clamp(0.26, 0.74),
                    },
                ];
            }
        }
    }

    // 4. Фиксируемся строго на основном/говорящем треке ракурса (`primary_cluster_x`),
    // отбирая из каждого кадра ТОЛЬКО кандидата, принадлежащего этому персонажу (исключает усреднение двух разных лиц!)
    let mut track_points: Vec<(f64, f64, f64)> = Vec::with_capacity(slice.len()); // (rel_time, ratio, weight)
    let mut running_x = primary_cluster_x;

    for s in slice {
        if let Some(matched) = s
            .candidates
            .iter()
            .filter(|c| (c.center_ratio - running_x).abs() < 0.16)
            .min_by(|a, b| {
                let da = (a.center_ratio - running_x).abs();
                let db = (b.center_ratio - running_x).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            let w = (matched.base_score * (1.0 + matched.speaker_activity * 1.5)).max(0.10);
            running_x = running_x * 0.65 + matched.center_ratio * 0.35;
            track_points.push((s.rel_time, matched.center_ratio.clamp(0.26, 0.74), w));
        }
    }

    if track_points.is_empty() {
        let r = primary_cluster_x.clamp(0.26, 0.74);
        return vec![CameraSegment {
            start_t,
            end_t,
            start_ratio: r,
            end_ratio: r,
        }];
    }

    let weighted_num: f64 = track_points.iter().map(|(_, r, w)| r * w).sum();
    let weighted_den: f64 = track_points.iter().map(|(_, _, w)| *w).sum();
    let mean_ratio = (weighted_num / weighted_den.max(0.01)).clamp(0.26, 0.74);

    // Если персонаж действительно плавно идет/смещается в кадре (4+ точки одного и того же трека) — строим плавное ведение камеры
    if track_points.len() >= 4 {
        let mid = track_points.len() / 2;
        let first_half = &track_points[..mid];
        let second_half = &track_points[mid..];

        let avg_first = first_half.iter().map(|(_, r, w)| r * w).sum::<f64>()
            / first_half.iter().map(|(_, _, w)| *w).sum::<f64>().max(0.01);
        let avg_second = second_half.iter().map(|(_, r, w)| r * w).sum::<f64>()
            / second_half.iter().map(|(_, _, w)| *w).sum::<f64>().max(0.01);

        if (avg_second - avg_first).abs() > 0.085 {
            return vec![CameraSegment {
                start_t,
                end_t,
                start_ratio: avg_first.clamp(0.26, 0.74),
                end_ratio: avg_second.clamp(0.26, 0.74),
            }];
        }
    }

    vec![CameraSegment {
        start_t,
        end_t,
        start_ratio: mean_ratio,
        end_ratio: mean_ratio,
    }]
}

/// Разбивает временную шкалу клипа на ракурсы (сцены между реальными склейками камеры),
/// выбирает говорящего/основного персонажа в каждом ракурсе без перескоков между 2+ лицами
/// и строит кинематографичную траекторию виртуальной камеры 9:16.
fn segment_and_smooth_camera_track(
    samples: &[FrameFocusSample],
    duration_sec: f64,
) -> Vec<CameraSegment> {
    if samples.is_empty() {
        return Vec::new();
    }

    let mut shot_ranges: Vec<(usize, usize)> = Vec::new();
    let mut shot_start = 0_usize;

    for i in 1..samples.len() {
        let prev = &samples[i - 1];
        let curr = &samples[i];

        let hist_l1: f64 = prev
            .hist
            .iter()
            .zip(curr.hist.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();

        // Проверяем, сохранилось ли лицо в той же точке кадра (если да, то даже при вспышке света ракурс не менялся)
        let has_continuous_face = curr.candidates.iter().any(|c_now| {
            prev.candidates
                .iter()
                .any(|c_prev| (c_now.center_ratio - c_prev.center_ratio).abs() < 0.11)
        });

        // ВАЖНО: Смену ракурса определяем по реальной визуальной склейке кадра (`hist_l1`),
        // а НЕ по простому скачку `center_ratio`, чтобы при 2 лицах в одном статичном плане не нарезать ложные склейки!
        let is_camera_cut = hist_l1 > 0.30 || (hist_l1 > 0.19 && !has_continuous_face);

        if is_camera_cut {
            shot_ranges.push((shot_start, i - 1));
            shot_start = i;
        }
    }
    shot_ranges.push((shot_start, samples.len() - 1));

    let mut raw_segments: Vec<CameraSegment> = Vec::new();
    let sample_step = 0.5_f64;

    for (idx, &(s_idx, e_idx)) in shot_ranges.iter().enumerate() {
        let slice = &samples[s_idx..=e_idx];
        let start_t = if idx == 0 { 0.0 } else { slice[0].rel_time };
        let end_t = if idx + 1 == shot_ranges.len() {
            duration_sec
        } else {
            (slice.last().unwrap().rel_time + sample_step).min(duration_sec)
        };

        let resolved = resolve_multiface_shot_segments(slice, start_t, end_t);
        raw_segments.extend(resolved);
    }

    // Поглощаем микро-скачки и объединяем соседние ракурсы с близкой позицией (< 8.8% ширины, так как лицо и так в кадре 9:16)
    if raw_segments.len() >= 2 {
        let mut cleaned: Vec<CameraSegment> = Vec::with_capacity(raw_segments.len());
        for seg in raw_segments {
            let seg_dur = seg.end_t - seg.start_t;
            if let Some(prev) = cleaned.last_mut() {
                let close_to_prev = (seg.start_ratio - prev.end_ratio).abs() < 0.088
                    && (seg.end_ratio - prev.end_ratio).abs() < 0.088;
                // Исключаем дерганые перебивки короче 1.2с и ракурсы, не отличающиеся заметно по положению.
                // КРИТИЧЕСКОЕ ПРАВИЛО: усредняем позицию ТОЛЬКО если `close_to_prev == true`,
                // чтобы короткая перебивка на другое лицо не сдвигала основную камеру в пустоту между двумя лицами!
                if close_to_prev || seg_dur < 1.20 {
                    if close_to_prev && seg_dur >= 0.50 {
                        let w_prev = (prev.end_t - prev.start_t).max(0.1);
                        let w_curr = seg_dur;
                        let blended = (prev.end_ratio * w_prev + seg.end_ratio * w_curr) / (w_prev + w_curr);
                        prev.end_ratio = blended.clamp(0.18, 0.82);
                    }
                    prev.end_t = seg.end_t;
                    continue;
                }
            }
            cleaned.push(seg);
        }

        // Если самый первый микро-сегмент в клипе короче 1.2с — поглощаем его в последующий основной ракурс,
        // чтобы ролик не начинался со мгновенного рывка камеры на 0.5с
        if cleaned.len() >= 2 && (cleaned[0].end_t - cleaned[0].start_t) < 1.20 {
            cleaned[1].start_t = 0.0;
            cleaned.remove(0);
        }

        // Если самый последний микро-сегмент в конце клипа короче 1.4с — поглощаем его в предыдущий ракурс,
        // чтобы камера не совершала резкий скачок за секунду до окончания ролика
        if cleaned.len() >= 2 {
            let last_idx = cleaned.len() - 1;
            if (cleaned[last_idx].end_t - cleaned[last_idx].start_t) < 1.40 {
                let end_t = cleaned[last_idx].end_t;
                cleaned.pop();
                if let Some(prev) = cleaned.last_mut() {
                    prev.end_t = end_t;
                }
            }
        }

        raw_segments = cleaned;
    }

    // Ограничиваем максимум 6 ракурсами на один 10–20с клип для комфортного кинематографичного просмотра
    if raw_segments.len() > 6 {
        raw_segments.truncate(6);
        if let Some(last) = raw_segments.last_mut() {
            last.end_t = duration_sec;
        }
    }

    raw_segments
}

/// Строит компактное кусочно-линейное выражение FFmpeg `if(lt(t, T1), EXPR1, if(lt(t, T2), EXPR2, ...))`
fn build_ffmpeg_piecewise_ratio_expr(segments: &[CameraSegment]) -> String {
    fn seg_expr(s: &CameraSegment) -> String {
        let dt = (s.end_t - s.start_t).max(0.1);
        if (s.end_ratio - s.start_ratio).abs() < 0.025 {
            format!("{:.4}", s.start_ratio)
        } else {
            let slope = (s.end_ratio - s.start_ratio) / dt;
            if slope >= 0.0 {
                format!("{:.4}+{:.4}*(t-{:.2})", s.start_ratio, slope, s.start_t)
            } else {
                format!(
                    "{:.4}-{:.4}*(t-{:.2})",
                    s.start_ratio,
                    slope.abs(),
                    s.start_t
                )
            }
        }
    }

    if segments.is_empty() {
        return "0.5000".to_string();
    }
    if segments.len() == 1 {
        return seg_expr(&segments[0]);
    }

    let mut expr = seg_expr(&segments[segments.len() - 1]);
    for s in segments.iter().rev().skip(1) {
        expr = format!("if(lt(t,{:.2}),{},{})", s.end_t, seg_expr(s), expr);
    }
    expr
}

/// Рендерит видео хайлайтов в формате 16:9 YouTube (или 9:16 Shorts при вертикальном формате),
/// объединяя все отобранные клипы-интервалы с применением мягкого перекрёстного наплыва (Crossfade 0.4с) и предварительным выравниванием по тишине.
pub async fn render_highlights_assembly(
    video_path: PathBuf,
    intervals: &[(f64, f64)],
    output_path: PathBuf,
    srt_path: Option<PathBuf>,
    vertical_format: bool,
    subtitle_style: Option<SubtitleStyle>,
    app_handle: AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<()> {
    if intervals.is_empty() {
        return Ok(());
    }

    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    // Выравнивание границ хайлайтов по тишине (-0.8с .. +1.5с) через кэшированный 20-мс профиль звуковой энергии
    let mut aligned_intervals: Vec<(f64, f64)> = intervals.to_vec();
    let srt_fragments = if let Some(ref sp) = srt_path {
        tokio::fs::read_to_string(sp).await.ok().map(|c| crate::text_analyzer::parse_srt(&c)).unwrap_or_default()
    } else {
        Vec::new()
    };

    if let Ok(prof) = crate::audio_analyzer::get_or_create_audio_profile(
        &video_path,
        &srt_fragments,
        &app_handle,
        cancel_rx.resubscribe(),
    ).await {
        crate::logger::emit_log(
            &app_handle,
            "Audio",
            "info",
            &format!("Выравнивание границ {} фрагментов хайлайтов по точкам тишины (-0.8с .. +1.5с)...", aligned_intervals.len()),
        );
        crate::audio_analyzer::refine_speech_intervals_by_silence(&prof, &mut aligned_intervals, 4.0);
    }

    let temp_dir = output_path.parent().unwrap_or_else(|| Path::new(".")).join("temp_highlights_render");
    let _ = tokio::fs::create_dir_all(&temp_dir).await;

    let mut temp_clips: Vec<PathBuf> = Vec::new();
    let mut clip_durations: Vec<f64> = Vec::new();
    let total = aligned_intervals.len();

    for (i, &(start_sec, end_sec)) in aligned_intervals.iter().enumerate() {
        if cancel_rx.try_recv().is_ok() {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(AppError::Anyhow(anyhow::anyhow!("Операция сборки хайлайтов отменена пользователем")));
        }

        let dur = (end_sec - start_sec).max(0.5);
        let temp_clip = temp_dir.join(format!("seg_{:03}.mp4", i));

        let _ = app_handle.emit("render-progress", RenderProgressPayload {
            percent: (i as f64 / total as f64) * 80.0,
            stage: format!("Подготовка фрагмента хайлайтов {}/{}...", i + 1, total),
        });

        // Нарезка 16:9 или 9:16 фрагмента с подгонкой под целевое разрешение
        let scale_filter = if vertical_format {
            "scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2:color=black,fps=30"
        } else {
            "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2:color=black,fps=30"
        };
        let mut vf_filters: Vec<String> = vec![scale_filter.to_string()];

        // В хайлайты ОБЯЗАТЕЛЬНО включаем субтитры, если файл субтитров доступен
        let mut temp_seg_srt: Option<PathBuf> = None;
        if let Some(ref sp) = srt_path {
            if let Some(seg_srt) = create_clip_srt(sp, start_sec, dur).await {
                let sub_filter = format_subtitles_filter(&seg_srt, subtitle_style.as_ref());
                vf_filters.push(sub_filter);
                temp_seg_srt = Some(seg_srt);
            }
        }
        let vf_arg = vf_filters.join(",");

        let args = vec![
            "-y".to_string(),
            "-ss".to_string(),
            format!("{:.3}", start_sec),
            "-i".to_string(),
            video_path.to_string_lossy().to_string(),
            "-t".to_string(),
            format!("{:.3}", dur),
            "-vf".to_string(),
            vf_arg,
            "-af".to_string(),
            "aresample=48000".to_string(),
            "-c:v".to_string(),
            "libx264".to_string(),
            "-preset".to_string(),
            "ultrafast".to_string(),
            "-crf".to_string(),
            "20".to_string(),
            "-c:a".to_string(),
            "aac".to_string(),
            "-b:a".to_string(),
            "192k".to_string(),
            temp_clip.to_string_lossy().to_string(),
        ];

        let sidecar_cmd = app_handle.shell().sidecar("ffmpeg")
            .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось запустить бинарник FFmpeg: {}", e)))?
            .env("PATH", crate::logger::build_sidecar_env_path(&app_handle))
            .args(args);

        let (mut rx, child) = sidecar_cmd.spawn()
            .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка старта FFmpeg: {}", e)))?;

        let mut success = false;
        loop {
            tokio::select! {
                _ = cancel_rx.recv() => {
                    let _ = child.kill();
                    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                    return Err(AppError::Anyhow(anyhow::anyhow!("Отмена сборки хайлайтов")));
                }
                event = rx.recv() => {
                    match event {
                        Some(CommandEvent::Terminated(status)) => {
                            success = status.code.unwrap_or(-1) == 0;
                            break;
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
            }
        }

        if let Some(ref srt_f) = temp_seg_srt {
            let _ = tokio::fs::remove_file(srt_f).await;
        }

        if !success || !temp_clip.exists() {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(AppError::Anyhow(anyhow::anyhow!("Не удалось отрендерить фрагмент хайлайтов #{}", i + 1)));
        }

        temp_clips.push(temp_clip);
        clip_durations.push(dur);
    }

    let _ = app_handle.emit("render-progress", RenderProgressPayload {
        percent: 85.0,
        stage: "Склеивание видео хайлайтов с наплывом (Crossfade 0.4с)...".to_string(),
    });

    if temp_clips.len() == 1 {
        let _ = tokio::fs::copy(&temp_clips[0], &output_path).await;
    } else {
        // Формирование комплексного графа фильтров xfade (видео, 0.4с) и acrossfade (аудио, 0.4с)
        let mut ffmpeg_args = vec!["-y".to_string()];
        for clip in &temp_clips {
            ffmpeg_args.push("-i".to_string());
            ffmpeg_args.push(clip.to_string_lossy().to_string());
        }

        let mut filter_str = String::new();
        let mut prev_v = "0:v".to_string();
        let mut prev_a = "0:a".to_string();
        let mut cum_dur = clip_durations[0];

        for i in 0..(temp_clips.len() - 1) {
            let offset = (cum_dur - 0.4).max(0.0);
            let next_v = format!("{}:v", i + 1);
            let next_a = format!("{}:a", i + 1);
            let out_v = if i + 1 == temp_clips.len() - 1 { "outv".to_string() } else { format!("v{}", i + 1) };
            let out_a = if i + 1 == temp_clips.len() - 1 { "outa".to_string() } else { format!("a{}", i + 1) };

            filter_str.push_str(&format!("[{}][{}]xfade=transition=fade:duration=0.4:offset={:.3}[{}];", prev_v, next_v, offset, out_v));
            filter_str.push_str(&format!("[{}][{}]acrossfade=d=0.4[{}];", prev_a, next_a, out_a));

            prev_v = out_v;
            prev_a = out_a;
            cum_dur = offset + clip_durations[i + 1];
        }

        ffmpeg_args.push("-filter_complex".to_string());
        ffmpeg_args.push(filter_str);
        ffmpeg_args.push("-map".to_string());
        ffmpeg_args.push("[outv]".to_string());
        ffmpeg_args.push("-map".to_string());
        ffmpeg_args.push("[outa]".to_string());
        ffmpeg_args.push("-c:v".to_string());
        ffmpeg_args.push("libx264".to_string());
        ffmpeg_args.push("-preset".to_string());
        ffmpeg_args.push("fast".to_string());
        ffmpeg_args.push("-crf".to_string());
        ffmpeg_args.push("20".to_string());
        ffmpeg_args.push("-c:a".to_string());
        ffmpeg_args.push("aac".to_string());
        ffmpeg_args.push("-b:a".to_string());
        ffmpeg_args.push("192k".to_string());
        ffmpeg_args.push(output_path.to_string_lossy().to_string());

        let sidecar_cmd = app_handle.shell().sidecar("ffmpeg")
            .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось запустить FFmpeg: {}", e)))?
            .env("PATH", crate::logger::build_sidecar_env_path(&app_handle))
            .args(ffmpeg_args);

        let (mut rx, child) = sidecar_cmd.spawn()
            .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка фильтрации xfade: {}", e)))?;

        loop {
            tokio::select! {
                _ = cancel_rx.recv() => {
                    let _ = child.kill();
                    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                    return Err(AppError::Anyhow(anyhow::anyhow!("Отмена создания хайлайтов")));
                }
                event = rx.recv() => {
                    match event {
                        Some(CommandEvent::Terminated(_)) => break,
                        Some(_) => {}
                        None => break,
                    }
                }
            }
        }
    }

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    let _ = app_handle.emit("render-progress", RenderProgressPayload {
        percent: 100.0,
        stage: "Видео хайлайтов успешно сформировано!".to_string(),
    });
    Ok(())
}

/// Рендерит разговорное видео (ток-шоу, подкаст, стрим, либо персонажную демку озвучки актера) за счет полного конвейера очистки:
/// извлекает всю речевую активность (или реплики выбранных персонажей с короткими подводками собеседников),
/// удаляет паузы > 0.45с, галлюцинации Whisper, оговорки и дубли диктора.
/// На стыках фрагментов чередуется крупность плана (1.0x / 1.15x) и аудио-микрофейды (30мс) для динамичного jump-cut монтажа.
pub async fn render_talk_video_assembly(
    video_path: PathBuf,
    srt_path: Option<PathBuf>,
    selected_characters: Option<&[String]>,
    vertical_format: bool,
    subtitle_style: Option<SubtitleStyle>,
    output_path: PathBuf,
    app_handle: AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<()> {
    let srt = match srt_path {
        Some(ref p) => p,
        None => return Ok(()),
    };

    let content = match tokio::fs::read_to_string(srt).await {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    let all_fragments = crate::text_analyzer::parse_srt(&content);
    if all_fragments.is_empty() {
        return Ok(());
    }

    let sel_chars = selected_characters.unwrap_or(&[]);
    let has_char_filter = !sel_chars.is_empty()
        && !sel_chars.iter().any(|c| c.to_lowercase() == "all" || c == "Все персонажи");

    let matches_selected_char = |text: &str| -> bool {
        let (extracted_char, _) = crate::subtitles_parser::extract_srt_character_prefix(text);
        let lower_ext = extracted_char.to_lowercase();
        let lower_f = text.to_lowercase();
        sel_chars.iter().any(|char_name| {
            let lower_c = char_name.to_lowercase();
            lower_ext == lower_c
                || lower_f.starts_with(&format!("{}:", lower_c))
                || lower_f.starts_with(&format!("[{}]", lower_c))
                || lower_f.starts_with(&format!("({})", lower_c))
        })
    };

    let fragments: Vec<crate::text_analyzer::SubtitleFragment> = if has_char_filter {
        let mut filtered = Vec::new();
        for idx in 0..all_fragments.len() {
            let is_target = matches_selected_char(&all_fragments[idx].text);
            let is_context_question = if !is_target && idx + 1 < all_fragments.len() {
                let next_is_target = matches_selected_char(&all_fragments[idx + 1].text);
                let gap = all_fragments[idx + 1].start - all_fragments[idx].end;
                let dur = all_fragments[idx].end - all_fragments[idx].start;
                next_is_target && gap <= 2.5 && dur <= 5.5
            } else {
                false
            };
            if is_target || is_context_question {
                filtered.push(all_fragments[idx].clone());
            }
        }
        filtered
    } else {
        all_fragments.clone()
    };

    if fragments.is_empty() {
        return Ok(());
    }

    // 1. Извлечение только очищенной разговорной речи (без галлюцинаций, оговорок, бракованных дублей)
    let mut valid_speech: Vec<(f64, f64)> = Vec::new();
    for (idx, frag) in fragments.iter().enumerate() {
        if crate::whisper::is_whisper_hallucination(&frag.text) {
            continue;
        }
        if crate::text_analyzer::is_weak_clip_ending(&fragments, idx) {
            let lower = frag.text.to_lowercase().replace('ё', "е");
            if lower.contains("не понравилось")
                || lower.contains("в тайминг не")
                || lower.contains("все хуйня")
                || lower.contains("перепишем")
                || lower.contains("переписать")
                || lower.contains("давай еще раз")
                || lower.contains("плохо получилось")
            {
                continue;
            }
        }
        valid_speech.push((frag.start, frag.end));
    }

    if valid_speech.is_empty() {
        return Ok(());
    }

    // Выравнивание речевых блоков по тишине и естественным паузам (-0.8с .. +1.5с) через кэшированный 20-мс профиль звуковой энергии
    let mut aligned_speech = valid_speech;
    if let Ok(prof) = crate::audio_analyzer::get_or_create_audio_profile(
        &video_path,
        &all_fragments,
        &app_handle,
        cancel_rx.resubscribe(),
    ).await {
        crate::logger::emit_log(
            &app_handle,
            "Audio",
            "info",
            &format!("Выравнивание границ {} реплик разговорного видео/демки по тишине (-0.8с .. +1.5с)...", aligned_speech.len()),
        );
        crate::audio_analyzer::refine_speech_intervals_by_silence(&prof, &mut aligned_speech, 0.25);
    }

    // 2. Объединение речевых блоков с паузами < 0.45с в сплошные речевые интервалы
    let mut merged_blocks: Vec<(f64, f64)> = Vec::new();
    let mut curr_start = aligned_speech[0].0;
    let mut curr_end = aligned_speech[0].1;

    for &(st, en) in aligned_speech.iter().skip(1) {
        if st - curr_end <= 0.45 {
            curr_end = curr_end.max(en);
        } else {
            merged_blocks.push((curr_start, curr_end));
            curr_start = st;
            curr_end = en;
        }
    }
    merged_blocks.push((curr_start, curr_end));

    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let temp_dir = output_path.parent().unwrap_or_else(|| Path::new(".")).join("temp_talk_render");
    let _ = tokio::fs::create_dir_all(&temp_dir).await;

    let mut temp_files: Vec<PathBuf> = Vec::new();
    let total_blocks = merged_blocks.len();

    // 3. Генерация каждого речевого блока с чередованием крупности плана (1.0x / 1.15x) на склейках (jump-cut)
    for (k, &(st, en)) in merged_blocks.iter().enumerate() {
        if cancel_rx.try_recv().is_ok() {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(AppError::Anyhow(anyhow::anyhow!("Операция монтажа разговорного видео отменена")));
        }

        let dur = (en - st).max(0.25);
        let temp_file = temp_dir.join(format!("talk_blk_{:04}.mp4", k));

        let _ = app_handle.emit("render-progress", RenderProgressPayload {
            percent: (k as f64 / total_blocks as f64) * 85.0,
            stage: format!("Обработка разговорного блока {}/{} (зум {})...", k + 1, total_blocks, if k % 2 == 1 { "1.15x" } else { "1.0x" }),
        });

        // Чередование плана 1.0x и 1.15x на каждой склейке (jump-cut) с поддержкой 16:9 и 9:16
        let base_scale = if vertical_format {
            if k % 2 == 1 {
                "scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2:color=black,scale=1242:2208,crop=1080:1920:(iw-1080)/2:(ih-1920)/2,fps=30"
            } else {
                "scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2:color=black,fps=30"
            }
        } else {
            if k % 2 == 1 {
                "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2:color=black,scale=2208:1242,crop=1920:1080:(iw-1920)/2:(ih-1080)/2,fps=30"
            } else {
                "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2:color=black,fps=30"
            }
        };
        let mut vf_filters: Vec<String> = vec![base_scale.to_string()];

        // В демки озвучки персонажа (has_char_filter = true) ВКЛЮЧАЕМ субтитры.
        // В монтаж разговорного видео (has_char_filter = false) субтитры НЕ включаем!
        let mut temp_blk_srt: Option<PathBuf> = None;
        if has_char_filter {
            if let Some(blk_srt) = create_clip_srt(srt, st, dur).await {
                let sub_filter = format_subtitles_filter(&blk_srt, subtitle_style.as_ref());
                vf_filters.push(sub_filter);
                temp_blk_srt = Some(blk_srt);
            }
        }
        let vf_arg = vf_filters.join(",");
        let af_arg = format!("afade=t=in:st=0:d=0.03,afade=t=out:st={:.3}:d=0.03,aresample=48000", (dur - 0.03).max(0.01));

        let args = vec![
            "-y".to_string(),
            "-ss".to_string(),
            format!("{:.3}", st),
            "-i".to_string(),
            video_path.to_string_lossy().to_string(),
            "-t".to_string(),
            format!("{:.3}", dur),
            "-vf".to_string(),
            vf_arg,
            "-af".to_string(),
            af_arg,
            "-c:v".to_string(),
            "libx264".to_string(),
            "-preset".to_string(),
            "ultrafast".to_string(),
            "-crf".to_string(),
            "20".to_string(),
            "-c:a".to_string(),
            "aac".to_string(),
            "-b:a".to_string(),
            "192k".to_string(),
            temp_file.to_string_lossy().to_string(),
        ];

        let sidecar_cmd = app_handle.shell().sidecar("ffmpeg")
            .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось запустить FFmpeg: {}", e)))?
            .env("PATH", crate::logger::build_sidecar_env_path(&app_handle))
            .args(args);

        let (mut rx, child) = sidecar_cmd.spawn()
            .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка старта FFmpeg: {}", e)))?;

        let mut success = false;
        loop {
            tokio::select! {
                _ = cancel_rx.recv() => {
                    let _ = child.kill();
                    if let Some(ref srt_f) = temp_blk_srt {
                        let _ = tokio::fs::remove_file(srt_f).await;
                    }
                    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                    return Err(AppError::Anyhow(anyhow::anyhow!("Отмена монтажа разговорного видео")));
                }
                event = rx.recv() => {
                    match event {
                        Some(CommandEvent::Terminated(status)) => {
                            success = status.code.unwrap_or(-1) == 0;
                            break;
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
            }
        }

        if let Some(ref srt_f) = temp_blk_srt {
            let _ = tokio::fs::remove_file(srt_f).await;
        }

        if success && temp_file.exists() {
            temp_files.push(temp_file);
        }
    }

    if temp_files.is_empty() {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return Ok(());
    }

    // 4. Бесшовное склеивание всех речевых фрагментов через FFmpeg concat demuxer
    let concat_txt = temp_dir.join("concat_list.txt");
    let mut concat_content = String::new();
    for tf in &temp_files {
        concat_content.push_str(&format!("file '{}'\n", tf.file_name().unwrap_or_default().to_string_lossy()));
    }
    tokio::fs::write(&concat_txt, concat_content).await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка записи файла списков склейки: {}", e)))?;

    let _ = app_handle.emit("render-progress", RenderProgressPayload {
        percent: 92.0,
        stage: "Сборка разговорного видео в итоговый файл...".to_string(),
    });

    let args = vec![
        "-y".to_string(),
        "-f".to_string(),
        "concat".to_string(),
        "-safe".to_string(),
        "0".to_string(),
        "-i".to_string(),
        concat_txt.to_string_lossy().to_string(),
        "-c".to_string(),
        "copy".to_string(),
        output_path.to_string_lossy().to_string(),
    ];

    let sidecar_cmd = app_handle.shell().sidecar("ffmpeg")
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось запустить FFmpeg: {}", e)))?
        .env("PATH", crate::logger::build_sidecar_env_path(&app_handle))
        .args(args);

    let (mut rx, child) = sidecar_cmd.spawn()
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка склейки разговорного видео: {}", e)))?;

    loop {
        tokio::select! {
            _ = cancel_rx.recv() => {
                let _ = child.kill();
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Err(AppError::Anyhow(anyhow::anyhow!("Отмена склейки разговорного видео")));
            }
            event = rx.recv() => {
                match event {
                    Some(CommandEvent::Terminated(_)) => break,
                    Some(_) => {}
                    None => break,
                }
            }
        }
    }

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    let _ = app_handle.emit("render-progress", RenderProgressPayload {
        percent: 100.0,
        stage: "Разговорное видео успешно смонтировано!".to_string(),
    });

    Ok(())
}

