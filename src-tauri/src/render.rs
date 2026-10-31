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
    
    // Преобразование в вертикальный формат 9:16 (Shorts / Reels / TikTok) с динамическим ведением лиц внутри клипа
    if config.vertical_format {
        let (crop_x_expr, tracking_summary) =
            build_dynamic_face_crop_expr(&video_path, start_sec, duration_sec, &app_handle).await;
        crate::logger::emit_log(
            &app_handle,
            "Render",
            "info",
            &format!("Smart Face Tracking: {}", tracking_summary),
        );
        // Формула динамического сдвига окна 9:16 во времени t: следует за персонажем и переключается при смене ракурса внутри клипа
        let crop_filter = format!(
            "crop=w='ih*(9/16)':h='ih':x='{}':y=0,scale=1080:1920:flags=lanczos",
            crop_x_expr
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

#[derive(Clone, Debug)]
struct FrameFocusSample {
    rel_time: f64,
    center_ratio: f64,
    confidence: f64,
    hist: [f64; 12],
}

#[derive(Clone, Debug)]
struct CameraSegment {
    start_t: f64,
    end_t: f64,
    start_ratio: f64,
    end_ratio: f64,
}

/// Smart Face & Character Tracking v2 (динамическое внутриклиповое слежение за лицами и переключение ракурсов):
/// 1) За один быстрый проход FFmpeg извлекает серию кадров 240x135 с шагом 0.5 сек (2 FPS) по всей длине клипа.
/// 2) В каждом кадре 2D-детектор ищет компактные кластеры головы/лица (реалистичная кожа + аниме-кожа + микроконтраст глаз/волос + силуэт персонажа),
///    отсекая плоские фоновые засветы, небо и взрывы.
/// 3) При резкой смене ракурса/камеры внутри клипа мгновенно переключает окно 9:16 на новое лицо,
///    а при плавном движении персонажа — плавно ведет виртуальную камеру вслед за ним.
async fn build_dynamic_face_crop_expr(
    video_path: &PathBuf,
    start_sec: f64,
    duration_sec: f64,
    app_handle: &AppHandle,
) -> (String, String) {
    let default_expr = "min(max(0, iw*0.500 - (ih*(9/16))/2), iw-(ih*(9/16)))".to_string();
    let shell = app_handle.shell();
    let Ok(ffmpeg) = shell.sidecar("ffmpeg") else {
        return (default_expr, "центр кадра 50.0% (дефолт)".to_string());
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
    let max_frames = (scan_dur * sample_fps).ceil() as usize + 2;

    for idx in 1..=max_frames {
        let frame_path = temp_dir.join(format!("frame_{:03}.bmp", idx));
        let Ok(bytes) = tokio::fs::read(&frame_path).await else {
            break;
        };
        let rel_time = ((idx - 1) as f64) / sample_fps;
        if let Some(sample) = analyze_bmp_frame_2d(&bytes, 240, 135, rel_time) {
            samples.push(sample);
        }
    }

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    if samples.is_empty() {
        return (default_expr, "центр кадра 50.0% (дефолт)".to_string());
    }

    let segments = segment_and_smooth_camera_track(&samples, duration_sec);
    if segments.is_empty() {
        return (default_expr, "центр кадра 50.0% (дефолт)".to_string());
    }

    let is_single_static = segments.len() == 1
        && (segments[0].start_ratio - segments[0].end_ratio).abs() < 0.025;

    if is_single_static {
        let r = segments[0].start_ratio.clamp(0.16, 0.84);
        let expr = format!(
            "min(max(0, iw*{:.4} - (ih*(9/16))/2), iw-(ih*(9/16)))",
            r
        );
        let summary = format!(
            "стабильный фокус на лице {:.1}% ширины кадра (1 ракурс)",
            r * 100.0
        );
        return (expr, summary);
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
        "динамическое слежение за лицами ({} ракурс(а): [{}])",
        segments.len(),
        seg_descriptions.join(", ")
    );

    (full_expr, summary)
}

/// 2D-анализатор кадра 240x135 (BMP BGR24):
/// Вместо простого суммирования столбцов строит 2D-сетку (48x27 ячеек по 5x5 пикселей) и ищет
/// компактную область головы/лица, где одновременно присутствуют:
/// - тон кожи (реалистичный YCbCr/RGB или аниме-персиковый/слоновая кость);
/// - внутренний микроконтраст глаз/бровей/ресниц/рта и контур волос вокруг головы;
/// - изоляция головы от фона (чтобы широкие стены, небо и взрывы не перетягивали фокус).
fn analyze_bmp_frame_2d(
    bytes: &[u8],
    w: usize,
    h: usize,
    rel_time: f64,
) -> Option<FrameFocusSample> {
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

    for gy in 1..(gh - 1) {
        let py0 = gy * ch;
        let y_norm = (py0 as f64) / (h as f64);
        if !(0.05..=0.82).contains(&y_norm) {
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
            let skin_frac = skin_pixels / cell_area;
            let local_contrast = luma_max - luma_min;
            let avg_edge = edge_sum / cell_area;

            let head_y_prior = if (0.10..=0.56).contains(&y_norm) {
                1.35
            } else if (0.06..=0.68).contains(&y_norm) {
                0.85
            } else {
                0.25
            };

            let cell_idx = gy * gw + gx;
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

    // Если весь кадр залит теплым цветом (взрыв, закат, пыль), штрафуем глобальный цвет кожи и опираемся на черты глаз/контуры
    let total_skin_cells = grid_skin.iter().filter(|&&v| v > 0.35).count() as f64;
    let global_skin_flood_penalty = if total_skin_cells > (gw * gh) as f64 * 0.42 {
        0.18
    } else if total_skin_cells > (gw * gh) as f64 * 0.26 {
        0.55
    } else {
        1.0
    };

    // 3. Многомасштабный 2D-поиск головы персонажа (крупный план 9x8, средний план 6x6, общий план 4x5 ячеек)
    let box_scales: [(usize, usize); 3] = [(9, 8), (6, 6), (4, 5)];
    let mut best_face_x_ratio = 0.5_f64;
    let mut best_face_score = 0.0_f64;

    for &(bw, bh) in &box_scales {
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

                // Проверяем окружение слева и справа от рамки головы (настоящее лицо ограничено прической/фоном, а не тянется на всю стену)
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

                let face_score = if skin_density > 0.13 && (eye_density > 0.07 || edge_density > 0.11) {
                    (skin_density * 2.5 + isolation_bonus * 1.9)
                        * (1.0 + eye_density * 2.3 + edge_density * 0.9)
                } else {
                    edge_density * 0.55 + eye_density * 0.70
                };

                let center_gx = (gx as f64 + (bw as f64) * 0.5) / (gw as f64);
                let edge_guard = if (0.08..=0.92).contains(&center_gx) {
                    1.0
                } else {
                    0.65
                };

                let score = face_score * edge_guard;
                if score > best_face_score {
                    best_face_score = score;
                    best_face_x_ratio = center_gx;
                }
            }
        }
    }

    // 4. Уточняем центр головы внутри найденного окна шириной ±14% кадра
    let half_span = 0.14_f64;
    let mut num = 0.0_f64;
    let mut den = 0.0_f64;
    for gx in 1..(gw - 1) {
        let x_r = (gx as f64 + 0.5) / (gw as f64);
        if (x_r - best_face_x_ratio).abs() <= half_span {
            let mut col_w = 0.0_f64;
            for gy in 2..(gh * 65 / 100) {
                let idx = gy * gw + gx;
                col_w += grid_skin[idx] * global_skin_flood_penalty * 2.0
                    + grid_eye[idx] * 2.6
                    + grid_edge[idx] * 0.6;
            }
            num += x_r * col_w;
            den += col_w;
        }
    }

    let refined_ratio = if den > 0.1 {
        (best_face_x_ratio * 0.55 + (num / den) * 0.45).clamp(0.16, 0.84)
    } else {
        best_face_x_ratio.clamp(0.16, 0.84)
    };

    let confidence = (best_face_score / 1.2).clamp(0.05, 1.0);

    Some(FrameFocusSample {
        rel_time,
        center_ratio: refined_ratio,
        confidence,
        hist,
    })
}

/// Разбивает временную шкалу клипа на ракурсы (сцены между склейками камеры),
/// стабилизирует позицию внутри каждого ракурса и строит плавное панорамирование при движении персонажа.
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

        let pos_jump = (curr.center_ratio - prev.center_ratio).abs();
        let both_confident = prev.confidence > 0.20 && curr.confidence > 0.20;

        // Детектируем смену ракурса камеры ИЛИ перескок фокуса на другого говорящего персонажа
        let is_camera_cut = hist_l1 > 0.30
            || (hist_l1 > 0.17 && pos_jump > 0.11)
            || (both_confident && pos_jump > 0.15);

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

        if end_t <= start_t + 0.05 {
            continue;
        }

        let mut weighted_num = 0.0_f64;
        let mut weighted_den = 0.0_f64;
        for s in slice {
            let w = s.confidence.max(0.08);
            weighted_num += s.center_ratio * w;
            weighted_den += w;
        }
        let mean_ratio = if weighted_den > 0.0 {
            (weighted_num / weighted_den).clamp(0.16, 0.84)
        } else {
            0.5
        };

        // Если ракурс длится >= 2 секунд (4+ сэмпла) и персонаж плавно смещается по кадру — строим плавный панорамный трекинг
        if slice.len() >= 4 {
            let mid = slice.len() / 2;
            let first_half = &slice[..mid];
            let second_half = &slice[mid..];

            let avg_first = first_half
                .iter()
                .map(|s| s.center_ratio * s.confidence)
                .sum::<f64>()
                / first_half
                    .iter()
                    .map(|s| s.confidence)
                    .sum::<f64>()
                    .max(0.01);
            let avg_second = second_half
                .iter()
                .map(|s| s.center_ratio * s.confidence)
                .sum::<f64>()
                / second_half
                    .iter()
                    .map(|s| s.confidence)
                    .sum::<f64>()
                    .max(0.01);

            if (avg_second - avg_first).abs() > 0.085 {
                raw_segments.push(CameraSegment {
                    start_t,
                    end_t,
                    start_ratio: avg_first.clamp(0.16, 0.84),
                    end_ratio: avg_second.clamp(0.16, 0.84),
                });
                continue;
            }
        }

        raw_segments.push(CameraSegment {
            start_t,
            end_t,
            start_ratio: mean_ratio,
            end_ratio: mean_ratio,
        });
    }

    // Поглощаем одиночные 0.5-секундные вспышки взрывов и объединяем соседние сегменты с близкой позицией (< 5.5% разницы)
    if raw_segments.len() >= 2 {
        let mut cleaned: Vec<CameraSegment> = Vec::with_capacity(raw_segments.len());
        for seg in raw_segments {
            let seg_dur = seg.end_t - seg.start_t;
            if let Some(prev) = cleaned.last_mut() {
                let close_to_prev = (seg.start_ratio - prev.end_ratio).abs() < 0.055
                    && (seg.end_ratio - prev.end_ratio).abs() < 0.055;
                if close_to_prev || seg_dur < 0.60 {
                    prev.end_t = seg.end_t;
                    continue;
                }
            }
            cleaned.push(seg);
        }
        raw_segments = cleaned;
    }

    if raw_segments.len() > 8 {
        raw_segments.truncate(8);
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

    let mut expr = seg_expr(segments.last().unwrap());
    for seg in segments[..segments.len() - 1].iter().rev() {
        expr = format!("if(lt(t,{:.2}),{},{})", seg.end_t, seg_expr(seg), expr);
    }
    expr
}

