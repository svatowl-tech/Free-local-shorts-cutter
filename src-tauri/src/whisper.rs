// src-tauri/src/whisper.rs
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandEvent;
use serde::Serialize;
use crate::error::{AppResult, AppError};
use tokio::sync::broadcast::Receiver;

#[derive(Serialize, Clone, Debug)]
pub struct WhisperProgressPayload {
    pub percent: f64,
    pub stage: String,
}

pub async fn transcribe_audio(
    audio_path: PathBuf,
    model_path: Option<PathBuf>,
    app_handle: AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<PathBuf> {
    let shell = app_handle.shell();
    
    // Пытаемся получить команду sidecar "whisper". Для этого файл должен лежать
    // в папке src-tauri/resources и называться, например, whisper-x86_64-pc-windows-msvc.exe
    let command = shell.sidecar("whisper")
        .map_err(|e| AppError::Pipeline(format!("Бинарник whisper.cpp не найден в sidecar ресурсах. Проверьте настройки tauri.conf.json и наличие файлов. Ошибка: {}", e)))?;

    // Ищем модель в папке ресурсов (внутри собранного приложения)
    let model = model_path.unwrap_or_else(|| {
        app_handle
            .path()
            .resource_dir()
            .unwrap_or_default()
            .join("models")
            .join("ggml-base.bin")
    });
    
    if !model.exists() {
        return Err(AppError::Pipeline(format!(
            "Модель не найдена по пути: {}. Скачайте модель (например, ggml-base.bin) и поместите её в папку src-tauri/models/",
            model.display()
        )));
    }

    // Генерируем пути. Ожидается, что -of <base> сгенерирует <base>.srt
    let srt_base_name = audio_path.with_extension(""); 
    let base_out = srt_base_name.to_string_lossy().to_string();
    let expected_srt_output = PathBuf::from(format!("{}.srt", base_out));
    
    // Формируем аргументы для whisper.cpp (или whisper-cli)
    // -m <model_file> -osrt -of <output_srt> <audio_path>
    let command = command.args([
        "-m", &model.to_string_lossy(),
        "-osrt", 
        "-of", &base_out,
        &audio_path.to_string_lossy(),
    ]);

    let (mut rx, child) = command.spawn()
        .map_err(|e| AppError::Pipeline(format!("Не удалось запустить процесс whisper.cpp: {}", e)))?;

    // Запускаем асинхронное чтение событий процесса
    let progress_future = async move {
        // Уведомление фронтенда о старте
        let _ = app_handle.emit("whisper-progress", WhisperProgressPayload { 
            percent: 0.0, 
            stage: "loading_model".to_string() 
        });

        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stderr(bytes) | CommandEvent::Stdout(bytes) => {
                    let log_line = String::from_utf8_lossy(&bytes);
                    
                    // whisper.cpp выводит прогресс примерно как "progress = 42%"
                    if log_line.contains("progress =") {
                        if let Some(mut percent_str) = log_line.split("progress =").nth(1) {
                            percent_str = percent_str.trim().trim_end_matches('%');
                            if let Ok(pct) = percent_str.parse::<f64>() {
                                let _ = app_handle.emit("whisper-progress", WhisperProgressPayload { 
                                    percent: pct, 
                                    stage: "transcribing".to_string() 
                                });
                            }
                        }
                    }
                }
                CommandEvent::Terminated(payload) => {
                    if payload.code == Some(0) {
                        let _ = app_handle.emit("whisper-progress", WhisperProgressPayload { 
                            percent: 100.0, 
                            stage: "saving".to_string() 
                        });
                        return Ok(expected_srt_output);
                    } else {
                        return Err(AppError::Pipeline(format!("whisper.cpp завершился с ошибкой: {:?}", payload.code)));
                    }
                }
                _ => {}
            }
        }
        Err(AppError::Pipeline("Процесс whisper неожиданно прервался.".to_string()))
    };

    // Ожидаем завершения или отмены
    tokio::select! {
        result = progress_future => result,
        _ = cancel_rx.recv() => {
            let _ = child.kill();
            Err(AppError::Pipeline("Распознавание отменено пользователем.".to_string()))
        }
    }
}
