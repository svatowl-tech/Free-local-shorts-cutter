// src-tauri/src/text_analyzer.rs

use crate::error::{AppError, AppResult};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tokio::sync::broadcast::Receiver;

static BROKEN_GGUF_MODELS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn is_model_marked_broken(path_str: &str) -> bool {
    BROKEN_GGUF_MODELS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map(|set| set.contains(path_str))
        .unwrap_or(false)
}

fn mark_model_broken(path_str: &str) {
    if let Ok(mut set) = BROKEN_GGUF_MODELS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
    {
        set.insert(path_str.to_string());
    }
}

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

/// Определяет, начинается ли реплика с союза или вводного оборота, который грамматически привязан к предыдущему предложению
/// (например: «Поэтому он все еще сидит...», «И в моих руках он...», «К тому же...», «Потому что...»)
pub fn starts_with_discourse_continuation(text: &str) -> bool {
    let clean = text
        .trim()
        .trim_start_matches(['-', '—', '"', '\'', '«', '(', '[', ' '])
        .to_lowercase();

    let continuation_prefixes = [
        "поэтому ",
        "потому что ",
        "так что ",
        "ведь ",
        "причем ",
        "причём ",
        "к тому же",
        "более того",
        "тем более",
        "однако ",
        "зато ",
        "и в моих ",
        "и поэтому ",
        "и тогда ",
        "и всё же",
        "и все же",
        "и при этом",
        "а также ",
        "а значит",
        "а потом ",
        "но и ",
        "да и ",
        "или же ",
        "во многом из-за",
        "вам многом из-за",
        "благодаря этому",
        "в результате ",
        "иначе говоря",
    ];

    if continuation_prefixes.iter().any(|p| clean.starts_with(p)) {
        return true;
    }

    // Одиночные сочинительные союзы «И ...», «А ...», «Но ...» в начале строки без диалогового тире — это продолжение аргумента того же персонажа
    let raw_trimmed = text.trim();
    let has_dialog_dash = raw_trimmed.starts_with('-') || raw_trimmed.starts_with('—');
    if !has_dialog_dash {
        if clean.starts_with("и ") || clean.starts_with("но ") || clean.starts_with("а ") || clean.starts_with("или ") {
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

    // Если между репликами длинная пауза (>= 1.25с) — это гарантированно начало новой фразы/сцены
    if gap >= 1.25 {
        return true;
    }

    let curr_trimmed = curr.text.trim();

    // Если текущая строка начинается с маленькой буквы и пауза < 1.1с — это середина незаконченного предложения!
    if starts_with_lowercase_continuation(curr_trimmed) && gap < 1.1 {
        return false;
    }

    // Если строка начинается с причинно-следственного или присоединительного союза («Поэтому...», «И в моих руках...», «К тому же...»)
    // и пауза после предыдущей фразы небольшая (< 1.1с) — нельзя начинать клип с середины рассуждения!
    if starts_with_discourse_continuation(curr_trimmed) && gap < 1.1 {
        return false;
    }

    // Если между репликами заметная пауза (>= 0.70с) и предыдущая фраза закончилась знаком препинания
    if gap >= 0.70 && has_terminal_punctuation(&prev.text) {
        return true;
    }

    // Если текущая строка начинается с диалогового тире («- » или «— ») и предыдущая завершилась точкой/!/? — новая реплика собеседника
    if (curr_trimmed.starts_with('-') || curr_trimmed.starts_with('—')) && has_terminal_punctuation(&prev.text) {
        return true;
    }

    // Если предыдущая строка закончилась точкой, !, ? или многоточием — текущая начинает новое предложение
    if has_terminal_punctuation(&prev.text) {
        return true;
    }

    // Иначе предыдущая строка оборвалась на запятой или на полуслове без паузы — значит это продолжение фразы
    false
}

/// Проверяет, является ли реплика `idx` слабым/оборванным финалом для клипа
/// (короткие поддакивания вроде «- Да уж.», «- Ясно.», либо незаконченные подводки перед немедленным ответом собеседника)
pub fn is_weak_clip_ending(fragments: &[SubtitleFragment], idx: usize) -> bool {
    if idx >= fragments.len() {
        return false;
    }
    let curr = &fragments[idx];
    let clean = curr
        .text
        .trim()
        .trim_start_matches(['-', '—', ' '])
        .trim_end_matches(['.', '!', '?', '…', '"', '»', ' '])
        .to_lowercase();
    let word_count = clean.split_whitespace().count();

    // 1. Многоточие в конце («Вам многом из-за...», «И пока не добьюсь свои цели...») — мысль явно не договорена
    if curr.text.trim().ends_with("...") || curr.text.trim().ends_with('…') {
        return true;
    }

    // 2. Короткие реплики-поддакивания (1–2 слова), после которых сразу идет настоящая панчлайн-реплика
    let weak_backchannels = [
        "да уж", "ясно", "понятно", "угу", "ага", "это да", "вот как", "точно",
        "да", "нет", "ну да", "конечно", "согласен", "интересно", "привет",
        "разрешите войти", "разрешите представиться", "прошу сюда",
    ];
    if weak_backchannels.contains(&clean.as_str()) {
        return true;
    }

    // 3. Приветствия и обращения, открывающие новую сцену («Прости за ожидание, офицер Синомия», «Значит, ты и есть та самая дочка Исао»)
    if clean.starts_with("прости за ожидан")
        || clean.starts_with("значит, и есть")
        || clean.starts_with("значит, ты и есть")
        || clean.contains("есть то самое дочка")
        || clean.contains("есть та самая дочка")
        || clean.starts_with("заходи, я представл")
    {
        return true;
    }

    // 4. Если после короткой реплики (<= 2 слов) без паузы (< 0.35с) следует продолжение диалога
    if idx + 1 < fragments.len() {
        let gap_next = (fragments[idx + 1].start - curr.end).max(0.0);
        if word_count <= 2 && gap_next < 0.35 && !curr.text.contains('!') {
            return true;
        }
    }

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
    let next_trimmed = next.text.trim();

    // Если следующая реплика начинается со строчной буквы без паузы — предложение ещё продолжается!
    if starts_with_lowercase_continuation(next_trimmed) && gap_next < 0.95 {
        return false;
    }

    // Если следующая реплика начинается с союза продолжения мысли («И в моих руках...», «Поэтому...») с малой паузой (< 0.55с) —
    // желательно не обрывать мысль до завершения аргумента
    if starts_with_discourse_continuation(next_trimmed) && gap_next < 0.55 {
        return false;
    }

    // Если текущая строка заканчивается многоточием, а следующая идет сразу следом (< 0.8с) — мысль еще не закончена
    if (curr.text.trim().ends_with("...") || curr.text.trim().ends_with('…')) && gap_next < 0.8 {
        return false;
    }

    // Если после реплики следует пауза >= 0.75с и есть знак конца предложения — фраза логически завершена
    if gap_next >= 0.75 && has_terminal_punctuation(&curr.text) {
        return true;
    }

    // Если пауза >= 1.2с — даже без точки это конец реплики перед паузой
    if gap_next >= 1.2 {
        return true;
    }

    // Если следующая реплика — реплика другого персонажа (начинается с тире) и текущая имеет знак завершения
    if (next_trimmed.starts_with('-') || next_trimmed.starts_with('—')) && has_terminal_punctuation(&curr.text) {
        return true;
    }

    // Если текущая реплика заканчивается на ., !, ? — предложение грамматически завершено
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
            &format!("Поиск локальной GGUF-модели для llama-cli (выбрана: '{}')...", config.model_name),
        );
        let is_legacy_default = config.model_name == "models/qwen-1.5b-chat.gguf"
            || config.model_name == "qwen-1.5b-chat.gguf";
        let exact_match = if is_legacy_default {
            crate::models::resolve_any_llm_model(app_handle, Some(&config.model_name))
        } else {
            crate::models::resolve_model_path(app_handle, &config.model_name)
        };
        let mut resolved_model = exact_match
            .clone()
            .or_else(|| crate::models::resolve_any_llm_model(app_handle, Some(&config.model_name)));

        if let Some(ref primary_path) = resolved_model {
            let primary_str = primary_path.to_string_lossy().to_string();
            if is_model_marked_broken(&primary_str) {
                // Проверяем, нет ли другой рабочей GGUF-модели на диске (например, qwen2.5-3b-instruct в Roaming вместо битой qwen-1.5b в Local)
                if let Some(alt_path) = crate::models::resolve_any_llm_model(app_handle, None) {
                    let alt_str = alt_path.to_string_lossy().to_string();
                    if !is_model_marked_broken(&alt_str) && alt_str != primary_str {
                        crate::logger::emit_log(
                            app_handle,
                            "LLM",
                            "info",
                            &format!(
                                "Переключение с проблемной модели '{}' на исправную локальную GGUF-модель '{}'",
                                primary_path.display(),
                                alt_path.display()
                            ),
                        );
                        resolved_model = Some(alt_path);
                    }
                }
            }
        }

        if let Some(mut model_path) = resolved_model {
            let mut model_path_str = model_path.to_string_lossy().to_string();

            if exact_match.is_none() {
                crate::logger::emit_log(
                    app_handle,
                    "LLM",
                    "warn",
                    &format!(
                        "Выбранная модель '{}' ещё не скачана в Менеджере ИИ-моделей. Найдена резервная локальная модель: '{}'",
                        config.model_name,
                        model_path.display()
                    ),
                );
            }

            if is_model_marked_broken(&model_path_str) {
                crate::logger::emit_log(
                    app_handle,
                    "LLM",
                    "warn",
                    &format!(
                        "Модель '{}' ранее показала ошибку токенизатора в этой сессии. Мгновенное применение встроенного NLP-анализатора полных фраз.",
                        model_path.display()
                    ),
                );
                return Ok(compute_text_scores_advanced(fragments));
            }

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
            let mut gpu_succeeded_once = false;
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
                    chunk.len(),
                    use_gpu,
                    cancel_rx.resubscribe(),
                ).await;

                // Переключаемся на чистый CPU ТОЛЬКО если сбой произошел на самом первом пакете (когда GPU еще ни разу не выдал валидный ответ).
                // Если GPU уже успешно обработал предыдущие пакеты 1..N, то видеокарта и GGUF-модель полностью исправны, и тратить 38с на CPU не нужно!
                let gpu_output_corrupted = match &batch_res {
                    Ok(out) => parse_json_array(out, chunk.len()).is_none(),
                    Err(_) => true,
                };

                if gpu_output_corrupted && use_gpu && !gpu_succeeded_once {
                    // Прежде чем отключать GPU, проверяем: возможно, выбрана старая/битая GGUF-модель (например qwen-1.5b в Local),
                    // тогда как рядом в Roaming установлена исправная qwen2.5-3b-instruct!
                    mark_model_broken(&model_path_str);
                    if let Some(alt_path) = crate::models::resolve_any_llm_model(app_handle, None) {
                        let alt_str = alt_path.to_string_lossy().to_string();
                        if !is_model_marked_broken(&alt_str) && alt_str != model_path_str {
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "warn",
                                &format!(
                                    "Модель '{}' выдала невалидный ответ. Автоматическое переключение на другую установленную GGUF-модель '{}' (GPU)...",
                                    model_path.display(),
                                    alt_path.display()
                                ),
                            );
                            model_path = alt_path;
                            model_path_str = alt_str;
                            batch_res = run_llama_cli(
                                app_handle,
                                &model_path_str,
                                &prompt,
                                chunk.len(),
                                true,
                                cancel_rx.resubscribe(),
                            )
                            .await;
                        }
                    }

                    let still_corrupted = match &batch_res {
                        Ok(out) => parse_json_array(out, chunk.len()).is_none(),
                        Err(_) => true,
                    };

                    if still_corrupted {
                        crate::logger::emit_log(
                            app_handle,
                            "LLM",
                            "warn",
                            "Обнаружен сбой генерации на GPU в первом пакете (артефакты CUDA FP16 / невалидный вывод). Отключение CUDA для llama-cli (переход на чистый CPU) и повтор пакета...",
                        );
                        use_gpu = false;
                        batch_res = run_llama_cli(
                            app_handle,
                            &model_path_str,
                            &prompt,
                            chunk.len(),
                            false,
                            cancel_rx.resubscribe(),
                        )
                        .await;
                    }
                }

                match batch_res {
                    Ok(result) => {
                        let parsed = parse_json_array(&result, chunk.len());
                        if let Some(ref batch_scores) = parsed {
                            consecutive_parse_failures = 0;
                            if use_gpu {
                                gpu_succeeded_once = true;
                            }
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
                                "info",
                                &format!(
                                    "Пакет {}/{}: нестандартный формат ответа LLM, для {} реплик применен семантический NLP-анализатор полных фраз.",
                                    batch_idx + 1,
                                    total_batches,
                                    chunk.len()
                                ),
                            );
                            all_scores.extend_from_slice(&compute_text_scores_advanced(chunk));

                            // Помечаем модель как сломанную ТОЛЬКО если она ни разу не выдала валидный ответ (!gpu_succeeded_once)
                            if !gpu_succeeded_once && (!use_gpu || consecutive_parse_failures >= 2) && batch_idx + 1 < total_batches {
                                mark_model_broken(&model_path_str);
                                crate::logger::emit_log(
                                    app_handle,
                                    "LLM",
                                    "warn",
                                    "Предохранитель LLM: выбранная GGUF-модель выдает некорректные токены. Мгновенное переключение всех оставшихся пакетов на встроенный NLP-движок анализа полных фраз.",
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
    expected_count: usize,
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

    // Динамический лимит токенов пропорционально размеру пакета (на последнем неполном пакете из 25 реплик
    // избыточный лимит 384 заставлял модель дописывать комментарии после закрывающей скобки `]`)
    let max_tokens = (expected_count * 8 + 28).clamp(64, 300).to_string();

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
        "-n", &max_tokens,
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
/// Сначала отсекает любой лишний текст/комментарии модели после закрывающей скобки `]`,
/// затем проверяет массив на отсутствие мусора и требует не менее 50% валидных чисел в диапазоне [0.0, 1.0].
fn parse_json_array(text: &str, expected_len: usize) -> Option<Vec<f64>> {
    let raw_trimmed = text.trim();
    if raw_trimmed.is_empty() || raw_trimmed.contains("GGGGGGGG") || raw_trimmed.contains('\u{FFFD}') {
        return None;
    }

    let cleaned = raw_trimmed
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

    // 1. Полноценный JSON-массив в квадратных скобках (берем ПЕРВУЮ закрывающую скобку `]` после `[`,
    // чтобы отбросить любые пояснения, которые LLM могла дописать после завершения массива на коротком последнем пакете!)
    if let Some(start) = cleaned.find('[') {
        let after_start = &cleaned[start..];
        if let Some(rel_end) = after_start.find(']') {
            let json_str = &after_start[..=rel_end];
            if !is_corrupted_llm_output(json_str) {
                if let Ok(nums) = serde_json::from_str::<Vec<f64>>(json_str) {
                    if let Some(valid) = validate_vec(nums) {
                        return Some(valid);
                    }
                }
            }
        }
    }

    // 2. Если ответ начинается с продолжения массива (т.к. промпт заканчивается на `[`), добавляем открывающую скобку до первой `]`
    if !cleaned.starts_with('[') {
        if let Some(end) = cleaned.find(']') {
            let slice_to_bracket = &cleaned[..=end];
            if !is_corrupted_llm_output(slice_to_bracket) {
                let with_opening = format!("[{}", slice_to_bracket);
                if let Ok(nums) = serde_json::from_str::<Vec<f64>>(&with_opening) {
                    if let Some(valid) = validate_vec(nums) {
                        return Some(valid);
                    }
                }
            }
        }
    }

    // 3. Извлекаем числа напрямую из префикса ответа (до первой `]` или до конца строки, если скобка обрезана лимитом токенов)
    let prefix_region = if let Some(start) = cleaned.find('[') {
        let after = &cleaned[start + 1..];
        if let Some(end) = after.find(']') {
            &after[..end]
        } else {
            after
        }
    } else if let Some(end) = cleaned.find(']') {
        &cleaned[..end]
    } else {
        cleaned
    };

    if is_corrupted_llm_output(prefix_region) {
        return None;
    }

    let nums: Vec<f64> = prefix_region
        .split(|c: char| c == ',' || c == '\n' || c == ';')
        .map(|s| s.trim().trim_matches(|c: char| c == '[' || c == ']'))
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();

    if let Some(valid) = validate_vec(nums) {
        return Some(valid);
    }

    None
}

/// Определяет, относится ли реплика к вокалу музыкального опенинга/эндинга (например, японская/английская песня поверх заставки)
pub fn is_opening_or_song_fragment(fragments: &[SubtitleFragment], idx: usize) -> bool {
    if idx >= fragments.len() {
        return false;
    }
    let f = &fragments[idx];
    let text = f.text.trim();

    // 1. В русскоязычном видео строки с латинскими словами («We will like to start!», «you'll be speedy. Yes!») — это вокал песни/опенинга
    let latin_letters = text.chars().filter(|c| c.is_ascii_alphabetic()).count();
    if latin_letters >= 3 {
        return true;
    }

    // 2. Проверяем, не находится ли реплика внутри 30-секундного вокального блока опенинга/эндинга,
    // который следует сразу после длинной музыкальной паузы (>= 14с, где были удалены теги [музыка]),
    // и где Whisper нарезает вокал ровно по целочисленным секундам (3.000с, 4.000с, 5.000с)
    let is_exact_int_dur = |frag: &SubtitleFragment| -> bool {
        let d = frag.end - frag.start;
        d >= 2.0 && d <= 6.0 && (d - d.round()).abs() < 0.025
    };

    if is_exact_int_dur(f) {
        let win_start = idx.saturating_sub(4);
        let win_end = (idx + 4).min(fragments.len().saturating_sub(1));
        let int_dur_neighbors = (win_start..=win_end)
            .filter(|&k| is_exact_int_dur(&fragments[k]))
            .count();

        // Проверяем, был ли перед этим кластером длинный музыкальный разрыв (>= 14с) или латинские строки песни рядом
        let has_music_gap_nearby = (win_start..=win_end).any(|k| {
            k > 0 && (fragments[k].start - fragments[k - 1].end) >= 14.0
        });
        let has_latin_nearby = (win_start..=win_end).any(|k| {
            fragments[k].text.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 3
        });

        if int_dur_neighbors >= 4 && (has_music_gap_nearby || has_latin_nearby) {
            return true;
        }
    }

    false
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
            if crate::whisper::is_whisper_hallucination(&f.text)
                || is_opening_or_song_fragment(fragments, idx)
            {
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
