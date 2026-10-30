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

/// Вырезает и сдвигает по времени субтитры для конкретного клипа `[start_sec, start_sec + duration_sec]`.
/// Поскольку FFmpeg при быстром поиске `-ss <start_sec> -i <video>` сбрасывает таймкоды выходного потока в `0.0 .. duration_sec`,
/// индивидуальный `.srt` со сдвинутыми таймкодами гарантирует 100% точную синхронизацию речи и титров в каждом клипе!
async fn create_clip_srt(
    full_srt_path: &Path,
    start_sec: f64,
    duration_sec: f64,
) -> Option<PathBuf> {
    let content = tokio::fs::read_to_string(full_srt_path).await.ok()?;
    let all_fragments = crate::text_analyzer::parse_srt(&content);
    let end_sec = start_sec + duration_sec;

    let mut clip_cues = String::new();
    let mut cue_idx = 1usize;

    for frag in all_fragments {
        if frag.end <= start_sec + 0.05 || frag.start >= end_sec - 0.05 {
            continue;
        }
        if crate::whisper::is_whisper_hallucination(&frag.text) {
            continue;
        }

        let rel_start = (frag.start - start_sec).max(0.0);
        let rel_end = (frag.end - start_sec).min(duration_sec);
        if rel_end - rel_start < 0.15 {
            continue;
        }

        clip_cues.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            cue_idx,
            crate::whisper::format_srt_timestamp(rel_start),
            crate::whisper::format_srt_timestamp(rel_end),
            frag.text.trim()
        ));
        cue_idx += 1;
    }

    if cue_idx == 1 {
        return None;
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_srt = std::env::temp_dir().join(format!("vc_clip_sub_{}_{}.srt", std::process::id(), ts));
    tokio::fs::write(&temp_srt, clip_cues).await.ok()?;
    Some(temp_srt)
}

/// Создает один клип из исходного видео с нарезкой, субтитрами и фильтрами.
pub async fn render_clip(
    video_path: PathBuf,
    start_sec: f64,
    duration_sec: f64,
    output_path: PathBuf,
    srt_path: Option<PathBuf>,
    config: RenderConfig,
    app_handle: AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<()> {
    log::info!("Запуск рендеринга клипа: {}", output_path.display());

    // Гарантируем существование родительской папки для сохранения
    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
    }
    
    let shell = app_handle.shell();
    
    // Формируем аргументы для FFmpeg
    let mut args = vec![
        "-hide_banner".to_string(),
        "-nostdin".to_string(),
        "-y".to_string(),
        "-ss".to_string(), format!("{:.3}", start_sec),
        "-i".to_string(), video_path.to_string_lossy().to_string(),
        "-t".to_string(), format!("{:.3}", duration_sec),
    ];

    let mut vf_filters = vec![];
    
    // Преобразование в вертикальный формат 9:16 (Shorts / Reels / TikTok) с умным центрированием лица персонажа
    if config.vertical_format {
        let sample_time = start_sec + (duration_sec / 2.0).min(2.0);
        let center_ratio = detect_character_center_ratio(&video_path, sample_time, &app_handle).await;
        crate::logger::emit_log(
            &app_handle,
            "Render",
            "info",
            &format!("Smart Face Centering: фокус персонажа определен в {:.1}% ширины кадра", center_ratio * 100.0),
        );
        // Формула сдвига окна 9:16 так, чтобы персонаж находился строго по центру вертикального клипа
        let crop_filter = format!(
            "crop=w='ih*(9/16)':h='ih':x='min(max(0, iw*{:.3} - (ih*(9/16))/2), iw-(ih*(9/16)))':y=0,scale=1080:1920:flags=lanczos",
            center_ratio
        );
        vf_filters.push(crop_filter);
    }

    // Формируем индивидуальный синхронизированный SRT-файл для таймкода данного клипа [start_sec .. start_sec + duration_sec]
    let mut temp_clip_srt: Option<PathBuf> = None;
    if let Some(ref srt) = srt_path {
        if srt.exists() {
            temp_clip_srt = create_clip_srt(srt, start_sec, duration_sec).await;
        }
    }

    if let Some(ref clip_srt) = temp_clip_srt {
        let srt_escaped = clip_srt
            .to_string_lossy()
            .replace('\\', "/")
            .replace(':', "\\:")
            .replace('\'', "\\'")
            .replace('[', "\\[")
            .replace(']', "\\]");
        
        let mut sub_filter = format!("subtitles='{}'", srt_escaped);
        
        if let Some(style) = config.subtitle_style {
            // Заменяем веб-шрифт Inter на системный Arial, чтобы libass не тратил время на поиск fallback-шрифтов
            let font = if style.font_name.is_empty() || style.font_name.eq_ignore_ascii_case("Inter") {
                "Arial".to_string()
            } else {
                style.font_name
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

    crate::logger::emit_log(
        &app_handle,
        "Render",
        "info",
        &format!("Рендеринг клипа: [{:.2}с - {:.2}с] -> '{}' (вертикально={}, субтитры={})", start_sec, start_sec + duration_sec, output_path.display(), config.vertical_format, temp_clip_srt.is_some()),
    );

    let command = match shell.sidecar("ffmpeg") {
        Ok(cmd) => cmd
            .env("PATH", crate::logger::build_sidecar_env_path(&app_handle))
            .args(args),
        Err(e) => {
            if let Some(ref tmp) = temp_clip_srt {
                let _ = tokio::fs::remove_file(tmp).await;
            }
            let err = format!("FFmpeg (render) не обнаружен: {}", e);
            crate::logger::emit_log(&app_handle, "Render", "error", &err);
            return Err(AppError::Pipeline(err));
        }
    };

    let (mut rx, child) = match command.spawn() {
        Ok(pair) => pair,
        Err(e) => {
            if let Some(ref tmp) = temp_clip_srt {
                let _ = tokio::fs::remove_file(tmp).await;
            }
            let err = format!("Не удалось запустить FFmpeg при рендере: {}", e);
            crate::logger::emit_log(&app_handle, "Render", "error", &err);
            return Err(AppError::Pipeline(err));
        }
    };

    let app_handle_clone = app_handle.clone();
    let out_display = output_path.display().to_string();
    let render_future = async move {
        let _ = app_handle_clone.emit("render-progress", RenderProgressPayload { 
            percent: 0.0, 
            stage: "rendering started".to_string() 
        });

        // Чтение stderr, где ffmpeg отдает прогресс и статус кодирования
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stderr(bytes) => {
                    crate::logger::emit_stream_lines(&app_handle_clone, "Render", "debug", &bytes);
                    let log_line = String::from_utf8_lossy(&bytes);
                    if let Some(pos) = log_line.rfind("time=") {
                        let time_part = &log_line[pos + 5..];
                        let space_pos = time_part.find(|c: char| c.is_whitespace()).unwrap_or(time_part.len());
                        let time_str = &time_part[..space_pos];
                        let parts: Vec<&str> = time_str.trim().split(':').collect();
                        if parts.len() == 3 {
                            if let (Ok(h), Ok(m), Ok(s)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                                let cur_secs = h * 3600.0 + m * 60.0 + s;
                                if duration_sec > 0.0 {
                                    let pct = (cur_secs / duration_sec * 100.0).min(99.0).max(0.0);
                                    let _ = app_handle_clone.emit("render-progress", RenderProgressPayload { 
                                        percent: pct, 
                                        stage: format!("{:.1}с / {:.1}с", cur_secs, duration_sec) 
                                    });
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
                        let _ = app_handle_clone.emit("render-progress", RenderProgressPayload { 
                            percent: 100.0, 
                            stage: "rendering complete".to_string() 
                        });
                        return Ok(());
                    } else {
                        let err_msg = format!("Рендер завершился с ошибкой, код: {:?}", payload.code);
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

    // Ожидаем окончания или отмены
    let res = tokio::select! {
        r = render_future => r,
        _ = cancel_rx.recv() => {
            let _ = child.kill();
            Err(AppError::Pipeline("Отрисовка отменена пользователем.".to_string()))
        }
    };

    if let Some(ref tmp) = temp_clip_srt {
        let _ = tokio::fs::remove_file(tmp).await;
    }

    res
}

/// Вычисляет коэффициент горизонтального положения лица/персонажа (0.0 .. 1.0) для центрирования 9:16
async fn detect_character_center_ratio(
    video_path: &PathBuf,
    sample_time_sec: f64,
    app_handle: &AppHandle,
) -> f64 {
    let shell = app_handle.shell();
    let Ok(ffmpeg) = shell.sidecar("ffmpeg") else {
        return 0.5;
    };

    let temp_frame = std::env::temp_dir().join(format!("crop_detect_{}_{}.bmp", std::process::id(), (sample_time_sec * 100.0) as u64));

    // Извлекаем один уменьшенный кадр BMP для мгновенного анализа пикселей
    let cmd = ffmpeg
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
        .args([
            "-nostdin",
            "-y",
            "-ss", &format!("{:.3}", sample_time_sec),
            "-i", &video_path.to_string_lossy(),
            "-vframes", "1",
            "-s", "320x180",
            "-pix_fmt", "bgr24",
            &temp_frame.to_string_lossy(),
        ]);

    let Ok((mut rx, child)) = cmd.spawn() else {
        return 0.5;
    };

    let wait_frame = async {
        while let Some(event) = rx.recv().await {
            if let CommandEvent::Terminated(_) = event {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), wait_frame).await;
    let _ = child.kill();

    if !temp_frame.exists() {
        return 0.5;
    }

    let Ok(bytes) = tokio::fs::read(&temp_frame).await else {
        let _ = tokio::fs::remove_file(&temp_frame).await;
        return 0.5;
    };
    let _ = tokio::fs::remove_file(&temp_frame).await;

    // В BMP заголовке смещение пикселей по смещению 0x0A (10)
    if bytes.len() < 54 {
        return 0.5;
    }
    let pixel_offset = if bytes.len() >= 14 {
        u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize
    } else {
        54
    };

    let width = 320usize;
    let height = 180usize;
    let row_size = ((width * 3 + 3) / 4) * 4; // выравнивание 4 байта

    let mut col_weights = vec![0.0f64; width];
    let pixel_data = &bytes[pixel_offset.min(bytes.len())..];

    for y in 0..height {
        let row_start = y * row_size;
        if row_start + width * 3 > pixel_data.len() {
            break;
        }
        // В BMP строки хранятся снизу вверх: y = height - 1 — верхний край кадра (0.0), y = 0 — нижний край (1.0)
        let y_from_top = (height - 1 - y) as f64 / height as f64;
        // Зона лиц и голов персонажей в кадре 16:9: от 10% до 62% сверху
        let y_mult = if y_from_top >= 0.10 && y_from_top <= 0.62 {
            2.8
        } else if y_from_top > 0.62 && y_from_top <= 0.80 {
            1.0
        } else {
            0.15
        };

        let mut prev_luma = 0.0f64;
        for x in 0..width {
            let b = pixel_data[row_start + x * 3] as f64;
            let g = pixel_data[row_start + x * 3 + 1] as f64;
            let r = pixel_data[row_start + x * 3 + 2] as f64;

            let max_c = r.max(g).max(b);
            let min_c = r.min(g).min(b);
            let sat = max_c - min_c;
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            let edge_diff = if x > 0 { (luma - prev_luma).abs() } else { 0.0 };
            prev_luma = luma;

            // 1. Реалистичные тона кожи (R > G > B)
            let is_real_skin = r > 95.0 && g > 40.0 && b > 20.0
                && sat > 15.0
                && (r - g) > 12.0
                && r > g && g >= b;

            // 2. Светлые тона кожи в аниме/анимации (высокая яркость, теплый персиково-розовый оттенок)
            let is_anime_skin = r > 175.0 && g > 135.0 && b > 115.0
                && r >= g && g >= b
                && (r - b) >= 12.0 && (r - b) <= 65.0
                && (r - g) <= 45.0;

            if is_real_skin || is_anime_skin {
                col_weights[x] += (4.5 + (edge_diff * 0.08).min(2.5)) * y_mult;
            } else if edge_diff > 22.0 && sat > 25.0 && luma > 35.0 && luma < 240.0 {
                // Четкие контуры глаз/волос/персонажа на переднем плане (без учета плоского фона)
                col_weights[x] += 1.2 * y_mult;
            }
        }
    }

    let total_weight: f64 = col_weights.iter().sum();
    if total_weight <= 5.0 {
        return 0.5;
    }

    // Поиск оптимального окна кадрирования 9:16 (ширина окна = 180 * 9 / 16 = 101 пиксель из 320)
    // Метод скользящего окна находит реальную позицию персонажа (например, 28% или 72%), не размывая её фоном до 50%
    let crop_win = ((height * 9) / 16).clamp(60, width);
    let half_win = crop_win / 2;
    let mut best_window_sum = -1.0f64;
    let mut best_center_x = width / 2;

    for center_x in half_win..=(width - half_win) {
        let start_x = center_x - half_win;
        let end_x = (start_x + crop_win).min(width);
        let win_sum: f64 = col_weights[start_x..end_x].iter().sum();
        if win_sum > best_window_sum {
            best_window_sum = win_sum;
            best_center_x = center_x;
        }
    }

    let window_ratio = (best_center_x as f64) / (width as f64);
    let mut center_of_mass = 0.0f64;
    for (x, &w) in col_weights.iter().enumerate() {
        center_of_mass += (x as f64) * w;
    }
    let com_ratio = (center_of_mass / total_weight) / (width as f64);

    // Комбинируем пик скользящего окна (70%) и центр масс значимых контуров (30%)
    let calculated_ratio = window_ratio * 0.70 + com_ratio * 0.30;
    calculated_ratio.clamp(0.18, 0.82)
}
