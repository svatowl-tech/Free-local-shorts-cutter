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

/// Определяет, заканчивается ли текст реплики знаком завершения предложения (., !, ?, …, »)
pub fn has_terminal_punctuation(text: &str) -> bool {
    let trimmed = text
        .trim()
        .trim_end_matches(['"', '\'', '»', ')', ']']);
    trimmed.ends_with('.')
        || trimmed.ends_with('!')
        || trimmed.ends_with('?')
        || trimmed.ends_with('…')
}

/// Определяет, начинается ли текст со строчной (маленькой) буквы или союза-продолжения после запятой
fn starts_with_lowercase_continuation(text: &str) -> bool {
    let trimmed = text
        .trim()
        .trim_start_matches(['"', '\'', '«', '(', '[']);
    if let Some(first_ch) = trimmed.chars().next() {
        if first_ch.is_lowercase() {
            return true;
        }
    }
    false
}

/// Проверяет, является ли реплика `idx` логическим НАЧАЛОМ нового предложения / законченной мысли
pub fn is_sentence_start(fragments: &[SubtitleFragment], idx: usize) -> bool {
    if idx == 0 || idx >= fragments.len() {
        return true;
    }
    let prev = &fragments[idx - 1];
    let curr = &fragments[idx];
    let gap = (curr.start - prev.end).max(0.0);

    // Если между репликами заметная пауза (>= 0.65с) — это гарантированно начало новой фразы
    if gap >= 0.65 {
        return true;
    }

    // Если текущая строка начинается с диалогового тире («- » или «— ») — это новая реплика собеседника
    let curr_trimmed = curr.text.trim();
    if curr_trimmed.starts_with('-') || curr_trimmed.starts_with('—') {
        return true;
    }

    // Если текущая строка начинается с маленькой буквы и пауза маленькая (< 1.0с) — это середина предложения!
    if starts_with_lowercase_continuation(curr_trimmed) && gap < 1.0 {
        return false;
    }

    // Если предыдущая строка закончилась точкой, !, ? или многоточием — текущая начинает новое предложение
    if has_terminal_punctuation(&prev.text) {
        return true;
    }

    // Иначе предыдущая строка оборвалась на запятой или на полуслове без паузы — значит это продолжение фразы
    false
}

/// Проверяет, является ли реплика `idx` логическим КОНЦОМ предложения / законченной мысли
pub fn is_sentence_end(fragments: &[SubtitleFragment], idx: usize) -> bool {
    if idx + 1 >= fragments.len() {
        return true;
    }
    let curr = &fragments[idx];
    let next = &fragments[idx + 1];
    let gap_next = (next.start - curr.end).max(0.0);

    // Если после реплики следует пауза >= 0.75с — фраза логически завершена
    if gap_next >= 0.75 {
        return true;
    }

    // Если следующая реплика — реплика другого персонажа (начинается с тире) и текущая не оборвана на предлоге
    let next_trimmed = next.text.trim();
    if (next_trimmed.starts_with('-') || next_trimmed.starts_with('—')) && has_terminal_punctuation(&curr.text) {
        return true;
    }

    // Если следующая реплика начинается со строчной буквы без паузы — предложение ещё продолжается!
    if starts_with_lowercase_continuation(next_trimmed) && gap_next < 0.85 {
        return false;
    }

    // Если текущая реплика заканчивается на ., !, ?, … — предложение завершено
    if has_terminal_punctuation(&curr.text) {
        return true;
    }

    false
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
                // Смешиваем оценку LLM (70%) с локальной оценкой границ фраз (30%), чтобы избежать плоских 0.99
                let blended = (val.clamp(0.10, 0.96) * 0.70 + fallback[i] * 0.30).clamp(0.12, 0.96);
                out.push(blended);
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

    // Если выбран Builtin — используем быстрый умный локальный семантический анализатор с контролем целостности фраз
    if config.engine == TextEngine::Builtin {
        crate::logger::emit_log(
            app_handle,
            "LLM",
            "info",
            &format!("Применяется высокоскоростной встроенный NLP-движок анализа целостных фраз (всего реплик: {})", fragments.len()),
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
                &format!("Запуск локального llama-cli с моделью: '{}' (контекст: 2048, контроль целостности фраз)", model_path.display()),
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

                // Если запуск с GPU offload (-ngl 99) упал ИЛИ вернул поврежденный вывод (например, 'GGGG...' или мусор токенизатора),
                // немедленно переключаемся в ЧИСТЫЙ режим CPU (CUDA_VISIBLE_DEVICES="" и -ngl 0) и повторяем пакет!
                let gpu_output_corrupted = match &batch_res {
                    Ok(out) => parse_json_array(out, chunk.len()).is_none(),
                    Err(_) => true,
                };

                if gpu_output_corrupted && use_gpu {
                    crate::logger::emit_log(
                        app_handle,
                        "LLM",
                        "warn",
                        "Обнаружен сбой генерации на GPU (артефакты CUDA FP16 / невалидный вывод). Отключение CUDA для llama-cli (переход на чистый CPU) и повтор пакета...",
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
                        let parsed = parse_json_array(&result, chunk.len());
                        if let Some(ref batch_scores) = parsed {
                            consecutive_parse_failures = 0;
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "info",
                                &format!("✓ Пакет {}/{}: получено {} валидных оценок от llama-cli", batch_idx + 1, total_batches, batch_scores.len()),
                            );
                            all_scores.extend_from_slice(&normalize_batch_scores(batch_scores, chunk));
                        } else {
                            consecutive_parse_failures += 1;
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "warn",
                                "Невалидный ответ от GGUF-модели (поврежденный токенизатор или формат). Применяется встроенный семантический NLP-анализатор полных фраз.",
                            );
                            all_scores.extend_from_slice(&compute_text_scores_advanced(chunk));

                            // Если даже в режиме чистого CPU модель выдает невалидный вывод (или 2 сбоя подряд) —
                            // мгновенно срабатывает предохранитель, чтобы не тратить 6 минут на битые пакеты!
                            if (!use_gpu || consecutive_parse_failures >= 2) && batch_idx + 1 < total_batches {
                                crate::logger::emit_log(
                                    app_handle,
                                    "LLM",
                                    "warn",
                                    "Предохранитель LLM: выбранная GGUF-модель выдает некорректные токены. Мгновенное переключение всех оставшихся пакетов на встроенный NLP-движок анализа полных фраз (экономия ~6 минут).",
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
                                "llama-cli завершился со сбоем ({}). Автоматическое переключение оставшихся пакетов на встроенный скоростной NLP-движок.",
                                e
                            ),
                        );
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
                    let parsed = parse_json_array(&result, chunk.len());
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
        let start_tag = if is_sentence_start(fragments, i) { "[НАЧАЛО ФРАЗЫ]" } else { "[ПРОДОЛЖЕНИЕ]" };
        let end_tag = if is_sentence_end(fragments, i) { "[КОНЕЦ ФРАЗЫ]" } else { "[НЕ ЗАВЕРШЕНА]" };
        lines.push(format!("{}. {} {} \"{}\"", i + 1, start_tag, end_tag, clean_txt.trim()));
    }
    let input_list = lines.join("\n");
    format!(
        "Instruction: Ты профессиональный режиссер монтажа вертикальных роликов (Shorts, Reels, TikTok) длительностью от 10 до 20 секунд.\n\
Оцени каждую из {} реплик по шкале от 0.0 до 1.0 для выбора лучших цельных диалогов.\n\
ВАЖНЫЕ ПРАВИЛА ЦЕЛОСТНОСТИ ФРАЗ:\n\
1. Высокую оценку (0.75 - 0.95) давай ярким, эмоциональным, драматичным или смешным репликам, которые начинают или завершают законченную мысль ([НАЧАЛО ФРАЗЫ] ... [КОНЕЦ ФРАЗЫ]).\n\
2. Если строка является обрывком на полуслове ([ПРОДОЛЖЕНИЕ] и [НЕ ЗАВЕРШЕНА]), оценивай её ниже (0.25 - 0.55), чтобы ролик всегда начинался с начала фразы и заканчивался на полной точке/восклицании.\n\
3. Выведи ТОЛЬКО валидный JSON-массив из {} чисел от 0.0 до 1.0 в том же порядке. Никакого текста и комментариев.\n\
Пример ответа: [0.85, 0.40, 0.72]\n\n\
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

    // Записываем промпт во временный UTF-8 файл (предотвращает искажение кириллицы в Windows argv)
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let prompt_file = std::env::temp_dir().join(format!("vc_llama_prompt_{}_{}.txt", std::process::id(), ts));
    tokio::fs::write(&prompt_file, prompt.as_bytes()).await.map_err(AppError::Io)?;

    let ngl_val = if use_gpu { "99" } else { "0" };
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().clamp(2, 8))
        .unwrap_or(4)
        .to_string();

    let mut cmd = sidecar_command
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle));

    // КРИТИЧЕСКОЕ ИСПРАВЛЕНИЕ:
    // В CUDA-сборке llama-cli простой флаг `-ngl 0` всё равно инициализирует `ggml_cuda_init` и выполняет матричные
    // операции на GPU, что на некоторых видеокартах/квантах вызывает мусорную генерацию токенов.
    // При `use_gpu == false` принудительно скрываем CUDA-устройства через `CUDA_VISIBLE_DEVICES=""` для 100% чистого CPU-расчета!
    if !use_gpu {
        cmd = cmd
            .env("CUDA_VISIBLE_DEVICES", "")
            .env("GGML_CUDA_NO_PINNED", "1");
    }

    let cmd = cmd.args([
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
                            &format!("llama-cli завершился (код {:?})", payload.code),
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
        r = tokio::time::timeout(Duration::from_secs(45), run_future) => {
            match r {
                Ok(inner) => inner,
                Err(_) => {
                    let _ = child.kill();
                    Err(AppError::Pipeline("Превышен таймаут ожидания ответа от llama-cli (45с)".to_string()))
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

/// Проверяет, содержит ли ответ LLM признаки сбоя токенизатора / FP16 переполнения («GGGG...», иероглифы, битые байты)
fn is_corrupted_llm_output(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.contains("GGGGGGGG") || trimmed.contains('\u{FFFD}') {
        return true;
    }

    // Подсчитываем иероглифы CJK / арабские / индийские символы и обычные буквы
    let mut exotic_count = 0usize;
    let mut alpha_count = 0usize;
    let mut digit_count = 0usize;

    for c in trimmed.chars() {
        if ('\u{4E00}'..='\u{9FFF}').contains(&c)
            || ('\u{0600}'..='\u{06FF}').contains(&c)
            || ('\u{0900}'..='\u{097F}').contains(&c)
        {
            exotic_count += 1;
        } else if c.is_alphabetic() {
            alpha_count += 1;
        } else if c.is_ascii_digit() {
            digit_count += 1;
        }
    }

    // В ответе должен быть JSON-массив чисел `[0.85, 0.40, ...]`. Если там иероглифы или сотни букв — это мусор!
    if exotic_count >= 2 || digit_count == 0 || alpha_count > digit_count + 25 {
        return true;
    }

    false
}

/// Строгий парсер JSON-массива оценок от LLM.
/// Отклоняет случайные числа из словесного мусора и требует не менее 50% валидных чисел в диапазоне [0.0, 1.0].
fn parse_json_array(text: &str, expected_len: usize) -> Option<Vec<f64>> {
    if is_corrupted_llm_output(text) {
        return None;
    }

    let cleaned = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let min_required = (expected_len / 2).max(1);

    let validate_vec = |nums: Vec<f64>| -> Option<Vec<f64>> {
        if nums.len() < min_required {
            return None;
        }
        // Все оценки должны быть осмысленными числами в пределах [0.0, 1.0] (допускаем 0.0..=1.05)
        let valid_range_count = nums.iter().filter(|&&x| x >= 0.0 && x <= 1.05).count();
        if valid_range_count < min_required {
            return None;
        }
        // Проверяем, что модель не выдала один и тот же токен (например [0.5, 0.5, 0.5, ...])
        if nums.len() >= 4 {
            let first = nums[0];
            if nums.iter().all(|&x| (x - first).abs() < 1e-4) {
                return None;
            }
        }
        Some(nums.into_iter().map(|x| x.clamp(0.05, 0.96)).collect())
    };

    // 1. Полноценный JSON-массив в квадратных скобках
    if let (Some(start), Some(end)) = (cleaned.find('['), cleaned.rfind(']')) {
        if start < end {
            let json_str = &cleaned[start..=end];
            if let Ok(nums) = serde_json::from_str::<Vec<f64>>(json_str) {
                if let Some(valid) = validate_vec(nums) {
                    return Some(valid);
                }
            }
        }
    }

    // 2. Если ответ начинается с продолжения массива (т.к. промпт заканчивается на `[`), добавляем открывающую скобку
    if !cleaned.starts_with('[') {
        if let Some(end) = cleaned.find(']') {
            let with_opening = format!("[{}", &cleaned[..=end]);
            if let Ok(nums) = serde_json::from_str::<Vec<f64>>(&with_opening) {
                if let Some(valid) = validate_vec(nums) {
                    return Some(valid);
                }
            }
        }
    }

    // 3. Если ответ начинается с массива, но закрывающая скобка обрезана лимитом токенов
    let candidate_body = cleaned.trim_start_matches('[').trim_end_matches(']');
    // Разрешаем парсинг списка через запятую ТОЛЬКО если строка состоит исключительно из чисел, точек, запятых и пробелов
    let is_pure_number_list = candidate_body
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == ',' || c == '-' || c.is_whitespace());

    if is_pure_number_list {
        let nums: Vec<f64> = candidate_body
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<f64>().ok())
            .collect();
        if let Some(valid) = validate_vec(nums) {
            return Some(valid);
        }
    }

    None
}

/// Продвинутый локальный семантический анализатор диалогов, эмоциональных всплесков и целостности фраз
pub fn compute_text_scores_advanced(fragments: &[SubtitleFragment]) -> Vec<f64> {
    if fragments.is_empty() {
        return Vec::new();
    }

    // Словник высокой драматургической, эмоциональной и сюжетной ценности (аниме, стримы, интервью, кино)
    let high_impact_keywords = [
        "ха-ха", "ахах", "haha", "лол", "вау", "ого", "что?!", "жесть", "круто", "капец",
        "не может быть", "невозможно", "вперёд", "стоп", "быстрее", "постой", "подожди",
        "бей", "беги", "смотри", "боже", "шок", "ужас", "победа", "атака", "удар",
        "сила", "смерть", "крик", "войн", "приказ", "огонь", "враг", "преступлен",
        "уничтож", "спас", "кров", "биться", "штурм", "взрыв", "молодец", "секрет",
        "тайна", "правда", "лжешь", "врёшь", "прости", "обещаю", "поклялся", "никогда",
        "навсегда", "люблю", "ненавижу", "убью", "защищ", "опасно", "ловушк", "план",
        "конец", "начало", "победил", "проиграл", "герой", "монстр", "магия", "способност",
        "уровень", "память", "разум", "сердц", "душа", "судьб", "выбор", "шанс", "черт", "чёрт"
    ];

    let hook_starters = [
        "слушай", "знаешь", "смотри", "погоди", "неужели", "значит", "почему", "зачем",
        "как же", "так вот", "на самом деле", "однако", "кстати", "представь", "эй",
        "послушай", "внимание", "осторожно", "клянусь", "честно"
    ];

    // Шаг 1. Базовая оценка каждой отдельной реплики
    let mut raw_scores: Vec<f64> = fragments
        .iter()
        .enumerate()
        .map(|(idx, f)| {
            if crate::whisper::is_whisper_hallucination(&f.text) {
                return 0.05;
            }

            let txt = f.text.trim().to_lowercase();
            let word_count = txt.split_whitespace().count();

            if word_count == 0 {
                return 0.05;
            }

            // Сверхкороткие междометия без восклицания не должны сами по себе становиться ядром клипа
            if word_count <= 1 && !txt.contains('!') && !txt.contains('?') {
                return 0.22;
            }

            let mut score: f64 = 0.38;

            // 1. Эмоциональная пунктуация
            if txt.contains('!') {
                score += 0.14;
            }
            if txt.contains("!!") || txt.contains("?!") || txt.contains("!?") {
                score += 0.11;
            }
            if txt.contains('?') {
                score += 0.08;
            }
            if txt.contains('…') || txt.contains("...") {
                score += 0.04;
            }

            // 2. Ключевые слова конфликта, юмора, откровений и экшена
            let mut kw_hits = 0usize;
            for kw in &high_impact_keywords {
                if txt.contains(kw) {
                    kw_hits += 1;
                    if kw_hits >= 3 {
                        break;
                    }
                }
            }
            score += (kw_hits as f64) * 0.11;

            // 3. Хук-маркеры в начале фразы (привлекают внимание в первые 2 секунды ролика)
            if is_sentence_start(fragments, idx) {
                score += 0.06;
                if hook_starters.iter().any(|h| txt.starts_with(h) || txt.contains(h)) {
                    score += 0.07;
                }
            } else {
                // Если реплика начинается с середины незаконченного предложения — снижаем её вес как точки старта
                score -= 0.08;
            }

            // Бонус за логическую завершенность мысли
            if is_sentence_end(fragments, idx) {
                score += 0.04;
            }

            // 4. Темп и плотность живой речи
            let duration = (f.end - f.start).max(0.4);
            let speech_rate = (word_count as f64) / duration;
            if speech_rate >= 2.2 && speech_rate <= 5.5 {
                score += ((speech_rate - 2.0) * 0.035).clamp(0.02, 0.10);
            }

            // 5. Насыщенность реплики по числу слов
            if word_count >= 5 {
                score += ((word_count as f64 - 4.0) * 0.012).min(0.09);
            }

            score.clamp(0.15, 0.94)
        })
        .collect();

    // Шаг 2. Группировка реплик в полные предложения и передача импульса к НАЧАЛУ предложения.
    // Это гарантирует, что алгоритм выбора клипов всегда выберет начало цельной фразы, а не её обрывок!
    let mut i = 0;
    while i < fragments.len() {
        let sent_start = i;
        let mut sent_end = i;
        while sent_end + 1 < fragments.len() && !is_sentence_end(fragments, sent_end) && (sent_end - sent_start) < 6 {
            sent_end += 1;
        }

        if sent_end > sent_start {
            // Находим пиковую эмоцию внутри всего предложения [sent_start..=sent_end]
            let max_in_sentence = raw_scores[sent_start..=sent_end]
                .iter()
                .copied()
                .fold(0.0_f64, f64::max);

            // Усиливаем первую реплику предложения (точку входа в мысль), чтобы окно клипа открывалось именно с неё
            raw_scores[sent_start] = (raw_scores[sent_start] * 0.45 + max_in_sentence * 0.55 + 0.05).clamp(0.18, 0.96);
        }

        i = sent_end + 1;
    }

    raw_scores
}

pub fn compute_text_scores_fallback(fragments: &[SubtitleFragment]) -> Vec<f64> {
    compute_text_scores_advanced(fragments)
}
