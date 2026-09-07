// src-tauri/src/ffmpeg.rs
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandEvent;
use serde::Serialize;
use crate::error::{AppResult, AppError};
use tokio::sync::broadcast::Receiver;

#[derive(Serialize, Clone, Debug)]
pub struct FfmpegProgressPayload {
    pub current_time: f64,
    pub duration: f64,
    pub percent: f64,
}

fn parse_time_to_seconds(time_str: &str) -> Option<f64> {
    let parts: Vec<&str> = time_str.trim().split(':').collect();
    if parts.len() != 3 { return None; }
    let hours = parts[0].parse::<f64>().ok()?;
    let minutes = parts[1].parse::<f64>().ok()?;
    let seconds = parts[2].parse::<f64>().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

pub async fn extract_audio(
    video_path: PathBuf,
    output_audio_path: PathBuf,
    app_handle: AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<()> {
    let shell = app_handle.shell();
    let command = shell.sidecar("ffmpeg")
        .map_err(|e| AppError::Pipeline(format!("FFmpeg не обнаружен в ресурсах sidecar: {}. Убедитесь, что ресурсы настроены в tauri.conf.json", e)))?;

    let command = command.args([
        "-y", "-i", &video_path.to_string_lossy(),
        "-vn", "-acodec", "pcm_s16le", "-ar", "16000", "-ac", "1",
        &output_audio_path.to_string_lossy(),
    ]);

    let (mut rx, child) = command.spawn()
        .map_err(|e| AppError::Pipeline(format!("Не удалось запустить процесс FFmpeg: {}", e)))?;

    let progress_future = async move {
        let mut total_duration: Option<f64> = None;
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stderr(bytes) => {
                    let log_line = String::from_utf8_lossy(&bytes);
                    if total_duration.is_none() {
                        if let Some(pos) = log_line.find("Duration: ") {
                            let duration_part = &log_line[pos + 10..];
                            if let Some(end_pos) = duration_part.find(',') {
                                if let Some(secs) = parse_time_to_seconds(&duration_part[..end_pos]) {
                                    total_duration = Some(secs);
                                }
                            }
                        }
                    }
                    if let Some(pos) = log_line.find("time=") {
                        let time_part = &log_line[pos + 5..];
                        let space_pos = time_part.find(' ').unwrap_or(time_part.len());
                        if let Some(current_secs) = parse_time_to_seconds(&time_part[..space_pos]) {
                            if let Some(duration_secs) = total_duration {
                                let percent = (current_secs / duration_secs * 100.0).min(100.0).max(0.0);
                                let _ = app_handle.emit("ffmpeg-progress", FfmpegProgressPayload {
                                    current_time: current_secs,
                                    duration: duration_secs,
                                    percent,
                                });
                            }
                        }
                    }
                }
                CommandEvent::Terminated(payload) => {
                    if payload.code == Some(0) { return Ok(()); } 
                    else { return Err(AppError::Pipeline(format!("FFmpeg завершился с кодом: {:?}", payload.code))); }
                }
                _ => {}
            }
        }
        Err(AppError::Pipeline("Процесс конвертации неожиданно прервался.".to_string()))
    };

    tokio::select! {
        result = progress_future => result,
        _ = cancel_rx.recv() => {
            let _ = child.kill();
            Err(AppError::Pipeline("Нарезка аудио прервана пользователем.".to_string()))
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct VideoMetadata {
    pub duration: f64,
    pub codec: Option<String>,
}

pub async fn probe_video(
    video_path: PathBuf,
    app_handle: AppHandle,
) -> AppResult<VideoMetadata> {
    let shell = app_handle.shell();
    let command = shell.sidecar("ffmpeg")
        .map_err(|e| AppError::Pipeline(format!("FFmpeg sidecar не найден: {}", e)))?;

    let command = command.args([
        "-i", &video_path.to_string_lossy(),
        "-hide_banner",
    ]);

    let (mut rx, _child) = command.spawn()
        .map_err(|e| AppError::Pipeline(format!("Не удалось запустить FFmpeg для анализа: {}", e)))?;

    let mut total_duration = 0.0;
    let mut codec = None;

    while let Some(event) = rx.recv().await {
        if let CommandEvent::Stderr(bytes) = event {
            let log_line = String::from_utf8_lossy(&bytes);
            if total_duration == 0.0 {
                if let Some(pos) = log_line.find("Duration: ") {
                    let duration_part = &log_line[pos + 10..];
                    if let Some(end_pos) = duration_part.find(',') {
                        if let Some(secs) = parse_time_to_seconds(&duration_part[..end_pos]) {
                            total_duration = secs;
                        }
                    }
                }
            }
            if codec.is_none() {
                if let Some(pos) = log_line.find("Video: ") {
                    let video_part = &log_line[pos + 7..];
                    if let Some(end_pos) = video_part.find(',') {
                        codec = Some(video_part[..end_pos].trim().to_string());
                    }
                }
            }
        }
    }

    Ok(VideoMetadata {
        duration: total_duration,
        codec,
    })
}
