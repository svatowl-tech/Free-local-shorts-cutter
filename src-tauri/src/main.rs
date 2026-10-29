// src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod error;
mod ffmpeg;
mod whisper;
mod analyzer;
mod text_analyzer;
mod render;
pub mod pipeline;
pub mod logger;
mod commands;
mod models;

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
        .plugin(tauri_plugin_fs::init())
        .manage(AppState { cancel_tx })
        .manage(models::init_models_state())
        .invoke_handler(tauri::generate_handler![
            commands::extract_audio,
            commands::transcribe_audio,
            commands::analyze_video,
            commands::start_pipeline,
            commands::render_clips_from_fragments,
            commands::generate_subtitles_only,
            commands::get_video_metadata,
            commands::cancel_pipeline,
            commands::open_folder,
            models::get_models_status,
            models::download_model,
            models::delete_model,
            models::cancel_model_download
        ])
        .setup(|app| {
            let augmented_path = logger::build_sidecar_env_path(app.handle());
            std::env::set_var("PATH", &augmented_path);

            // На Windows гарантируем наличие DLL рядом с бинарниками sidecar
            #[cfg(target_os = "windows")]
            {
                if let (Ok(exe_path), Ok(res_dir)) = (std::env::current_exe(), app.path().resource_dir()) {
                    if let Some(exe_dir) = exe_path.parent() {
                        for candidate_dir in [res_dir.clone(), res_dir.join("resources"), exe_dir.join("resources")] {
                            if let Ok(entries) = std::fs::read_dir(&candidate_dir) {
                                for entry in entries.flatten() {
                                    let p = entry.path();
                                    if p.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("dll")).unwrap_or(false) {
                                        if let Some(fname) = p.file_name() {
                                            if fname != "placeholder.dll" {
                                                let dest = exe_dir.join(fname);
                                                if !dest.exists() {
                                                    let _ = std::fs::copy(&p, &dest);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            log::info!("Приложение проинициализировано. Ожидание команд...");
            Ok(())
        })
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("Критическая ошибка работы Tauri: {}", e))?;

    Ok(())
}
