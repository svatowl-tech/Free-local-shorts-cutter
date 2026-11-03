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
            selected.push(crate::text_analyzer::SubtitleFragment {
                start: rel_start,
                end: rel_end,
                text: frag.text.trim().to_string(),
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
    
    // Формируем аргументы для FFmpeg (-accurate_seek + -avoid_negative_ts make_zero для идеальной синхронизации аудио, видео и фильтра subtitles)
    let mut args = vec![
        "-hide_banner".to_string(),
        "-nostdin".to_string(),
        "-y".to_string(),
        "-accurate_seek".to_string(),
        "-ss".to_string(), format!("{:.3}", start_sec),
        "-i".to_string(), video_path.to_string_lossy().to_string(),
        "-t".to_string(), format!("{:.3}", duration_sec),
        "-avoid_negative_ts".to_string(), "make_zero".to_string(),
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
struct FaceCandidate {
    center_ratio: f64,
    base_score: f64,
    speaker_activity: f64,
    scale_weight: f64,
}

#[derive(Clone, Debug)]
struct FrameFocusSample {
    rel_time: f64,
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
        return (default_expr, "центр кадра 50.0% (дефолт)".to_string());
    }

    let multi_face_frames = samples.iter().filter(|s| s.candidates.len() >= 2).count();
    let max_faces_seen = samples.iter().map(|s| s.candidates.len()).max().unwrap_or(1);

    let segments = segment_and_smooth_camera_track(&samples, duration_sec);
    if segments.is_empty() {
        return (default_expr, "центр кадра 50.0% (дефолт)".to_string());
    }

    let multi_face_note = if multi_face_frames >= 2 {
        format!(", анти-дребезг: до {} лиц в кадре", max_faces_seen)
    } else {
        String::new()
    };

    let is_single_static = segments.len() == 1
        && (segments[0].start_ratio - segments[0].end_ratio).abs() < 0.025;

    if is_single_static {
        let r = segments[0].start_ratio.clamp(0.16, 0.84);
        let expr = format!(
            "min(max(0, iw*{:.4} - (ih*(9/16))/2), iw-(ih*(9/16)))",
            r
        );
        let summary = format!(
            "стабильный фокус на лице {:.1}% ширины кадра (1 ракурс{})",
            r * 100.0,
            multi_face_note
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
        "динамическое слежение за лицами ({} ракурс(а){}: [{}])",
        segments.len(),
        multi_face_note,
        seg_descriptions.join(", ")
    );

    (full_expr, summary)
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

            let head_y_prior = if (0.10..=0.56).contains(&y_norm) {
                1.35
            } else if (0.06..=0.68).contains(&y_norm) {
                0.85
            } else {
                0.25
            };

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
                let edge_guard = if (0.08..=0.92).contains(&center_gx) {
                    1.0
                } else {
                    0.65
                };

                let score = face_score * edge_guard;
                if score > 0.18 {
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
            (raw_cx * 0.60 + (num / den) * 0.40).clamp(0.16, 0.84)
        } else {
            raw_cx.clamp(0.16, 0.84)
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
        // В аниме и живой речи открывание/закрывание рта дает локальный перепад яркости ~3..18 единиц в ячейке
        let speaker_activity = ((avg_mouth_motion - 1.2).max(0.0) / 10.5).clamp(0.0, 1.0);

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
                .clamp(0.16, 0.84);
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
        let group_ratio = (num / den.max(0.01)).clamp(0.16, 0.84);
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
                * (1.0 + cand.speaker_activity * 1.65);

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
    let primary_cluster_x = clusters[0].0;

    // 3. Если в ракурсе есть 2 конкурирующих разнесенных лица (например левый собеседник ~28% и правый ~72%)
    // и ракурс длится >= 3.5с (7+ сэмплов), проверяем, нет ли устойчивой смены говорящего между первой и второй половиной ракурса
    if clusters.len() >= 2 && slice.len() >= 7 {
        let secondary_cluster_x = clusters[1].0;
        let cluster_dist = (primary_cluster_x - secondary_cluster_x).abs();

        if cluster_dist >= 0.20 && clusters[1].1 >= clusters[0].1 * 0.45 && clusters[1].2 >= 3 {
            // Для каждого кадра ракурса определяем, кто из двух персонажей доминирует/говорит (0 = первый, 1 = второй)
            let mut frame_winner: Vec<usize> = Vec::with_capacity(slice.len());
            let mut current_speaker = 0_usize;

            for (f_idx, s) in slice.iter().enumerate() {
                let mut score_0 = 0.0_f64;
                let mut score_1 = 0.0_f64;

                for cand in &s.candidates {
                    let s_val = cand.base_score * (1.0 + cand.speaker_activity * 1.95);
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
                        start_ratio: target_a.clamp(0.16, 0.84),
                        end_ratio: target_a.clamp(0.16, 0.84),
                    },
                    CameraSegment {
                        start_t: split_t,
                        end_t,
                        start_ratio: target_b.clamp(0.16, 0.84),
                        end_ratio: target_b.clamp(0.16, 0.84),
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
            let w = (matched.base_score * (1.0 + matched.speaker_activity * 1.2)).max(0.10);
            running_x = running_x * 0.65 + matched.center_ratio * 0.35;
            track_points.push((s.rel_time, matched.center_ratio, w));
        }
    }

    if track_points.is_empty() {
        let r = primary_cluster_x.clamp(0.16, 0.84);
        return vec![CameraSegment {
            start_t,
            end_t,
            start_ratio: r,
            end_ratio: r,
        }];
    }

    let weighted_num: f64 = track_points.iter().map(|(_, r, w)| r * w).sum();
    let weighted_den: f64 = track_points.iter().map(|(_, _, w)| *w).sum();
    let mean_ratio = (weighted_num / weighted_den.max(0.01)).clamp(0.16, 0.84);

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
                start_ratio: avg_first.clamp(0.16, 0.84),
                end_ratio: avg_second.clamp(0.16, 0.84),
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
                        prev.end_ratio = blended.clamp(0.16, 0.84);
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

    let mut expr = seg_expr(segments.last().unwrap());
    for seg in segments[..segments.len() - 1].iter().rev() {
        expr = format!("if(lt(t,{:.2}),{},{})", seg.end_t, seg_expr(seg), expr);
    }
    expr
}

