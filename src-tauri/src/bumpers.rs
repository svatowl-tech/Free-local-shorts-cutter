// src-tauri/src/bumpers.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use crate::error::{AppError, AppResult};

fn default_true() -> bool {
    true
}
fn default_one() -> f64 {
    1.0
}
fn default_bg_mode() -> String {
    "blur_frame".to_string()
}
fn default_bg_primary() -> String {
    "#0f172a".to_string()
}
fn default_bg_secondary() -> String {
    "#312e81".to_string()
}
fn default_bg_blur() -> f64 {
    12.0
}
fn default_bg_dim() -> f64 {
    0.35
}
fn default_bg_scale() -> f64 {
    1.10
}
fn default_subtitle_size() -> i32 {
    18
}
fn default_subtitle_color() -> String {
    "#cbd5e1".to_string()
}
fn default_outline_color() -> String {
    "#000000".to_string()
}
fn default_outline_width() -> i32 {
    2
}
fn default_plate_style() -> String {
    "glass".to_string()
}
fn default_plate_color() -> String {
    "#000000".to_string()
}
fn default_plate_opacity() -> f64 {
    0.55
}
fn default_animation() -> String {
    "fade_zoom".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BumperItemConfig {
    pub enabled: bool,
    pub mode: String, // "custom_video" | "generate"
    pub custom_video_path: Option<String>,
    pub custom_audio_path: Option<String>,
    #[serde(default)]
    pub custom_audio_duration: Option<f64>,
    #[serde(default = "default_true")]
    pub sync_duration_to_audio: bool,
    #[serde(default = "default_one")]
    pub audio_volume: f64,
    #[serde(default = "default_true")]
    pub audio_fade_in: bool,
    #[serde(default = "default_true")]
    pub audio_fade_out: bool,
    pub title: String,
    pub subtitle: Option<String>,
    pub duration_sec: f64,
    #[serde(default = "default_bg_mode")]
    pub bg_mode: String, // "blur_frame" | "custom_video" | "custom_image" | "gradient"
    #[serde(default)]
    pub bg_video_path: Option<String>,
    pub bg_image_path: Option<String>,
    #[serde(default = "default_bg_primary")]
    pub bg_color_primary: String,
    #[serde(default = "default_bg_secondary")]
    pub bg_color_secondary: String,
    #[serde(default = "default_bg_blur")]
    pub bg_blur: f64, // 0 .. 40
    #[serde(default = "default_bg_dim")]
    pub bg_dim: f64, // 0.0 .. 0.90
    #[serde(default = "default_bg_scale")]
    pub bg_scale: f64, // 1.0 .. 2.0
    #[serde(default = "default_one")]
    pub bg_saturation: f64, // 0.0 .. 2.0
    #[serde(default = "default_one")]
    pub bg_contrast: f64, // 0.5 .. 1.5
    #[serde(default)]
    pub bg_vignette: bool,
    #[serde(default = "default_one")]
    pub bg_video_speed: f64, // 0.5 .. 2.0
    pub logo_image_path: Option<String>,
    pub logo_scale: f64, // 0.1 .. 2.0
    pub logo_x_percent: f64, // 0 .. 100
    pub logo_y_percent: f64, // 0 .. 100
    #[serde(default = "default_one")]
    pub logo_opacity: f64, // 0.1 .. 1.0
    #[serde(default)]
    pub logo_radius: f64, // 0 .. 50
    #[serde(default = "default_true")]
    pub logo_shadow: bool,
    pub title_color: String,
    pub title_size: i32,
    #[serde(default = "default_subtitle_size")]
    pub subtitle_size: i32,
    #[serde(default = "default_subtitle_color")]
    pub subtitle_color: String,
    #[serde(default = "default_outline_color")]
    pub outline_color: String,
    #[serde(default = "default_outline_width")]
    pub outline_width: i32,
    #[serde(default = "default_plate_style")]
    pub plate_style: String, // "glass" | "solid" | "none"
    #[serde(default = "default_plate_color")]
    pub plate_color: String,
    #[serde(default = "default_plate_opacity")]
    pub plate_opacity: f64,
    pub title_x_percent: f64,
    pub title_y_percent: f64,
    #[serde(default = "default_animation")]
    pub animation: String, // "fade_zoom" | "slide_up" | "pulse" | "none"
    pub transition: String, // "crossfade" | "fade_black" | "cut"
    #[serde(default)]
    pub overlay_png_base64: Option<String>,
}

impl Default for BumperItemConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "generate".to_string(),
            custom_video_path: None,
            custom_audio_path: None,
            custom_audio_duration: None,
            sync_duration_to_audio: true,
            audio_volume: 1.0,
            audio_fade_in: true,
            audio_fade_out: true,
            title: "Студия озвучки".to_string(),
            subtitle: Some("Подпишись на канал!".to_string()),
            duration_sec: 2.5,
            bg_mode: "blur_frame".to_string(),
            bg_video_path: None,
            bg_image_path: None,
            bg_color_primary: "#0f172a".to_string(),
            bg_color_secondary: "#312e81".to_string(),
            bg_blur: 12.0,
            bg_dim: 0.35,
            bg_scale: 1.10,
            bg_saturation: 1.0,
            bg_contrast: 1.0,
            bg_vignette: true,
            bg_video_speed: 1.0,
            logo_image_path: None,
            logo_scale: 0.5,
            logo_x_percent: 50.0,
            logo_y_percent: 38.0,
            logo_opacity: 1.0,
            logo_radius: 0.0,
            logo_shadow: true,
            title_color: "#FFFFFF".to_string(),
            title_size: 36,
            subtitle_size: 18,
            subtitle_color: "#cbd5e1".to_string(),
            outline_color: "#000000".to_string(),
            outline_width: 2,
            plate_style: "glass".to_string(),
            plate_color: "#000000".to_string(),
            plate_opacity: 0.55,
            title_x_percent: 50.0,
            title_y_percent: 74.0,
            animation: "fade_zoom".to_string(),
            transition: "crossfade".to_string(),
            overlay_png_base64: None,
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

/// Декодирует Base64-строку (включая `data:...;base64,...`) в сырые байты
pub fn decode_base64_payload(input: &str) -> Option<Vec<u8>> {
    let raw = if let Some((_, b64)) = input.split_once(";base64,") {
        b64.trim()
    } else {
        input.trim()
    };
    if raw.is_empty() {
        return None;
    }

    const LUT: [u8; 256] = {
        let mut table = [255u8; 256];
        let mut i = 0u8;
        while i < 26 {
            table[(b'A' + i) as usize] = i;
            table[(b'a' + i) as usize] = i + 26;
            i += 1;
        }
        let mut d = 0u8;
        while d < 10 {
            table[(b'0' + d) as usize] = d + 52;
            d += 1;
        }
        table[b'+' as usize] = 62;
        table[b'/' as usize] = 63;
        table[b'-' as usize] = 62;
        table[b'_' as usize] = 63;
        table
    };

    let mut out = Vec::with_capacity((raw.len() * 3) / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;

    for &b in raw.as_bytes() {
        if b == b'=' || b == b'\r' || b == b'\n' || b == b' ' {
            continue;
        }
        let val = LUT[b as usize];
        if val == 255 {
            continue;
        }
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

/// Материализует путь к медиафайлу: если передан `data:...;base64,...`, сохраняет во временный файл в `cache_dir`
fn materialize_media_path(
    path_or_data: Option<&String>,
    cache_dir: &Path,
    prefix: &str,
    default_ext: &str,
) -> Option<PathBuf> {
    let s = path_or_data?.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with("data:") {
        let ext = if s.contains("image/png") {
            "png"
        } else if s.contains("image/jpeg") || s.contains("image/jpg") {
            "jpg"
        } else if s.contains("image/gif") {
            "gif"
        } else if s.contains("image/webp") {
            "webp"
        } else if s.contains("audio/wav") {
            "wav"
        } else if s.contains("audio/ogg") {
            "ogg"
        } else if s.contains("audio/mp4") || s.contains("audio/m4a") {
            "m4a"
        } else if s.contains("audio/") {
            "mp3"
        } else if s.contains("video/webm") {
            "webm"
        } else if s.contains("video/") {
            "mp4"
        } else {
            default_ext
        };
        let bytes = decode_base64_payload(s)?;
        let out_file = cache_dir.join(format!("{}.{}", prefix, ext));
        std::fs::write(&out_file, bytes).ok()?;
        return Some(out_file);
    }

    let p = PathBuf::from(s);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// Получает точную длительность аудио- или видеофайла через FFmpeg probe
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

    let (mut rx, child) = match sidecar_cmd
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
        .args(args)
        .spawn()
    {
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
                return (total * 100.0).round() / 100.0;
            }
        }
    }

    2.5
}

/// Генерация / Предрендеринг одного бампера (преролла или постролла) с поддержкой:
/// - авто-длительности по кастомному звуку (`sync_duration_to_audio`),
/// - кастомного логотипа, заставки, фонового изображения и заднего видео,
/// - эффектов заднего видео/фона (размытие, затемнение, зум, насыщенность, контраст, скорость, виньетка),
/// - наложения текста с подложкой или PNG-оверлея из интерактивного превью.
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

    let cache_dir = output_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = tokio::fs::create_dir_all(&cache_dir).await;

    let target_w = if is_vertical { 1080 } else { 1920 };
    let target_h = if is_vertical { 1920 } else { 1080 };
    let tag_prefix = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("bumper");

    // Материализуем все возможные пользовательские медиафайлы (работает и с путями на диске, и с data:base64)
    let custom_audio_file = materialize_media_path(
        item.custom_audio_path.as_ref(),
        &cache_dir,
        &format!("{}_audio", tag_prefix),
        "mp3",
    );
    let custom_video_file = materialize_media_path(
        item.custom_video_path.as_ref(),
        &cache_dir,
        &format!("{}_custom_vid", tag_prefix),
        "mp4",
    );
    let bg_video_file = materialize_media_path(
        item.bg_video_path.as_ref(),
        &cache_dir,
        &format!("{}_bg_vid", tag_prefix),
        "mp4",
    );
    let bg_image_file = materialize_media_path(
        item.bg_image_path.as_ref(),
        &cache_dir,
        &format!("{}_bg_img", tag_prefix),
        "png",
    );
    let logo_image_file = materialize_media_path(
        item.logo_image_path.as_ref(),
        &cache_dir,
        &format!("{}_logo", tag_prefix),
        "png",
    );
    let overlay_png_file = materialize_media_path(
        item.overlay_png_base64.as_ref(),
        &cache_dir,
        &format!("{}_canvas_overlay", tag_prefix),
        "png",
    );

    // Определяем итоговую длительность бампера:
    // Если загружен свой звук и включена синхронизация по длине звука — берем точную длину аудиофайла!
    let mut duration = item.duration_sec.clamp(0.5, 30.0);
    if item.sync_duration_to_audio {
        if let Some(ref audio_p) = custom_audio_file {
            let probed = probe_audio_duration(audio_p, app_handle).await;
            if probed > 0.2 {
                duration = probed.clamp(0.5, 30.0);
            } else if let Some(cached_dur) = item.custom_audio_duration {
                if cached_dur > 0.2 {
                    duration = cached_dur.clamp(0.5, 30.0);
                }
            }
        } else if let Some(cached_dur) = item.custom_audio_duration {
            if cached_dur > 0.2 && item.custom_audio_path.is_some() {
                duration = cached_dur.clamp(0.5, 30.0);
            }
        }
    }

    // Режим А: Готовая видео-заставка (с возможностью замены звука на кастомный аудиофайл)
    if item.mode == "custom_video" {
        if let Some(ref custom_p) = custom_video_file {
            if custom_p.exists() {
                let mut args = vec![
                    "-y".to_string(),
                    "-i".to_string(),
                    custom_p.to_string_lossy().to_string(),
                ];

                if let Some(ref aud_p) = custom_audio_file {
                    args.push("-i".to_string());
                    args.push(aud_p.to_string_lossy().to_string());
                    args.push("-t".to_string());
                    args.push(format!("{:.3}", duration));
                }

                let vf_scale = format!(
                    "scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,fps=30",
                    target_w, target_h, target_w, target_h
                );
                args.push("-vf".to_string());
                args.push(vf_scale);

                if custom_audio_file.is_some() {
                    args.push("-map".to_string());
                    args.push("0:v:0".to_string());
                    args.push("-map".to_string());
                    args.push("1:a:0".to_string());
                    let vol = item.audio_volume.clamp(0.0, 2.0);
                    let mut af_parts = vec![format!("volume={:.2}", vol)];
                    if item.audio_fade_in {
                        af_parts.push("afade=t=in:st=0:d=0.20".to_string());
                    }
                    if item.audio_fade_out && duration > 0.4 {
                        af_parts.push(format!("afade=t=out:st={:.3}:d=0.25", (duration - 0.25).max(0.05)));
                    }
                    af_parts.push("aresample=48000".to_string());
                    args.push("-af".to_string());
                    args.push(af_parts.join(","));
                } else {
                    args.push("-af".to_string());
                    args.push("aresample=48000".to_string());
                }

                args.extend([
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
                ]);

                let sidecar_cmd = app_handle
                    .shell()
                    .sidecar("ffmpeg")
                    .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка старта FFmpeg: {}", e)))?
                    .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
                    .args(args);

                let (mut rx, child) = sidecar_cmd
                    .spawn()
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

    // Режим Б: Студийный генератор заставки с настраиваемым задним видео/фоном, логотипом, текстом и звуком
    let mut ffmpeg_args = vec!["-y".to_string()];

    // Вход 0: Задний фон (кастомное видео | кастомная картинка | кадр из видео | градиентная плашка)
    let mut using_video_bg = false;
    if item.bg_mode == "custom_video" && bg_video_file.is_some() {
        using_video_bg = true;
        ffmpeg_args.push("-stream_loop".to_string());
        ffmpeg_args.push("-1".to_string());
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push(bg_video_file.as_ref().unwrap().to_string_lossy().to_string());
    } else if (item.bg_mode == "custom_image" || bg_image_file.is_some()) && bg_image_file.is_some() {
        ffmpeg_args.push("-loop".to_string());
        ffmpeg_args.push("1".to_string());
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push(bg_image_file.as_ref().unwrap().to_string_lossy().to_string());
    } else if item.bg_mode != "gradient" && source_video_path.exists() {
        using_video_bg = true;
        ffmpeg_args.push("-stream_loop".to_string());
        ffmpeg_args.push("-1".to_string());
        ffmpeg_args.push("-ss".to_string());
        ffmpeg_args.push("3.0".to_string());
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push(source_video_path.to_string_lossy().to_string());
    } else {
        // Студийный цветной фон через lavfi color
        let hex_color = item.bg_color_primary.trim_start_matches('#');
        let safe_hex = if hex_color.len() == 6 { hex_color } else { "0f172a" };
        ffmpeg_args.push("-f".to_string());
        ffmpeg_args.push("lavfi".to_string());
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push(format!("color=c=0x{}:s={}x{}:r=30", safe_hex, target_w, target_h));
    }

    let mut next_input_idx = 1usize;

    // Если фронтенд передал готовый 1:1 PNG-оверлей из интерактивного Canvas (с логотипом и текстом)
    let canvas_overlay_idx = if let Some(ref ov_path) = overlay_png_file {
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push(ov_path.to_string_lossy().to_string());
        let idx = next_input_idx;
        next_input_idx += 1;
        Some(idx)
    } else {
        None
    };

    // Если оверлей Canvas не передан, подключаем логотип отдельным входом
    let logo_input_idx = if canvas_overlay_idx.is_none() {
        if let Some(ref l_path) = logo_image_file {
            ffmpeg_args.push("-i".to_string());
            ffmpeg_args.push(l_path.to_string_lossy().to_string());
            let idx = next_input_idx;
            next_input_idx += 1;
            Some(idx)
        } else {
            None
        }
    } else {
        None
    };

    // Вход аудио: кастомный звук либо тишина
    let audio_input_idx = if let Some(ref aud_path) = custom_audio_file {
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push(aud_path.to_string_lossy().to_string());
        let idx = next_input_idx;
        Some(idx)
    } else {
        None
    };

    ffmpeg_args.push("-t".to_string());
    ffmpeg_args.push(format!("{:.3}", duration));

    // Сборка цепочки видеоэффектов для заднего видео / изображения
    let mut bg_filters: Vec<String> = Vec::new();

    if using_video_bg && (item.bg_video_speed - 1.0).abs() > 0.05 {
        let spd = item.bg_video_speed.clamp(0.5, 2.0);
        bg_filters.push(format!("setpts=PTS/{:.2}", spd));
    }

    let zoom = item.bg_scale.clamp(1.0, 2.0);
    let scaled_w = ((target_w as f64 * zoom).round() as i32 / 2) * 2;
    let scaled_h = ((target_h as f64 * zoom).round() as i32 / 2) * 2;
    bg_filters.push(format!(
        "scale={}:{}:force_original_aspect_ratio=increase,crop={}:{}",
        scaled_w.max(target_w),
        scaled_h.max(target_h),
        target_w,
        target_h
    ));

    // Насыщенность, контраст и затемнение (dim -> brightness shift)
    let sat = item.bg_saturation.clamp(0.0, 2.0);
    let cont = item.bg_contrast.clamp(0.5, 1.5);
    let bright = -(item.bg_dim.clamp(0.0, 0.90) * 0.75);
    bg_filters.push(format!(
        "eq=saturation={:.2}:contrast={:.2}:brightness={:.2}",
        sat, cont, bright
    ));

    // Размытие фона (Blur)
    let blur_val = item.bg_blur.clamp(0.0, 40.0) as i32;
    if blur_val > 0 {
        let luma_r = blur_val.clamp(1, 40);
        let chroma_r = (blur_val / 2).clamp(1, 20);
        bg_filters.push(format!("boxblur={}:1:{}:1", luma_r, chroma_r));
    }

    if item.bg_vignette {
        bg_filters.push("vignette=PI/4".to_string());
    }

    let mut filter_chain = format!("[0:v]{}[bg];", bg_filters.join(","));
    let mut current_v = "bg".to_string();

    if let Some(ov_idx) = canvas_overlay_idx {
        // Наложение pixel-perfect прозрачного PNG оверлея из интерактивного превью
        let next_v = "with_canvas_ov".to_string();
        filter_chain.push_str(&format!(
            "[{}:v]scale={}:{}[ov];[{}][ov]overlay=0:0:format=auto[{}];",
            ov_idx, target_w, target_h, current_v, next_v
        ));
        current_v = next_v;
    } else {
        // Fallback: наложение логотипа и текста средствами FFmpeg
        if let Some(l_idx) = logo_input_idx {
            let scale_w = ((target_w as f64 * item.logo_scale.clamp(0.08, 1.5)) as i32).max(32);
            let pos_x = (target_w as f64 * (item.logo_x_percent / 100.0) - (scale_w as f64 / 2.0)) as i32;
            let pos_y = (target_h as f64 * (item.logo_y_percent / 100.0) - (scale_w as f64 / 2.0)) as i32;

            let next_v = "with_logo".to_string();
            filter_chain.push_str(&format!(
                "[{}:v]scale={}:-1[logo];[{}][logo]overlay=x={}:y={}[{}];",
                l_idx, scale_w, current_v, pos_x, pos_y, next_v
            ));
            current_v = next_v;
        }

        if !item.title.trim().is_empty() {
            let clean_t = item
                .title
                .replace('\\', "\\\\")
                .replace(':', "\\:")
                .replace('\'', "");
            let font_size = ((item.title_size as f64 * (target_w as f64 / 1080.0)) as i32).max(18);
            let pos_x = (target_w as f64 * (item.title_x_percent / 100.0)) as i32;
            let pos_y = (target_h as f64 * (item.title_y_percent / 100.0)) as i32;
            let next_v = "with_text".to_string();

            let box_part = if item.plate_style == "none" {
                format!(
                    "borderw={}:bordercolor={}",
                    item.outline_width.clamp(0, 8),
                    item.outline_color
                )
            } else {
                format!(
                    "box=1:boxcolor={}@{:.2}:boxborderw=14:borderw={}:bordercolor={}",
                    item.plate_color,
                    item.plate_opacity.clamp(0.1, 1.0),
                    item.outline_width.clamp(0, 6),
                    item.outline_color
                )
            };

            filter_chain.push_str(&format!(
                "[{}]drawtext=text='{}':fontsize={}:fontcolor={}:x={}-w/2:y={}-h/2:{}[{}];",
                current_v, clean_t, font_size, item.title_color, pos_x, pos_y, box_part, next_v
            ));
            current_v = next_v;
        }
    }

    // Плавное появление/затухание видеокадра при переходе fade_black
    if item.transition == "fade_black" && duration > 0.6 {
        let fade_out_st = (duration - 0.25).max(0.1);
        filter_chain.push_str(&format!(
            "[{}]fade=t=in:st=0:d=0.20,fade=t=out:st={:.3}:d=0.25,fps=30[outv]",
            current_v, fade_out_st
        ));
    } else {
        filter_chain.push_str(&format!("[{}]fps=30[outv]", current_v));
    }

    ffmpeg_args.push("-filter_complex".to_string());
    ffmpeg_args.push(filter_chain);
    ffmpeg_args.push("-map".to_string());
    ffmpeg_args.push("[outv]".to_string());

    if let Some(a_idx) = audio_input_idx {
        ffmpeg_args.push("-map".to_string());
        ffmpeg_args.push(format!("{}:a", a_idx));
        let vol = item.audio_volume.clamp(0.0, 2.0);
        let mut af_parts = vec![format!("volume={:.2}", vol)];
        if item.audio_fade_in {
            af_parts.push("afade=t=in:st=0:d=0.20".to_string());
        }
        if item.audio_fade_out && duration > 0.4 {
            af_parts.push(format!(
                "afade=t=out:st={:.3}:d=0.25",
                (duration - 0.25).max(0.05)
            ));
        }
        af_parts.push("aresample=48000".to_string());
        ffmpeg_args.push("-af".to_string());
        ffmpeg_args.push(af_parts.join(","));
    } else {
        ffmpeg_args.push("-f".to_string());
        ffmpeg_args.push("lavfi".to_string());
        ffmpeg_args.push("-i".to_string());
        ffmpeg_args.push("anullsrc=r=48000:cl=stereo".to_string());
        ffmpeg_args.push("-shortest".to_string());
    }

    ffmpeg_args.extend([
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
    ]);

    let sidecar_cmd = app_handle
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось запустить FFmpeg: {}", e)))?
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
        .args(ffmpeg_args);

    let (mut rx, child) = sidecar_cmd
        .spawn()
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка отрисовки заставки: {}", e)))?;

    while let Some(event) = rx.recv().await {
        if let CommandEvent::Terminated(_) = event {
            break;
        }
    }
    let _ = child.kill();

    Ok(())
}

// ============================================================================
// УПАКОВКА И РАСПАКОВКА ПОРТАТИВНОГО ZIP-АРХИВА ПРЕСЕТА (.vcpb / .zip)
// ============================================================================

fn crc32_compute(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Записывает набор именованных файлов `(имя_в_архиве, байты)` в стандартный ZIP-архив (метод STORE = 0)
pub fn build_zip_archive(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central_dir = Vec::new();

    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        let crc = crc32_compute(data);
        let size = data.len() as u32;
        let offset = out.len() as u32;

        // Local file header (0x04034b50)
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 flag
        out.extend_from_slice(&0u16.to_le_bytes()); // compression = 0 (Stored)
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        // Central directory header (0x02014b50)
        central_dir.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central_dir.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central_dir.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central_dir.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 flag
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // compression = 0
        central_dir.extend_from_slice(&0u16.to_le_bytes());
        central_dir.extend_from_slice(&0u16.to_le_bytes());
        central_dir.extend_from_slice(&crc.to_le_bytes());
        central_dir.extend_from_slice(&size.to_le_bytes());
        central_dir.extend_from_slice(&size.to_le_bytes());
        central_dir.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // extra
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // comment
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // disk
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // int attr
        central_dir.extend_from_slice(&0u32.to_le_bytes()); // ext attr
        central_dir.extend_from_slice(&offset.to_le_bytes());
        central_dir.extend_from_slice(name_bytes);
    }

    let cd_offset = out.len() as u32;
    let cd_size = central_dir.len() as u32;
    let count = entries.len() as u16;
    out.extend_from_slice(&central_dir);

    // End of central directory record (0x06054b50)
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    out
}

/// Читает файлы из стандартного ZIP-архива (метод STORE)
pub fn read_zip_archive(bytes: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut files = HashMap::new();
    let mut pos = 0usize;

    while pos + 30 <= bytes.len() {
        let sig = u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
        if sig != 0x0403_4b50 {
            break;
        }
        let compression = u16::from_le_bytes([bytes[pos + 8], bytes[pos + 9]]);
        let comp_size = u32::from_le_bytes([
            bytes[pos + 18],
            bytes[pos + 19],
            bytes[pos + 20],
            bytes[pos + 21],
        ]) as usize;
        let name_len = u16::from_le_bytes([bytes[pos + 26], bytes[pos + 27]]) as usize;
        let extra_len = u16::from_le_bytes([bytes[pos + 28], bytes[pos + 29]]) as usize;

        let name_start = pos + 30;
        let data_start = name_start + name_len + extra_len;
        if data_start + comp_size > bytes.len() {
            return Err("Поврежденный архив пресета (.vcpb)".to_string());
        }
        if compression != 0 {
            return Err("Поддерживаются только стандартные архивы пресетов (.vcpb STORE)".to_string());
        }
        let name = String::from_utf8_lossy(&bytes[name_start..name_start + name_len]).to_string();
        let data = bytes[data_start..data_start + comp_size].to_vec();
        files.insert(name, data);
        pos = data_start + comp_size;
    }

    if files.is_empty() {
        return Err("Архив пресета не содержит файлов".to_string());
    }
    Ok(files)
}

fn pack_item_media_into_zip(
    item: &mut BumperItemConfig,
    slot_tag: &str,
    entries: &mut Vec<(String, Vec<u8>)>,
) {
    let mut pack_field = |field: &mut Option<String>, kind: &str, default_ext: &str| {
        let Some(val) = field.clone() else { return; };
        let trimmed = val.trim();
        if trimmed.is_empty() {
            return;
        }
        if trimmed.starts_with("data:") {
            if let Some(bytes) = decode_base64_payload(trimmed) {
                let archive_name = format!("media/{}_{}.{}", slot_tag, kind, default_ext);
                entries.push((archive_name.clone(), bytes));
                *field = Some(format!("zip://{}", archive_name));
            }
        } else {
            let p = Path::new(trimmed);
            if p.exists() && p.is_file() {
                if let Ok(bytes) = std::fs::read(p) {
                    let ext = p
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or(default_ext);
                    let archive_name = format!("media/{}_{}.{}", slot_tag, kind, ext);
                    entries.push((archive_name.clone(), bytes));
                    *field = Some(format!("zip://{}", archive_name));
                }
            }
        }
    };

    pack_field(&mut item.custom_video_path, "custom_video", "mp4");
    pack_field(&mut item.custom_audio_path, "custom_audio", "mp3");
    pack_field(&mut item.bg_video_path, "bg_video", "mp4");
    pack_field(&mut item.bg_image_path, "bg_image", "png");
    pack_field(&mut item.logo_image_path, "logo_image", "png");
}

fn unpack_item_media_from_zip(
    item: &mut BumperItemConfig,
    files: &HashMap<String, Vec<u8>>,
    extract_dir: &Path,
) {
    let unpack_field = |field: &mut Option<String>| {
        let Some(val) = field.clone() else { return; };
        if let Some(rel) = val.strip_prefix("zip://") {
            if let Some(bytes) = files.get(rel) {
                let file_name = Path::new(rel)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("asset.bin");
                let dest = extract_dir.join(file_name);
                if std::fs::write(&dest, bytes).is_ok() {
                    *field = Some(dest.to_string_lossy().to_string());
                }
            }
        }
    };

    unpack_field(&mut item.custom_video_path);
    unpack_field(&mut item.custom_audio_path);
    unpack_field(&mut item.bg_video_path);
    unpack_field(&mut item.bg_image_path);
    unpack_field(&mut item.logo_image_path);
}

/// Экспортирует пресет вместе со всеми прикрепленными медиафайлами в единый ZIP-архив `.vcpb`
pub fn export_preset_to_vcpb_file(preset: &BumperPreset, dest_path: &Path) -> AppResult<String> {
    let mut cloned = preset.clone();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();

    pack_item_media_into_zip(&mut cloned.v916.preroll, "v916_preroll", &mut entries);
    pack_item_media_into_zip(&mut cloned.v916.postroll, "v916_postroll", &mut entries);
    pack_item_media_into_zip(&mut cloned.v169.preroll, "v169_preroll", &mut entries);
    pack_item_media_into_zip(&mut cloned.v169.postroll, "v169_postroll", &mut entries);

    let json_bytes = serde_json::to_vec_pretty(&cloned)
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка сериализации пресета: {}", e)))?;
    entries.insert(0, ("preset.json".to_string(), json_bytes));

    let zip_bytes = build_zip_archive(&entries);
    if let Some(parent) = dest_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(dest_path, zip_bytes).map_err(AppError::Io)?;
    Ok(dest_path.to_string_lossy().to_string())
}

/// Импортирует пресет из ZIP-архива `.vcpb` (или JSON-файла), распаковывая медиафайлы в локальную папку приложения
pub fn import_preset_from_vcpb_file(
    archive_path: &Path,
    app_data_dir: &Path,
) -> AppResult<BumperPreset> {
    let raw_bytes = std::fs::read(archive_path).map_err(AppError::Io)?;

    // Поддержка как бинарного ZIP-архива (.vcpb / .zip), так и текстового JSON
    if raw_bytes.starts_with(b"PK\x03\x04") {
        let files = read_zip_archive(&raw_bytes)
            .map_err(|e| AppError::Anyhow(anyhow::anyhow!(e)))?;
        let preset_bytes = files
            .get("preset.json")
            .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("В архиве .vcpb отсутствует preset.json")))?;
        let mut preset: BumperPreset = serde_json::from_slice(preset_bytes)
            .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Ошибка чтения preset.json: {}", e)))?;

        let new_id = format!(
            "preset_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        preset.id = new_id.clone();

        let extract_dir = app_data_dir.join("imported_presets").join(&new_id);
        let _ = std::fs::create_dir_all(&extract_dir);

        unpack_item_media_from_zip(&mut preset.v916.preroll, &files, &extract_dir);
        unpack_item_media_from_zip(&mut preset.v916.postroll, &files, &extract_dir);
        unpack_item_media_from_zip(&mut preset.v169.preroll, &files, &extract_dir);
        unpack_item_media_from_zip(&mut preset.v169.postroll, &files, &extract_dir);

        Ok(preset)
    } else {
        let mut preset: BumperPreset = serde_json::from_slice(&raw_bytes)
            .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Некорректный формат пресета: {}", e)))?;
        preset.id = format!(
            "preset_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        Ok(preset)
    }
}
