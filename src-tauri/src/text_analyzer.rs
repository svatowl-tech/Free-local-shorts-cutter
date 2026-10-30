// src-tauri/src/text_analyzer.rs

use crate::error::{AppError, AppResult};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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

/// Нормализует вектор оценок от LLM так, чтобы его длина строго совпадала с длиной пакета `chunk`
fn normalize_batch_scores(parsed: &[f64], chunk: &[SubtitleFragment]) -> Vec<f64> {
    let fallback = compute_text_scores_advanced(chunk);
    let mut out = Vec::with_capacity(chunk.len());
    for i in 0..chunk.len() {
        if let Some(&val) = parsed.get(i) {
            // Если фраза является явной галлюцинацией Whisper, принудительно занижаем вес
            if crate::whisper::is_whisper_hallucination(&chunk[i].text) {
                out.push(0.05);
            } else {
                out.push(val.clamp(0.05, 0.99));
            }
        } else {
            out.push(fallback[i]);
        }
    }
    out
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

    // Если выбран Builtin — используем быстрый умный локальный семантический анализатор
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
            &format!("Поиск локальной GGUF-модели для llama-cli (предпочтительная: '{}')...", config.model_name),
        );
        if let Some(model_path) = crate::models::resolve_any_llm_model(app_handle, Some(&config.model_name)) {
            crate::logger::emit_log(
                app_handle,
                "LLM",
                "info",
                &format!("Запуск локального llama-cli с моделью: '{}' (контекст: 2048, UTF-8 файл промпта)", model_path.display()),
            );
            let mut all_scores = Vec::with_capacity(fragments.len());
            let batch_size = 35;
            let total_batches = (fragments.len() + batch_size - 1) / batch_size;
            let mut use_gpu = true;
            let mut consecutive_parse_failures = 0usize;

            for (batch_idx, chunk) in fragments.chunks(batch_size).enumerate() {
                if cancel_rx.try_recv().is_ok() {
                    crate::logger::emit_log(app_handle, "LLM", "warn", "Анализ отменен пользователем.");
                    return Err(AppError::Pipeline("Отменено пользователем".into()));
                }

                crate::logger::emit_log(
                    app_handle,
                    "LLM",
                    "info",
                    &format!("Пакет {}/{}: анализ {} реплик через llama-cli (GPU offload={})...", batch_idx + 1, total_batches, chunk.len(), use_gpu),
                );

                let prompt = build_prompt(chunk);
                let mut batch_res = run_llama_cli(
                    app_handle,
                    &model_path.to_string_lossy(),
                    &prompt,
                    use_gpu,
                    cancel_rx.resubscribe(),
                ).await;

                // Если запуск с GPU offload (-ngl 99) упал ИЛИ вернул поврежденный вывод (например, 'GGGG...' из-за переполнения FP16 NaN в CUDA на RTX 20xx),
                // немедленно переключаемся на стабильный режим CPU (-ngl 0) и повторяем текущий пакет!
                let gpu_output_corrupted = match &batch_res {
                    Ok(out) => parse_json_array(out).is_none(),
                    Err(_) => true,
                };

                if gpu_output_corrupted && use_gpu {
                    crate::logger::emit_log(
                        app_handle,
                        "LLM",
                        "warn",
                        "Обнаружен сбой генерации на GPU (артефакты CUDA FP16 / невалидный вывод). Автоматическое переключение llama-cli в стабильный режим CPU (-ngl 0) и повтор пакета...",
                    );
                    use_gpu = false;
                    batch_res = run_llama_cli(
                        app_handle,
                        &model_path.to_string_lossy(),
                        &prompt,
                        false,
                        cancel_rx.resubscribe(),
                    ).await;
                }

                match batch_res {
                    Ok(result) => {
                        let parsed = parse_json_array(&result);
                        if let Some(ref batch_scores) = parsed {
                            consecutive_parse_failures = 0;
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "info",
                                &format!("✓ Пакет {}/{}: получено {} оценок от llama-cli", batch_idx + 1, total_batches, batch_scores.len()),
                            );
                            all_scores.extend_from_slice(&normalize_batch_scores(batch_scores, chunk));
                        } else {
                            consecutive_parse_failures += 1;
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "warn",
                                "Не удалось распарсить JSON массив от llama-cli. Применяется встроенный NLP расчет для пакета.",
                            );
                            all_scores.extend_from_slice(&compute_text_scores_advanced(chunk));

                            if consecutive_parse_failures >= 2 && batch_idx + 1 < total_batches {
                                crate::logger::emit_log(
                                    app_handle,
                                    "LLM",
                                    "warn",
                                    "Предохранитель LLM: повторный невалидный вывод модели. Автоматическое переключение оставшихся пакетов на скоростной встроенный NLP-движок без потери времени.",
                                );
                                let remaining = &fragments[(batch_idx + 1) * batch_size..];
                                all_scores.extend_from_slice(&compute_text_scores_advanced(remaining));
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        crate::logger::emit_log(
                            app_handle,
                            "LLM",
                            "warn",
                            &format!(
                                "llama-cli недоступен или завершился со сбоем ({}). Автоматическое переключение оставшихся пакетов на встроенный скоростной NLP-движок без потери времени.",
                                e
                            ),
                        );
                        // Circuit breaker: рассчитываем текущий и все оставшиеся фрагменты через встроенный NLP мгновенно,
                        // чтобы не спавнить десятки падающих процессов подряд!
                        let remaining = &fragments[batch_idx * batch_size..];
                        all_scores.extend_from_slice(&compute_text_scores_advanced(remaining));
                        break;
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

        let mut all_scores = Vec::with_capacity(fragments.len());
        let batch_size = 30;
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
                        all_scores.extend_from_slice(&normalize_batch_scores(batch_scores, chunk));
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
                        &format!("Ollama недоступна ({})! Переключение оставшихся пакетов на встроенный NLP анализ.", e),
                    );
                    let remaining = &fragments[batch_idx * batch_size..];
                    all_scores.extend_from_slice(&compute_text_scores_advanced(remaining));
                    break;
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
        let clean_txt = frag.text.replace(['\n', '\r', '"'], " ");
        lines.push(format!("{}. \"{}\"", i + 1, clean_txt.trim()));
    }
    let input_list = lines.join("\n");
    format!(
        "Instruction: Ты профессиональный видеомонтажер и аналитик вирусных роликов (Shorts, Reels, TikTok).\n\
Оцени каждую из {} реплик по шкале от 0.0 до 1.0 на предмет виральности, эмоций, драмы, юмора, панчлайна или кульминации.\n\
ПРАВИЛО: Выведи ТОЛЬКО валидный JSON-массив из {} чисел с плавающей точкой в том же порядке. Без текста и без пояснений.\n\
Пример ответа: [0.85, 0.30, 0.65]\n\n\
Реплики ({} шт.):\n{}\n\n\
Ответ (JSON-массив из {} чисел):\n[",
        fragments.len(),
        fragments.len(),
        fragments.len(),
        input_list,
        fragments.len()
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
    use_gpu: bool,
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

    // КРИТИЧЕСКОЕ ИСПРАВЛЕНИЕ ДЛЯ WINDOWS:
    // Передача кириллического промпта через аргумент командной строки `-p` на Windows приводит к конвертации
    // в однобайтовую кодировку CP1251 внутри MSVC CRT `argv`, из-за чего `llama_tokenize` (`unicode_cpt_from_utf8`)
    // выбрасывает исключение `invalid utf-8` и аварийно завершает процесс с кодом -1073740791 (0xC0000409) сразу после `system_info`!
    // Запись промпта в чистый UTF-8 файл и передача через `-f <path>` полностью решает проблему кодировки!
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let prompt_file = std::env::temp_dir().join(format!("vc_llama_prompt_{}_{}.txt", std::process::id(), ts));
    tokio::fs::write(&prompt_file, prompt.as_bytes()).await.map_err(AppError::Io)?;

    let ngl_val = if use_gpu { "99" } else { "0" };
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().clamp(2, 8))
        .unwrap_or(4)
        .to_string();

    // Передаем:
    // -f <prompt_file>: чтение UTF-8 промпта напрямую из файла (без искажения кириллицы в Windows argv)
    // -c 2048: ограничиваем контекст до 2048 вместо 32768 по умолчанию (экономит ~1.7 ГБ памяти и ускоряет старт в 15 раз!)
    // -ngl 99/0: выгружаем слои модели на GPU или CPU
    // -t <threads>: оптимальное число потоков CPU
    // -n 384: лимит генерируемых токенов для JSON-массива
    // --temp 0.1: детерминированный JSON-вывод
    // --repeat-penalty 1.15: защита от зацикливания одного токена
    // --no-display-prompt: выводим в stdout только сгенерированный ответ
    let cmd = sidecar_command
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
        .args([
            "-m", model_path,
            "-f", &prompt_file.to_string_lossy(),
            "-c", "2048",
            "-ngl", ngl_val,
            "-t", &threads,
            "-n", "384",
            "--temp", "0.1",
            "--repeat-penalty", "1.15",
            "--no-display-prompt",
        ]);

    let (mut rx, child) = match cmd.spawn() {
        Ok(pair) => pair,
        Err(e) => {
            let _ = tokio::fs::remove_file(&prompt_file).await;
            let err = format!("Не удалось запустить процесс llama-cli: {}", e);
            crate::logger::emit_log(app_handle, "LLM", "error", &err);
            return Err(AppError::Pipeline(err));
        }
    };

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

    let res = tokio::select! {
        r = tokio::time::timeout(Duration::from_secs(60), run_future) => {
            match r {
                Ok(inner) => inner,
                Err(_) => {
                    let _ = child.kill();
                    Err(AppError::Pipeline("Превышен таймаут ожидания ответа от llama-cli (60с)".to_string()))
                }
            }
        },
        _ = cancel_rx.recv() => {
            let _ = child.kill();
            Err(AppError::Pipeline("Анализ llama-cli отменен пользователем.".to_string()))
        }
    };

    let _ = tokio::fs::remove_file(&prompt_file).await;
    res
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
                if !nums.is_empty() {
                    return Some(nums);
                }
            }
        }
    }

    // 2. Если ответ начинается с продолжения массива (т.к. промпт заканчивается на `[`), добавляем открывающую скобку
    if !cleaned.starts_with('[') {
        if let Some(end) = cleaned.find(']') {
            let with_opening = format!("[{}", &cleaned[..=end]);
            if let Ok(nums) = serde_json::from_str::<Vec<f64>>(&with_opening) {
                if !nums.is_empty() {
                    return Some(nums);
                }
            }
        }
    }

    // 3. Если ответ начинается с массива, но закрывающая скобка отсутствует из-за обрезки токенов
    if cleaned.starts_with('[') && !cleaned.ends_with(']') {
        let with_closing = format!("{}]", cleaned.trim_end().trim_end_matches(','));
        if let Ok(nums) = serde_json::from_str::<Vec<f64>>(&with_closing) {
            if !nums.is_empty() {
                return Some(nums);
            }
        }
    }

    // 4. Fallback: извлечение отдельных чисел через стандартные разделители
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
        "ужас", "победа", "атака", "удар", "сила", "смерть", "крик",
        "войн", "приказ", "огонь", "враг", "преступлен", "капитуляц",
        "уничтож", "спасем", "кров", "биться", "штурм", "взрыв", "молодец"
    ];

    fragments.iter().map(|f| {
        if crate::whisper::is_whisper_hallucination(&f.text) {
            return 0.05;
        }

        let txt = f.text.trim().to_lowercase();
        let word_count = txt.split_whitespace().count();

        // Очень короткие однословные реплики («Нет.», «Да.», «Чай.») не должны становиться центром хайлайта сами по себе
        if word_count <= 1 && !txt.contains('!') {
            return 0.25;
        }

        let mut score: f64 = 0.42;

        // 1. Восклицания и эмоциональные знаки
        if txt.contains('!') {
            score += 0.18;
        }
        if txt.contains("!!") || txt.contains("?!") || txt.contains("!?") {
            score += 0.14;
        }
        if txt.contains('?') {
            score += 0.08;
        }

        // 2. Поиск маркеров эмоций/смеха/драмы/хайлайтов (с накоплением до 2 совпадений)
        let mut kw_hits = 0;
        for kw in &intense_keywords {
            if txt.contains(kw) {
                kw_hits += 1;
                if kw_hits >= 2 {
                    break;
                }
            }
        }
        score += (kw_hits as f64) * 0.14;

        // 3. Плотность речи (скорость произношения слов)
        let duration = (f.end - f.start).max(0.4);
        let speech_rate = (word_count as f64) / duration;

        if speech_rate > 2.5 {
            score += ((speech_rate - 2.5) * 0.06).min(0.14);
        }

        // 4. Насыщенность и длина фразы (плавная градация без одинаковых значений)
        let char_count = txt.chars().count();
        if char_count > 15 {
            score += ((char_count as f64 - 15.0) / 120.0).min(0.12);
        }

        score.clamp(0.15, 0.96)
    }).collect()
}

pub fn compute_text_scores_fallback(fragments: &[SubtitleFragment]) -> Vec<f64> {
    compute_text_scores_advanced(fragments)
}
