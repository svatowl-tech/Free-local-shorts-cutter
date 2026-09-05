// src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod error;
mod ffmpeg;
mod whisper;
mod analyzer;
mod text_analyzer;
mod render;
pub mod pipeline;
mod commands;
mod models;

use error::AppResult;
use tauri::Manager;

pub struct AppState {
    pub cancel_tx: tokio::sync::broadcast::Sender<()>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    
    log::info!("Запуск бэкенда видеорезака на Tauri 2 + Tokio рантайме...");

    const MAX_BROADCAST_RECEIVERS: usize = 16;
    let (cancel_tx, _) = tokio::sync::broadcast::channel(MAX_BROADCAST_RECEIVERS);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { cancel_tx })
        .manage(models::init_models_state())
        .invoke_handler(tauri::generate_handler![
            commands::extract_audio,
            commands::transcribe_audio,
            commands::analyze_video,
            commands::start_pipeline,
            commands::render_clips_from_fragments,
            commands::generate_subtitles_only,
            models::get_models_status,
            models::download_model
        ])
        .setup(|app| {
            log::info!("Приложение проинициализировано. Ожидание команд...");
            Ok(())
        })
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("Критическая ошибка работы Tauri: {}", e))?;

    Ok(())
}

