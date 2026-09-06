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

    // Если выбран Builtin или Auto без явных внешних сервисов — используем быстрый умный локальный семантический анализатор
    if config.engine == TextEngine::Builtin || config.engine == TextEngine::Auto {
        log::info!("Применяется высокоскоростной встроенный автономный NLP-движок анализа реплик ({} сегментов)", fragments.len());
        return Ok(compute_text_scores_advanced(fragments));
    }

    if config.engine == TextEngine::LlamaCli {
        if let Some(model_path) = crate::models::resolve_model_path(app_handle, &config.model_name) {
            log::info!("Запуск локального llama-cli с моделью: {}", model_path.display());
            let mut all_scores = Vec::new();
            let batch_size = 15;

            for chunk in fragments.chunks(batch_size) {
                if cancel_rx.try_recv().is_ok() {
                    return Err(AppError::Pipeline("Отменено пользователем".into()));
                }

                let prompt = build_prompt(chunk);
                match run_llama_cli(app_handle, &model_path.to_string_lossy(), &prompt).await {
                    Ok(result) => {
                        let batch_scores = parse_json_array(&result).unwrap_or_else(|| compute_text_scores_advanced(chunk));
                        all_scores.extend_from_slice(&batch_scores);
                    }
                    Err(e) => {
                        log::warn!("llama-cli завершился с ошибкой: {}, переключаемся на встроенный NLP", e);
                        all_scores.extend_from_slice(&compute_text_scores_advanced(chunk));
                    }
                }
            }
            return Ok(all_scores);
        } else {
            log::warn!("Модель для llama-cli '{}' не найдена. Применяется встроенный NLP-движок.", config.model_name);
            return Ok(compute_text_scores_advanced(fragments));
        }
    }

    if config.engine == TextEngine::Ollama {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        let mut all_scores = Vec::new();
        let batch_size = 20;

        for chunk in fragments.chunks(batch_size) {
            if cancel_rx.try_recv().is_ok() {
                return Err(AppError::Pipeline("Отменено пользователем".into()));
            }

            let prompt = build_prompt(chunk);
            match run_ollama(&client, config, &prompt).await {
                Ok(result) => {
                    let batch_scores = parse_json_array(&result).unwrap_or_else(|| compute_text_scores_advanced(chunk));
                    all_scores.extend_from_slice(&batch_scores);
                }
                Err(e) => {
                    log::warn!("Ollama недоступна ({}). Мгновенное переключение на встроенный NLP анализ.", e);
                    all_scores.extend_from_slice(&compute_text_scores_advanced(chunk));
                }
            }
        }
        return Ok(all_scores);
    }

    Ok(compute_text_scores_advanced(fragments))
}

fn build_prompt(fragments: &[SubtitleFragment]) -> String {
    let mut lines = Vec::new();
    for (i, frag) in fragments.iter().enumerate() {
        lines.push(format!("{}. \"{}\"", i + 1, frag.text.replace('\n', " ")));
    }
    format!(
        "Оцени каждую реплику по шкале от 0.0 до 1.0 (юмор, эмоции, динамика). \
        Ответь строго JSON-массивом чисел: [0.8, 0.3]. Реплики:\n{}",
        lines.join("\n")
    )
}

async fn run_ollama(client: &Client, config: &TextAnalysisConfig, prompt: &str) -> AppResult<String> {
    let url = format!("{}/api/generate", config.ollama_url.trim_end_matches('/'));
    
    let req_body = OllamaRequest {
        model: config.model_name.clone(),
        prompt: prompt.to_string(),
        stream: false,
    };

    let resp = client.post(&url)
        .json(&req_body)
        .send()
        .await
        .map_err(|e| AppError::Pipeline(format!("Ошибка POST-запроса Ollama: {}", e)))?;

    if !resp.status().is_success() {
        return Err(AppError::Pipeline(format!("Неуспешный статус от Ollama: {}", resp.status())));
    }

    let parsed: OllamaResponse = resp.json().await
        .map_err(|e| AppError::Pipeline(format!("Не удалось прочитать JSON от Ollama: {}", e)))?;

    Ok(parsed.response)
}

async fn run_llama_cli(app_handle: &AppHandle, model_path: &str, prompt: &str) -> AppResult<String> {
    let sidecar_command = app_handle.shell()
        .sidecar("llama-cli")
        .map_err(|e| AppError::Pipeline(format!("llama-cli не найден: {}", e)))?;

    let output = sidecar_command
        .args(["-m", model_path, "--prompt", prompt, "--json"])
        .output()
        .await
        .map_err(|e| AppError::Pipeline(format!("Ошибка llama-cli: {}", e)))?;

    if !output.status.success() {
        let err_text = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Pipeline(format!("llama-cli ошибка: {}", err_text)));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_json_array(text: &str) -> Option<Vec<f64>> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    
    if start <= end {
        let json_str = &text[start..=end];
        serde_json::from_str(json_str).ok()
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
        let mut score: f64 = 0.25; // Базовый уровень речи

        // 1. Восклицания и эмоциональные знаки
        if txt.contains('!') {
            score += 0.25;
        }
        if txt.contains("!!") || txt.contains("?!") {
            score += 0.20;
        }
        if txt.contains('?') {
            score += 0.15;
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
            score += 0.20;
        }

        // 4. Длина фразы
        if txt.len() > 15 {
            score += 0.10;
        }

        score.min(1.0).max(0.0)
    }).collect()
}

pub fn compute_text_scores_fallback(fragments: &[SubtitleFragment]) -> Vec<f64> {
    compute_text_scores_advanced(fragments)
}
