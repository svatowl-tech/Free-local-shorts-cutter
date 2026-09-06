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

    let command = shell.sidecar("ffmpeg")
        .map_err(|e| AppError::Pipeline(format!("FFmpeg (render) не обнаружен: {}", e)))?
        .args(args);

    let (mut rx, child) = command.spawn()
        .map_err(|e| AppError::Pipeline(format!("Не удалось запустить FFmpeg при рендере: {}", e)))?;

    let render_future = async {
        let _ = app_handle.emit("render-progress", RenderProgressPayload { 
            percent: 0.0, 
            stage: "rendering started".to_string() 
        });

        // Чтение stderr, где ffmpeg отдает прогресс 
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Terminated(payload) => {
                    if payload.code == Some(0) {
                        let _ = app_handle.emit("render-progress", RenderProgressPayload { 
                            percent: 100.0, 
                            stage: "rendering complete".to_string() 
                        });
                        return Ok(());
                    } else {
                        return Err(AppError::Pipeline(format!("Рендер завершился с ошибкой: {:?}", payload.code)));
                    }
                }
                _ => {}
            }
        }
        Err(AppError::Pipeline("Процесс рендера неожиданно прервался.".to_string()))
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
