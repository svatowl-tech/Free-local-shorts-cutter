// src-tauri/src/render.rs
use std::path::PathBuf;
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
        "-y".to_string(),
        "-ss".to_string(), format!("{:.3}", start_sec),
        "-i".to_string(), video_path.to_string_lossy().to_string(),
        "-t".to_string(), format!("{:.3}", duration_sec),
    ];

    let mut vf_filters = vec![];
    
    // Преобразование в вертикальный формат 9:16 (Shorts / Reels / TikTok)
    if config.vertical_format {
        vf_filters.push("crop=ih*(9/16):ih,scale=1080:1920:flags=lanczos".to_string());
    }

    // Наложение субтитров
    if let Some(srt) = srt_path {
        if srt.exists() {
            // Экранирование для фильтра FFmpeg
            let srt_escaped = srt
                .to_string_lossy()
                .replace('\\', "/")
                .replace(':', "\\:")
                .replace('\'', "\\'");
            
            let mut sub_filter = format!("subtitles='{}'", srt_escaped);
            
            if let Some(style) = config.subtitle_style {
                let font = if style.font_name.is_empty() { "Arial".to_string() } else { style.font_name };
                let font_size = if style.font_size <= 0 { 24 } else { style.font_size };
                let color = if style.color.is_empty() { "&H00FFFFFF".to_string() } else { style.color };
                sub_filter.push_str(&format!(
                    ":force_style='FontName={},FontSize={},PrimaryColour={}'", 
                    font, font_size, color
                ));
            }

            vf_filters.push(sub_filter);
        }
    }

    if !vf_filters.is_empty() {
        args.push("-vf".to_string());
        args.push(vf_filters.join(","));
    }

    // Кодеки видео и аудио (максимальная совместимость со всеми устройствами)
    args.push("-c:v".to_string());
    args.push("libx264".to_string());
    args.push("-preset".to_string());
    args.push("fast".to_string());
    args.push("-crf".to_string());
    args.push("22".to_string());
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
        &format!("Рендеринг клипа: [{:.2}с - {:.2}с] -> '{}' (вертикально={})", start_sec, start_sec + duration_sec, output_path.display(), config.vertical_format),
    );

    let command = shell.sidecar("ffmpeg")
        .map_err(|e| {
            let err = format!("FFmpeg (render) не обнаружен: {}", e);
            crate::logger::emit_log(&app_handle, "Render", "error", &err);
            AppError::Pipeline(err)
        })?
        .args(args);

    let (mut rx, child) = command.spawn()
        .map_err(|e| {
            let err = format!("Не удалось запустить FFmpeg при рендере: {}", e);
            crate::logger::emit_log(&app_handle, "Render", "error", &err);
            AppError::Pipeline(err)
        })?;

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
                    if let Some(pos) = log_line.find("time=") {
                        let time_part = &log_line[pos + 5..];
                        let space_pos = time_part.find(' ').unwrap_or(time_part.len());
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
    tokio::select! {
        res = render_future => res,
        _ = cancel_rx.recv() => {
            let _ = child.kill();
            Err(AppError::Pipeline("Отрисовка отменена пользователем.".to_string()))
        }
    }
}
