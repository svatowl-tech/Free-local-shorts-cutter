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
    crate::logger::emit_log(
        &app_handle,
        "Whisper",
        "info",
        &format!("Инициализация ASR для аудиофайла: '{}'", audio_path.display()),
    );
    
    // Пытаемся получить команду sidecar "whisper".
    let command = shell.sidecar("whisper")
        .map_err(|e| {
            let err_msg = format!("Бинарник whisper.cpp не найден в sidecar ресурсах. Проверьте настройки tauri.conf.json: {}", e);
            crate::logger::emit_log(&app_handle, "Whisper", "error", &err_msg);
            AppError::Pipeline(err_msg)
        })?;

    // Ищем модель в папке ресурсов или рядом с бинарником с помощью умного резолвера
    let model = match model_path {
        Some(p) => crate::models::resolve_model_path(&app_handle, &p.to_string_lossy()),
        None => None,
    }
    .or_else(|| crate::models::resolve_model_path(&app_handle, "ggml-base.bin"))
    .or_else(|| crate::models::resolve_model_path(&app_handle, "ggml-tiny.bin"))
    .or_else(|| crate::models::resolve_model_path(&app_handle, "models/ggml-base.bin"));
    
    let model = match model {
        Some(m) if m.exists() => {
            crate::logger::emit_log(
                &app_handle,
                "Whisper",
                "info",
                &format!("Найдена модель распознавания речи: '{}'", m.display()),
            );
            m
        },
        _ => {
            let err_msg = "Предустановленная модель Whisper (ggml-base.bin / ggml-tiny.bin) не найдена в папке models. Загрузите модель во вкладке Моделей.".to_string();
            crate::logger::emit_log(&app_handle, "Whisper", "error", &err_msg);
            return Err(AppError::Pipeline(err_msg));
        }
    };

    // Генерируем пути. Ожидается, что -of <base> сгенерирует <base>.srt
    let srt_base_name = audio_path.with_extension(""); 
    let base_out = srt_base_name.to_string_lossy().to_string();
    let expected_srt_output = PathBuf::from(format!("{}.srt", base_out));
    
    crate::logger::emit_log(
        &app_handle,
        "Whisper",
        "info",
        &format!("Запуск whisper.cpp (модель: {}, вывод: {}.srt)...", model.display(), base_out),
    );

    let command = command.args([
        "-m", &model.to_string_lossy(),
        "-osrt", 
        "-of", &base_out,
        &audio_path.to_string_lossy(),
    ]);

    let (mut rx, child) = command.spawn()
        .map_err(|e| {
            let err_msg = format!("Не удалось запустить процесс whisper.cpp: {}", e);
            crate::logger::emit_log(&app_handle, "Whisper", "error", &err_msg);
            AppError::Pipeline(err_msg)
        })?;

    let app_handle_clone = app_handle.clone();
    let progress_future = async move {
        let _ = app_handle_clone.emit("whisper-progress", WhisperProgressPayload { 
            percent: 0.0, 
            stage: "loading_model".to_string() 
        });

        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stderr(bytes) | CommandEvent::Stdout(bytes) => {
                    crate::logger::emit_stream_lines(&app_handle_clone, "Whisper", "debug", &bytes);
                    let log_line = String::from_utf8_lossy(&bytes);
                    
                    if log_line.contains("progress =") {
                        if let Some(mut percent_str) = log_line.split("progress =").nth(1) {
                            percent_str = percent_str.trim().trim_end_matches('%');
                            if let Ok(pct) = percent_str.parse::<f64>() {
                                let _ = app_handle_clone.emit("whisper-progress", WhisperProgressPayload { 
                                    percent: pct, 
                                    stage: "transcribing".to_string() 
                                });
                            }
                        }
                    }
                }
                CommandEvent::Terminated(payload) => {
                    if payload.code == Some(0) {
                        crate::logger::emit_log(
                            &app_handle_clone,
                            "Whisper",
                            "info",
                            &format!("Распознавание речи успешно завершено! Субтитры сохранены в '{}'", expected_srt_output.display()),
                        );
                        let _ = app_handle_clone.emit("whisper-progress", WhisperProgressPayload { 
                            percent: 100.0, 
                            stage: "saving".to_string() 
                        });
                        return Ok(expected_srt_output);
                    } else {
                        let err_msg = format!("whisper.cpp завершился с кодом ошибки: {:?}", payload.code);
                        crate::logger::emit_log(&app_handle_clone, "Whisper", "error", &err_msg);
                        return Err(AppError::Pipeline(err_msg));
                    }
                }
                _ => {}
            }
        }
        let err_msg = "Процесс whisper неожиданно прервался.".to_string();
        crate::logger::emit_log(&app_handle_clone, "Whisper", "error", &err_msg);
        Err(AppError::Pipeline(err_msg))
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
