// src-tauri/src/text_analyzer.rs

use crate::error::{AppError, AppResult};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tokio::sync::broadcast::Receiver;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleFragment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TextEngine {
    Builtin,
    LlamaCli,
    Ollama,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextAnalysisConfig {
    pub engine: TextEngine,
    pub model_name: String,   // for LlamaCli: path to ".gguf" or name, for Ollama: "qwen2.5:1.5b"
    pub ollama_url: String,   // e.g. "http://localhost:11434"
}

/// Функция парсинга SRT
pub fn parse_srt(content: &str) -> Vec<SubtitleFragment> {
    let mut fragments = Vec::new();
    let mut current_fragment = SubtitleFragment {
        start: 0.0,
        end: 0.0,
        text: String::new(),
    };
    let mut state = 0; // 0: index, 1: time, 2: text, 3: text ongoing

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            if state >= 2 {
                if !current_fragment.text.trim().is_empty() {
                    fragments.push(current_fragment.clone());
                }
                current_fragment.text.clear();
            }
            state = 0;
            continue;
        }

        match state {
            0 => {
                state = 1;
            }
            1 => {
                // Парсинг времени SRT: 00:00:01,000 --> 00:00:03,500
                if let Some((start_str, end_str)) = line.split_once("-->") {
                    current_fragment.start = parse_srt_time(start_str.trim());
                    current_fragment.end = parse_srt_time(end_str.trim());
                    state = 2;
                }
            }
            _ => {
                if state == 2 {
                    current_fragment.text = line.to_string();
                    state = 3;
                } else {
                    current_fragment.text.push(' ');
                    current_fragment.text.push_str(line);
                }
            }
        }
    }

    if state >= 2 && !current_fragment.text.trim().is_empty() {
        fragments.push(current_fragment);
    }

    fragments
}

fn parse_srt_time(time_str: &str) -> f64 {
    let normalized = time_str.replace(',', ".");
    let parts: Vec<&str> = normalized.split(':').collect();
    if parts.len() == 3 {
        let h: f64 = parts[0].parse().unwrap_or(0.0);
        let m: f64 = parts[1].parse().unwrap_or(0.0);
        let s: f64 = parts[2].parse().unwrap_or(0.0);
        h * 3600.0 + m * 60.0 + s
    } else {
        0.0
    }
}

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

/// Асинхронная аналитика текста реплик
pub async fn analyze_text_with_llm(
    fragments: &[SubtitleFragment],
    config: &TextAnalysisConfig,
    app_handle: &AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<Vec<f64>> {
    if fragments.is_empty() {
        return Ok(Vec::new());
    }

    // Если выбран Builtin или Auto — используем быстрый умный локальный семантический анализатор
    if config.engine == TextEngine::Builtin {
        crate::logger::emit_log(
            app_handle,
            "LLM",
            "info",
            &format!("Применяется высокоскоростной встроенный автономный NLP-движок анализа реплик (всего сегментов: {})", fragments.len()),
        );
        return Ok(compute_text_scores_advanced(fragments));
    }

    if config.engine == TextEngine::LlamaCli {
        crate::logger::emit_log(
            app_handle,
            "LLM",
            "info",
            &format!("Поиск весов модели для llama-cli: '{}'...", config.model_name),
        );
        if let Some(model_path) = crate::models::resolve_model_path(app_handle, &config.model_name) {
            crate::logger::emit_log(
                app_handle,
                "LLM",
                "info",
                &format!("Запуск локального llama-cli с моделью: '{}'", model_path.display()),
            );
            let mut all_scores = Vec::new();
            let batch_size = 15;
            let total_batches = (fragments.len() + batch_size - 1) / batch_size;

            for (batch_idx, chunk) in fragments.chunks(batch_size).enumerate() {
                if cancel_rx.try_recv().is_ok() {
                    crate::logger::emit_log(app_handle, "LLM", "warn", "Анализ отменен пользователем.");
                    return Err(AppError::Pipeline("Отменено пользователем".into()));
                }

                crate::logger::emit_log(
                    app_handle,
                    "LLM",
                    "info",
                    &format!("Пакет {}/{}: анализ {} реплик через llama-cli...", batch_idx + 1, total_batches, chunk.len()),
                );

                let prompt = build_prompt(chunk);
                match run_llama_cli(app_handle, &model_path.to_string_lossy(), &prompt, cancel_rx.resubscribe()).await {
                    Ok(result) => {
                        let parsed = parse_json_array(&result);
                        if let Some(ref batch_scores) = parsed {
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "info",
                                &format!("Успешно получен JSON-ответ от llama-cli: {} оценок", batch_scores.len()),
                            );
                            all_scores.extend_from_slice(batch_scores);
                        } else {
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "warn",
                                "Не удалось распарсить JSON массив от llama-cli. Применяется встроенный NLP расчет для пакета.",
                            );
                            all_scores.extend_from_slice(&compute_text_scores_advanced(chunk));
                        }
                    }
                    Err(e) => {
                        crate::logger::emit_log(
                            app_handle,
                            "LLM",
                            "warn",
                            &format!("llama-cli завершился с ошибкой: {}. Переключение на встроенный NLP для данного пакета.", e),
                        );
                        all_scores.extend_from_slice(&compute_text_scores_advanced(chunk));
                    }
                }
            }
            return Ok(all_scores);
        } else {
            crate::logger::emit_log(
                app_handle,
                "LLM",
                "warn",
                &format!("Модель для llama-cli '{}' не найдена. Применяется встроенный NLP-движок.", config.model_name),
            );
            return Ok(compute_text_scores_advanced(fragments));
        }
    }

    if config.engine == TextEngine::Ollama {
        crate::logger::emit_log(
            app_handle,
            "LLM",
            "info",
            &format!("Подключение к Ollama API: {} (модель: {})", config.ollama_url, config.model_name),
        );

        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        let mut all_scores = Vec::new();
        let batch_size = 20;
        let total_batches = (fragments.len() + batch_size - 1) / batch_size;

        for (batch_idx, chunk) in fragments.chunks(batch_size).enumerate() {
            if cancel_rx.try_recv().is_ok() {
                crate::logger::emit_log(app_handle, "LLM", "warn", "Анализ отменен пользователем.");
                return Err(AppError::Pipeline("Отменено пользователем".into()));
            }

            crate::logger::emit_log(
                app_handle,
                "LLM",
                "info",
                &format!("Пакет {}/{}: отправка {} реплик в Ollama...", batch_idx + 1, total_batches, chunk.len()),
            );

            let prompt = build_prompt(chunk);
            match run_ollama(&client, config, &prompt, app_handle).await {
                Ok(result) => {
                    let parsed = parse_json_array(&result);
                    if let Some(ref batch_scores) = parsed {
                        crate::logger::emit_log(
                            app_handle,
                            "LLM",
                            "info",
                            &format!("Успешный ответ от Ollama: получено {} оценок", batch_scores.len()),
                        );
                        all_scores.extend_from_slice(batch_scores);
                    } else {
                        crate::logger::emit_log(
                            app_handle,
                            "LLM",
                            "warn",
                            "Ollama вернула невалидный JSON. Применяется встроенный NLP анализ для пакета.",
                        );
                        all_scores.extend_from_slice(&compute_text_scores_advanced(chunk));
                    }
                }
                Err(e) => {
                    crate::logger::emit_log(
                        app_handle,
                        "LLM",
                        "warn",
                        &format!("Ollama недоступна ({})! Переключение на встроенный NLP анализ для пакета.", e),
                    );
                    all_scores.extend_from_slice(&compute_text_scores_advanced(chunk));
                }
            }
        }
        return Ok(all_scores);
    }

    crate::logger::emit_log(app_handle, "LLM", "info", "Расчет семантических оценок через встроенный NLP завершен.");
    Ok(compute_text_scores_advanced(fragments))
}

fn build_prompt(fragments: &[SubtitleFragment]) -> String {
    let mut lines = Vec::new();
    for (i, frag) in fragments.iter().enumerate() {
        lines.push(format!("{}. \"{}\"", i + 1, frag.text.replace(['\n', '\r'], " ")));
    }
    let input_list = lines.join("\n");
    format!(
        "<|im_start|>system\n\
        Ты профессиональный видеомонтажер и аналитик вовлекающего контента. \
        Твоя задача — оценить каждую реплику по шкале от 0.0 до 1.0 на предмет виральности, эмоций, юмора, панчлайна или кульминации.\n\
        ПРАВИЛО: Выведи ИСКЛЮЧИТЕЛЬНО валидный JSON-массив из {} чисел. Без пояснений, без вводных слов. Пример: [0.85, 0.20]\n\
        <|im_end|>\n\
        <|im_start|>user\n\
        Оцени следующие реплики:\n{}\n\
        <|im_end|>\n\
        <|im_start|>assistant\n[",
        fragments.len(),
        input_list
    )
}

async fn run_ollama(client: &Client, config: &TextAnalysisConfig, prompt: &str, app_handle: &AppHandle) -> AppResult<String> {
    let url = format!("{}/api/generate", config.ollama_url.trim_end_matches('/'));
    
    let req_body = OllamaRequest {
        model: config.model_name.clone(),
        prompt: prompt.to_string(),
        stream: false,
    };

    crate::logger::emit_log(
        app_handle,
        "LLM",
        "debug",
        &format!("Ollama POST {} (длина промпта: {} символов)", url, prompt.len()),
    );

    let resp = client.post(&url)
        .json(&req_body)
        .send()
        .await
        .map_err(|e| {
            let err_msg = format!("Ошибка POST-запроса Ollama: {}", e);
            crate::logger::emit_log(app_handle, "LLM", "error", &err_msg);
            AppError::Pipeline(err_msg)
        })?;

    if !resp.status().is_success() {
        let err_msg = format!("Неуспешный HTTP-статус от Ollama: {}", resp.status());
        crate::logger::emit_log(app_handle, "LLM", "error", &err_msg);
        return Err(AppError::Pipeline(err_msg));
    }

    let parsed: OllamaResponse = resp.json().await
        .map_err(|e| {
            let err_msg = format!("Не удалось распарсить JSON-ответ от Ollama: {}", e);
            crate::logger::emit_log(app_handle, "LLM", "error", &err_msg);
            AppError::Pipeline(err_msg)
        })?;

    crate::logger::emit_log(
        app_handle,
        "LLM",
        "debug",
        &format!("Ollama ответ получен (длина: {} символов)", parsed.response.len()),
    );

    Ok(parsed.response)
}

async fn run_llama_cli(
    app_handle: &AppHandle, 
    model_path: &str, 
    prompt: &str,
    mut cancel_rx: Receiver<()>,
) -> AppResult<String> {
    use tauri_plugin_shell::process::CommandEvent;

    let sidecar_command = app_handle.shell()
        .sidecar("llama-cli")
        .map_err(|e| {
            let err = format!("llama-cli не найден в sidecar ресурсах: {}", e);
            crate::logger::emit_log(app_handle, "LLM", "error", &err);
            AppError::Pipeline(err)
        })?;

    let cmd = sidecar_command
        .args([
            "-m", model_path,
            "-p", prompt,
            "-n", "512",
            "--temp", "0.2",
            "--no-display-prompt"
        ]);

    let (mut rx, child) = cmd.spawn()
        .map_err(|e| {
            let err = format!("Не удалось запустить процесс llama-cli: {}", e);
            crate::logger::emit_log(app_handle, "LLM", "error", &err);
            AppError::Pipeline(err)
        })?;

    let app_handle_clone = app_handle.clone();
    let run_future = async move {
        let mut stdout_accum = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    crate::logger::emit_stream_lines(&app_handle_clone, "LLM", "debug", &bytes);
                    stdout_accum.push_str(&String::from_utf8_lossy(&bytes));
                }
                CommandEvent::Stderr(bytes) => {
                    crate::logger::emit_stream_lines(&app_handle_clone, "LLM", "debug", &bytes);
                }
                CommandEvent::Terminated(payload) => {
                    if payload.code == Some(0) || !stdout_accum.trim().is_empty() {
                        crate::logger::emit_log(
                            &app_handle_clone,
                            "LLM",
                            "info",
                            &format!("llama-cli завершился успешно (код {:?})", payload.code),
                        );
                        return Ok(stdout_accum);
                    } else {
                        let err_msg = format!("llama-cli завершился с ошибкой: {:?}", payload.code);
                        crate::logger::emit_log(&app_handle_clone, "LLM", "error", &err_msg);
                        return Err(AppError::Pipeline(err_msg));
                    }
                }
                _ => {}
            }
        }
        if !stdout_accum.trim().is_empty() {
            Ok(stdout_accum)
        } else {
            let err_msg = "Процесс llama-cli неожиданно прервался.".to_string();
            crate::logger::emit_log(&app_handle_clone, "LLM", "error", &err_msg);
            Err(AppError::Pipeline(err_msg))
        }
    };

    tokio::select! {
        res = run_future => res,
        _ = cancel_rx.recv() => {
            let _ = child.kill();
            Err(AppError::Pipeline("Анализ llama-cli отменен пользователем.".to_string()))
        }
    }
}

fn parse_json_array(text: &str) -> Option<Vec<f64>> {
    let cleaned = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    // 1. Полноценный JSON-массив в квадратных скобках
    if let (Some(start), Some(end)) = (cleaned.find('['), cleaned.rfind(']')) {
        if start <= end {
            let json_str = &cleaned[start..=end];
            if let Ok(nums) = serde_json::from_str::<Vec<f64>>(json_str) {
                return Some(nums);
            }
        }
    }

    // 2. Если ответ начинается с массива, но закрывающая скобка отсутствует из-за обрезки токенов
    if cleaned.starts_with('[') && !cleaned.ends_with(']') {
        let with_closing = format!("{}]", cleaned.trim_end().trim_end_matches(','));
        if let Ok(nums) = serde_json::from_str::<Vec<f64>>(&with_closing) {
            return Some(nums);
        }
    }

    // 3. Fallback: извлечение отдельных чисел через любые стандартные разделители
    let inner = cleaned
        .trim_start_matches('[')
        .trim_end_matches(']');
    let numbers: Vec<f64> = inner
        .split(|c: char| c == ',' || c == ';' || c == '\n' || c == ' ')
        .map(|s| s.trim().trim_matches(|c: char| !c.is_numeric() && c != '.'))
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f64>().ok())
        .map(|n| n.max(0.0).min(1.0))
        .collect();

    if !numbers.is_empty() {
        Some(numbers)
    } else {
        None
    }
}

/// Продвинутый локальный семантический анализатор диалогов и эмоциональных всплесков
pub fn compute_text_scores_advanced(fragments: &[SubtitleFragment]) -> Vec<f64> {
    // Ключевые слова хайлайтов для аниме, стримов, диалогов, подкастов и динамики
    let intense_keywords = [
        "ха-ха", "ахах", "haha", "omg", "лол", "lol", "lmao",
        "вау", "ого", "чё", "что?!", "жесть", "круто", "капец",
        "не может быть", "вперёд", "ура", "стоп", "быстрее",
        "бей", "беги", "смотри", "боже", "шок", "красава", "лул",
        "ужас", "победа", "атака", "удар", "сила", "смерть", "крик"
    ];

    fragments.iter().map(|f| {
        let txt = f.text.to_lowercase();
        let mut score: f64 = 0.45; // Базовый уровень значимости диалогов

        // 1. Восклицания и эмоциональные знаки
        if txt.contains('!') {
            score += 0.20;
        }
        if txt.contains("!!") || txt.contains("?!") {
            score += 0.15;
        }
        if txt.contains('?') {
            score += 0.10;
        }

        // 2. Поиск маркеров эмоций/смеха/хайлайтов
        for kw in &intense_keywords {
            if txt.contains(kw) {
                score += 0.25;
                break;
            }
        }

        // 3. Плотность речи (скорость произношения слов)
        let duration = (f.end - f.start).max(0.3);
        let word_count = txt.split_whitespace().count();
        let speech_rate = (word_count as f64) / duration;

        if speech_rate > 3.0 {
            // Быстрая напряженная речь
            score += 0.15;
        }

        // 4. Длина фразы
        if txt.len() > 20 {
            score += 0.10;
        }

        score.min(0.95).max(0.20)
    }).collect()
}

pub fn compute_text_scores_fallback(fragments: &[SubtitleFragment]) -> Vec<f64> {
    compute_text_scores_advanced(fragments)
}
