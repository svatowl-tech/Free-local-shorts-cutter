// src-tauri/src/bumpers.rs

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandEvent;
use crate::error::{AppResult, AppError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BumperItemConfig {
    pub enabled: bool,
    pub mode: String, // "custom_video" | "generate"
    pub custom_video_path: Option<String>,
    pub custom_audio_path: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
    pub duration_sec: f64,
    pub bg_image_path: Option<String>,
    pub logo_image_path: Option<String>,
    pub logo_scale: f64, // 0.1 .. 2.0
    pub logo_x_percent: f64, // 0 .. 100
    pub logo_y_percent: f64, // 0 .. 100
    pub title_color: String,
    pub title_size: i32,
    pub title_x_percent: f64,
    pub title_y_percent: f64,
    pub transition: String, // "crossfade" | "fade_black" | "cut"
}

impl Default for BumperItemConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "generate".to_string(),
            custom_video_path: None,
            custom_audio_path: None,
            title: "Студия озвучки".to_string(),
            subtitle: Some("Подпишись на канал!".to_string()),
            duration_sec: 2.5,
            bg_image_path: None,
            logo_image_path: None,
            logo_scale: 0.5,
            logo_x_percent: 50.0,
            logo_y_percent: 40.0,
            title_color: "#FFFFFF".to_string(),
            title_size: 32,
            title_x_percent: 50.0,
            title_y_percent: 75.0,
            transition: "crossfade".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BumperCategoryConfig {
    pub preroll: BumperItemConfig,
    pub postroll: BumperItemConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BumperPreset {
    pub id: String,
    pub name: String,
    pub v916: BumperCategoryConfig,
    pub v169: BumperCategoryConfig,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BumperGlobalConfig {
    pub enabled: bool,
    pub active_preset_id: String,
    pub presets: Vec<BumperPreset>,
}

/// Получает длительность аудиофайла через FFmpeg probe
pub async fn probe_audio_duration(audio_path: &Path, app_handle: &AppHandle) -> f64 {
    let sidecar_cmd = match app_handle.shell().sidecar("ffmpeg") {
        Ok(cmd) => cmd,
        Err(_) => return 2.5,
    };

    let args = vec![
        "-i".to_string(),
        audio_path.to_string_lossy().to_string(),
        "-hide_banner".to_string(),
    ];

    let (mut rx, child) = match sidecar_cmd.args(args).spawn() {
        Ok(res) => res,
        Err(_) => return 2.5,
    };

    let mut output = String::new();
    while let Some(event) = rx.recv().await {
        if let CommandEvent::Stderr(bytes) = event {
            output.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    let _ = child.kill();

    if let Some(pos) = output.find("Duration: ") {
        let dur_str = &output[pos + 10..];
        let parts: Vec<&str> = dur_str.split(',').next().unwrap_or("").trim().split(':').collect();
        if parts.len() == 3 {
            let h: f64 = parts[0].parse().unwrap_or(0.0);
            let m: f64 = parts[1].parse().unwrap_or(0.0);
            let s: f64 = parts[2].parse().unwrap_or(0.0);
            let total = h * 3600.0 + m * 60.0 + s;
            if total > 0.1 {
                return total;
            }
        }
    }

    2.5
}

/// Генерация / Предрендеринг одного бампера (преролла или постролла)
pub async fn render_single_bumper_video(
    item: &BumperItemConfig,
    is_vertical: bool,
    source_video_path: &Path,
    output_path: &Path,
    app_handle: &AppHandle,
) -> AppResult<()> {
    if !item.enabled {
        return Ok(());
    }

    if let Some(parent) = output_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let target_w = if is_vertical { 1080 } else { 1920 };
    let target_h = if is_vertical { 1920 } else { 1080 };

    // Режим А: Использование готового кастомного видеоролика
    if item.mode == "custom_video" {
        if let Some(ref custom_path_str) = item.custom_video_path {
            let custom_p = PathBuf::from(custom_path_str);
            if custom_p.exists() {
                let vf_scale = format!(
                    "scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,fps=30",
                    target_w, target_h, target_w, target_h
                );
                let args = vec![
                    "-y".to_string(),
                    "-i".to_string(),
                    custom_p.to_string_lossy().to_string(),
                    "-vf".to_string(),
                    vf_scale,
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
                    output_path.to_string_lossy().to_string(),
                ];

                let sidecar_cmd = app_handle.shell().sidecar("ffmpeg")
                    .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка старта FFmpeg: {}", e)))?
                    .args(args);

                let (mut rx, child) = sidecar_cmd.spawn()
                    .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка создания бампера: {}", e)))?;

                while let Some(event) = rx.recv().await {
                    if let CommandEvent::Terminated(_) = event {
                        break;
                    }
                }
                let _ = child.kill();
                return Ok(());
            }
        }
    }

    // Режим Б: Автоматическая генерация заставки в приложении
    let mut duration = item.duration_sec.clamp(0.5, 15.0);

    let has_custom_audio = item
        .custom_audio_path
        .as_ref()
        .map(|p| Path::new(p).exists())
        .unwrap_or(false);

    if has_custom_audio {
        let audio_p = PathBuf::from(item.custom_audio_path.as_ref().unwrap());
        duration = probe_audio_duration(&audio_p, app_handle).await;
    }

    // 1. Формирование входных файлов для FFmpeg
    let mut ffmpeg_args = vec!["-y".to_string()];

    // Вход 0: Видеофон (размытый кадр исходника или картинка)
    let bg_image_exists = item
        .bg_image_path
        .as_ref()
        .map(|p| Path::new(p).exists())
        .unwrap_or(false);

    if bg_image_exists {
        ffmpeg_args.push("-loop".to_string());
        ffmpeg_args.push("1".to_string());
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push(item.bg_image_path.as_ref().unwrap().clone());
    } else {
        ffmpeg_args.push("-ss".to_string());
        ffmpeg_args.push("5.0".to_string());
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push(source_video_path.to_string_lossy().to_string());
    }

    // Вход 1 (Опционально): Изображение логотипа (PNG / GIF / JPG)
    let logo_image_exists = item
        .logo_image_path
        .as_ref()
        .map(|p| Path::new(p).exists())
        .unwrap_or(false);

    let logo_input_idx = if logo_image_exists {
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push(item.logo_image_path.as_ref().unwrap().clone());
        Some(1usize)
    } else {
        None
    };

    // Вход аудио: Кастомный аудиофайл либо генерируемая тишина
    let audio_input_idx = if has_custom_audio {
        let idx = if logo_image_exists { 2 } else { 1 };
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push(item.custom_audio_path.as_ref().unwrap().clone());
        Some(idx)
    } else {
        None
    };

    ffmpeg_args.push("-t".to_string());
    ffmpeg_args.push(format!("{:.3}", duration));

    // 2. Сборка комплексного фильтра FFmpeg
    let mut filter_chain = String::new();

    // Видеофон
    if bg_image_exists {
        filter_chain.push_str(&format!(
            "[0:v]scale={}:{}:force_original_aspect_ratio=increase,crop={}:{}[bg];",
            target_w, target_h, target_w, target_h
        ));
    } else {
        filter_chain.push_str(&format!(
            "[0:v]boxblur=25:5,scale={}:{}:force_original_aspect_ratio=increase,crop={}:{}[bg];",
            target_w, target_h, target_w, target_h
        ));
    }

    let mut current_v = "bg".to_string();

    // Наложение логотипа PNG/GIF с масштабированием и координатами X/Y
    if let Some(l_idx) = logo_input_idx {
        let scale_w = (target_w as f64 * item.logo_scale.clamp(0.05, 1.5)) as i32;
        let pos_x = (target_w as f64 * (item.logo_x_percent / 100.0) - (scale_w as f64 / 2.0)) as i32;
        let pos_y = (target_h as f64 * (item.logo_y_percent / 100.0) - (scale_w as f64 / 2.0)) as i32;

        let next_v = "with_logo".to_string();
        filter_chain.push_str(&format!(
            "[{}][{}:v]scale={}:-1[logo];[{}][logo]overlay=x={}:y={}[{}];",
            current_v, l_idx, scale_w, current_v, pos_x, pos_y, next_v
        ));
        current_v = next_v;
    }

    // Наложение надписи заголовка
    if !item.title.trim().is_empty() {
        let clean_t = item.title.replace(':', "\\:").replace('\'', "");
        let font_size = (item.title_size as f64 * (target_w as f64 / 1080.0)) as i32;
        let pos_x = (target_w as f64 * (item.title_x_percent / 100.0)) as i32;
        let pos_y = (target_h as f64 * (item.title_y_percent / 100.0)) as i32;
        let next_v = "with_text".to_string();

        filter_chain.push_str(&format!(
            "[{}]drawtext=text='{}':fontsize={}:fontcolor={}:x={}-w/2:y={}-h/2:box=1:boxcolor=black@0.5:boxborderw=8[{}];",
            current_v, clean_t, font_size, item.title_color, pos_x, pos_y, next_v
        ));
        current_v = next_v;
    }

    filter_chain.push_str(&format!("[{}]fps=30[outv]", current_v));

    ffmpeg_args.push("-filter_complex".to_string());
    ffmpeg_args.push(filter_chain);
    ffmpeg_args.push("-map".to_string());
    ffmpeg_args.push("[outv]".to_string());

    if let Some(a_idx) = audio_input_idx {
        ffmpeg_args.push("-map".to_string());
        ffmpeg_args.push(format!("{}:a", a_idx));
        ffmpeg_args.push("-af".to_string());
        ffmpeg_args.push(format!("afade=t=in:st=0:d=0.2,afade=t=out:st={:.3}:d=0.2,aresample=48000", (duration - 0.2).max(0.01)));
    } else {
        ffmpeg_args.push("-f".to_string());
        ffmpeg_args.push("lavfi".to_string());
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push("anullsrc=r=48000:cl=stereo".to_string());
    }

    ffmpeg_args.push("-c:v".to_string());
    ffmpeg_args.push("libx264".to_string());
    ffmpeg_args.push("-preset".to_string());
    ffmpeg_args.push("ultrafast".to_string());
    ffmpeg_args.push("-crf".to_string());
    ffmpeg_args.push("20".to_string());
    ffmpeg_args.push("-c:a".to_string());
    ffmpeg_args.push("aac".to_string());
    ffmpeg_args.push("-b:a".to_string());
    ffmpeg_args.push("192k".to_string());
    ffmpeg_args.push(output_path.to_string_lossy().to_string());

    let sidecar_cmd = app_handle.shell().sidecar("ffmpeg")
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось запустить FFmpeg: {}", e)))?
        .args(ffmpeg_args);

    let (mut rx, child) = sidecar_cmd.spawn()
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка отрисовки заставки: {}", e)))?;

    while let Some(event) = rx.recv().await {
        if let CommandEvent::Terminated(_) = event {
            break;
        }
    }
    let _ = child.kill();

    Ok(())
}
