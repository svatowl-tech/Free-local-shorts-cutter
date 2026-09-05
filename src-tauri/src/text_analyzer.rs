// src-tauri/src/text_analyzer.rs

use crate::error::{AppError, AppResult};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tokio::sync::mpsc::Receiver;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleFragment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TextEngine {
    Ollama,
    LlamaCli,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextAnalysisConfig {
    pub engine: TextEngine,
    pub model_name: String,   // for Ollama: "qwen2.5:1.5b", for LlamaCli: path to ".gguf"
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
                fragments.push(current_fragment.clone());
                current_fragment.text.clear();
            }
            state = 0;
            continue;
        }

        match state {
            0 => {
                // Если строка число (индекс), переходим к парсингу времени
                state = 1;
            }
            1 => {
                // Парсинг времени SRT, формат: 00:00:01,000 --> 00:00:03,500
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

    // Если конец файла достигнут без пустой строки
    if state >= 2 {
        fragments.push(current_fragment);
    }

    fragments
}

fn parse_srt_time(time_str: &str) -> f64 {
    // Формат: HH:MM:SS,MMM
    let parts: Vec<&str> = time_str.replace(',', ".").split(':').collect();
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

/// Проверка доступности Ollama
pub async fn check_ollama_available(url: &str) -> bool {
    let ping_url = format!("{}/api/tags", url.trim_end_matches('/'));
    if let Ok(ping_client) = Client::builder().timeout(Duration::from_secs(2)).build() {
        if let Ok(resp) = ping_client.get(&ping_url).send().await {
            return resp.status().is_success();
        }
    }
    false
}

/// Асинхронная аналитика через LLM
pub async fn analyze_text_with_llm(
    fragments: &[SubtitleFragment],
    config: &TextAnalysisConfig,
    app_handle: &AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<Vec<f64>> {
    let mut active_engine = config.engine.clone();
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| AppError::Pipeline(format!("HTTP client error: {}", e)))?;

    // Автоопределение
    if active_engine == TextEngine::Auto {
        if check_ollama_available(&config.ollama_url).await {
            log::info!("Ollama обнаружена по адресу {}", config.ollama_url);
            active_engine = TextEngine::Ollama;
        } else {
            log::info!("Ollama недоступна по таймауту, используем llama-cli (локально)");
            active_engine = TextEngine::LlamaCli;
        }
    }

    let mut all_scores = Vec::new();
    let batch_size = 20;

    for chunk in fragments.chunks(batch_size) {
        // Проверка отмены пользователем
        if let Ok(_) = cancel_rx.try_recv() {
            log::info!("Анализ текста отменен пользователем");
            return Err(AppError::Pipeline("Отменено пользователем".into()));
        }

        let prompt = build_prompt(chunk);
        log::debug!("Отправка батча для анализа: {}", prompt);

        // Инкапсулируем логику работы с выбранным движком
        let result = match active_engine {
            TextEngine::Ollama => {
                match run_ollama(&client, config, &prompt).await {
                    Ok(res) => res,
                    Err(e) => {
                        log::error!("Ошибка анализа в Ollama: {}, используем fallback", e);
                        return Ok(fallback_for_all(fragments)); // Fallback, если двигатель умер
                    }
                }
            }
            TextEngine::LlamaCli => {
                match run_llama_cli(app_handle, config, &prompt).await {
                    Ok(res) => res,
                    Err(e) => {
                        log::error!("Ошибка запуска llama-cli: {}, используем fallback", e);
                        return Ok(fallback_for_all(fragments)); // Fallback
                    }
                }
            }
            TextEngine::Auto => unreachable!(),
        };

        log::debug!("Сырой ответ от LLM: {}", result);

        // Парсим ответ JSON и объединяем с общим результатом
        let batch_scores = match parse_json_array(&result) {
            Some(mut scores) => {
                // Если LLM вернула меньше оценок чем нужно, добавляем fallback-оценки
                if scores.len() < chunk.len() {
                    let missing = compute_text_scores_fallback(&chunk[scores.len()..]);
                    scores.extend(missing);
                }
                // Если больше, обрезаем
                scores.truncate(chunk.len());
                scores
            }
            None => {
                log::warn!("Не удалось распарсить JSON, используем запасную эвристику. Ответ: {}", result);
                compute_text_scores_fallback(chunk)
            }
        };

        all_scores.extend_from_slice(&batch_scores);
    }

    Ok(all_scores)
}

fn build_prompt(fragments: &[SubtitleFragment]) -> String {
    let mut lines = Vec::new();
    for (i, frag) in fragments.iter().enumerate() {
        lines.push(format!("{}. \"{}\"", i + 1, frag.text.replace("\n", " ")));
    }
    format!(
        "Проанализируй следующие реплики из видео. Оцени каждую по шкале от 0 до 1 \
        по критериям: юмор, сарказм, эмоциональная напряжённость, динамичность. \
        Ответь строго JSON-массивом чисел, например: [0.8, 0.3, 0.9]. Реплики:\n{}",
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

async fn run_llama_cli(app_handle: &AppHandle, config: &TextAnalysisConfig, prompt: &str) -> AppResult<String> {
    // Формат запуска: llama-cli -m <model_path> --prompt "<batched_prompt>" --json
    let sidecar_command = app_handle.shell()
        .sidecar("llama-cli")
        .map_err(|e| AppError::Pipeline(format!("Не удалось найти sidecar-команду llama-cli: {}", e)))?;

    log::info!("Запуск llama-cli с моделью: {}", config.model_name);

    let output = sidecar_command
        .args(["-m", &config.model_name, "--prompt", prompt, "--json"])
        .output()
        .await
        .map_err(|e| AppError::Pipeline(format!("Ошибка выполнения llama-cli sidecar: {}", e)))?;

    if !output.status.success() {
        let err_text = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Pipeline(format!("llama-cli завершился с ошибкой: {}", err_text)));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_json_array(text: &str) -> Option<Vec<f64>> {
    // Простая попытка вырезать JSON массив из смешенного текстового ответа
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    
    if start <= end {
        let json_str = &text[start..=end];
        serde_json::from_str(json_str).ok()
    } else {
        None
    }
}

pub fn compute_text_scores_fallback(fragments: &[SubtitleFragment]) -> Vec<f64> {
    fragments.iter().map(|f| {
        let txt = f.text.to_lowercase();
        let mut score = 0.1;
        if txt.contains('!') { score += 0.2; }
        if txt.contains("ха-ха") || txt.contains("haha") || txt.contains("ахах") { score += 0.3; }
        if txt.contains('?') { score += 0.1; }
        if txt.len() > 10 { score += 0.1; }
        score.min(1.0)
    }).collect()
}

fn fallback_for_all(fragments: &[SubtitleFragment]) -> Vec<f64> {
    compute_text_scores_fallback(fragments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_srt() {
        let srt = "1\n00:00:01,000 --> 00:00:03,500\nПривет, как дела?\nВсё отлично!\n\n2\n00:00:03,600 --> 00:00:05,000\nПока.";
        let fragments = parse_srt(srt);
        
        assert_eq!(fragments.len(), 2);
        
        assert_eq!(fragments[0].start, 1.0);
        assert_eq!(fragments[0].end, 3.5);
        assert_eq!(fragments[0].text, "Привет, как дела? Всё отлично!");
        
        assert_eq!(fragments[1].start, 3.6);
        assert_eq!(fragments[1].end, 5.0);
        assert_eq!(fragments[1].text, "Пока.");
    }
}
