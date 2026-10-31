// src-tauri/src/whisper.rs
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
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

/// Возвращает канонический путь к файлу субтитров в папке проекта для данного видео:
/// `<папка_видео>/<имя_видео>_Проект/Субтитры/<имя_видео>_Субтитры.srt`
pub fn get_project_srt_path(video_path: &Path) -> PathBuf {
    let file_stem = video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let video_parent = video_path.parent().unwrap_or_else(|| Path::new("."));
    video_parent
        .join(format!("{}_Проект", file_stem))
        .join("Субтитры")
        .join(format!("{}_Субтитры.srt", file_stem))
}

/// Известные галлюцинации Whisper на музыке, титрах, опенингах и тишине
pub fn is_whisper_hallucination(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }

    let lower = trimmed.to_lowercase().replace('ё', "е");

    // Отсекаем технические теги музыки и фонового шума вида [музыка], (музыка), ♪ и т.д.
    if (lower.starts_with('[') && lower.ends_with(']'))
        || (lower.starts_with('(') && lower.ends_with(')'))
        || lower.contains('♪')
        || lower.contains('♫')
    {
        return true;
    }

    // Одиночные звуковые ремарки КАПСОМ или без скобок
    let exact_tags = [
        "музыка",
        "аплодисменты",
        "смех",
        "вздох",
        "звук",
        "шум",
        "тишина",
        "music",
        "applause",
        "выстрелы",
        "выстрел",
        "вздостный телефон",
        "крик",
        "крики",
        "взрыв",
        "взрывы",
        "стон",
        "стоны",
    ];
    let clean_exact = lower.trim_matches(['.', '!', '?', '-', '—', ' ']);
    if exact_tags.contains(&clean_exact) {
        return true;
    }

    let patterns = [
        "[музыка]",
        "(музыка)",
        "[music]",
        "(music)",
        "[аплодисменты]",
        "аплодисменты",
        "[смех]",
        "редактор субтитров",
        "корректор а.",
        "корректор в.",
        "корректор н.",
        "корректор е.",
        "корректор ",
        "синецкая",
        "семкин",
        "егорова",
        "сухиашвили",
        "александрова",
        "кулакова",
        "диматоржок",
        "dimatorzok",
        "субтитры сделал",
        "субтитры создавал",
        "субтитры подготовил",
        "субтитры подогнал",
        "субтитры добав",
        "amara.org",
        "амара.орг",
        "спокойная музыка",
        "динамичная музыка",
        "тревожная музыка",
        "играет музыка",
        "лирическая музыка",
        "веселая музыка",
        "грустная музыка",
        "напряженная музыка",
        "драматичная музыка",
        "эпичная музыка",
        "тихая музыка",
        "громкая музыка",
        "фоновая музыка",
        "интригующая музыка",
        "торжественная музыка",
        "зловещая музыка",
        "романтическая музыка",
        "музыкальная заставка",
        "музыкальная пауза",
        "позитивается",
        "вздостный телефон",
        "продолжение следует",
        "подпишись на канал",
        "ставьте лайки",
    ];

    for pat in &patterns {
        if lower.contains(pat) {
            return true;
        }
    }

    // Любая короткая ремарка (1–3 слова), содержащая слово «музыка» (например «ИНТРИГУЮЩАЯ МУЗЫКА»)
    let word_count = clean_exact.split_whitespace().count();
    if word_count <= 3 && (clean_exact.contains("музыка") || clean_exact.contains("music")) {
        return true;
    }

    // Повторяющиеся капс-слова музыкального описания (например «ГРУСТНАЯ МЕЛОДИЧЕСКАЯ ДИНАМИЧЕСКАЯ...»)
    if lower.matches("динамическая").count() >= 2
        || lower.matches("мелодическая").count() >= 2
        || lower.matches("музыка").count() >= 2
    {
        return true;
    }

    false
}

/// Очищает растянутые циклические гласные и слоги Whisper (например «Ооооооо...tragenа» или 20x «Ха-ха-ха...»),
/// а также сжимает зацикленные повторы одной и той же фразы внутри одной реплики
pub fn clean_stretched_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // 1. Сжимаем любые 5+ одинаковых символов подряд до 3 (например «Оооооооооо» -> «Ооо»)
    let mut compressed = String::with_capacity(trimmed.len());
    let mut prev_lower: Option<char> = None;
    let mut run_len = 0usize;
    let mut had_long_stretch = false;

    for ch in trimmed.chars() {
        let ch_low = ch.to_lowercase().next().unwrap_or(ch);
        if Some(ch_low) == prev_lower && ch.is_alphabetic() {
            run_len += 1;
            if run_len >= 12 {
                had_long_stretch = true;
            }
            if run_len <= 3 {
                compressed.push(ch);
            }
        } else {
            prev_lower = Some(ch_low);
            run_len = 1;
            compressed.push(ch);
        }
    }

    // Если была экстремальная растяжка крика (12+ одинаковых букв подряд) и в конец приклеился латинский мусор вроде «tragenа»
    if had_long_stretch {
        let has_cyrillic = compressed.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
        let has_latin = compressed.chars().any(|c| c.is_ascii_alphabetic());
        if has_cyrillic && has_latin {
            let clean_prefix: String = compressed
                .chars()
                .take_while(|c| !c.is_ascii_alphabetic())
                .collect();
            let clean_prefix = clean_prefix.trim();
            if !clean_prefix.is_empty() {
                return format!("{}!", clean_prefix.trim_end_matches(['.', ',', '-', ' ']));
            }
        }
    }

    // 2. Сжимаем бесконечные «Ха-ха-ха-ха-ха-ха...» до аккуратного «Ха-ха-ха!»
    let lower_comp = compressed.to_lowercase();
    if lower_comp.matches("-ха").count() > 3 {
        return "Ха-ха-ха!".to_string();
    }
    if lower_comp.matches("-хе").count() > 3 {
        return "Хе-хе-хе!".to_string();
    }
    if lower_comp.matches(", дай").count() >= 4 {
        return "Ай-ай-ай!".to_string();
    }

    compressed
}

/// Проверяет, содержит ли существующий SRT-файл зацикленные галлюцинации от старых сборок
fn has_hallucination_loops(srt_content: &str) -> bool {
    let fragments = crate::text_analyzer::parse_srt(srt_content);
    if fragments.is_empty() {
        return true;
    }

    let mut hallucination_count = 0usize;
    let mut consecutive_dups = 0usize;
    let mut max_consecutive_dups = 0usize;
    let mut prev_norm = String::new();

    for frag in &fragments {
        if is_whisper_hallucination(&frag.text) {
            hallucination_count += 1;
        }
        let norm = frag.text.trim().to_lowercase();
        if !norm.is_empty() && norm == prev_norm {
            consecutive_dups += 1;
            if consecutive_dups > max_consecutive_dups {
                max_consecutive_dups = consecutive_dups;
            }
        } else {
            consecutive_dups = 0;
            prev_norm = norm;
        }
    }

    // Если более 60% строк — галлюцинации или более 6 одинаковых строк подряд — требуется повторная транскрибация
    hallucination_count * 2 > fragments.len() || max_consecutive_dups >= 6
}

/// Проверяет, является ли токен типичным оторванным окончанием/суффиксом русского слова
/// (возникает, когда whisper.cpp с флагом -ml без -sow разрезает длинное слово пополам)
fn is_bound_russian_word_tail(w_prev_lower: &str, w_next_alpha_lower: &str) -> bool {
    if w_prev_lower.is_empty() || w_next_alpha_lower.is_empty() {
        return false;
    }

    // Ни одно настоящее русское слово не начинается с ь, ъ или ы
    if w_next_alpha_lower.starts_with('ь')
        || w_next_alpha_lower.starts_with('ъ')
        || w_next_alpha_lower.starts_with('ы')
    {
        return true;
    }

    // Короткие самостоятельные предлоги, союзы, частицы и местоимения, после которых ВСЕГДА идет отдельное слово через пробел
    let standalone_short_words = [
        "а", "и", "о", "у", "я", "в", "с", "к",
        "на", "по", "из", "за", "от", "до", "об", "во", "со", "ко",
        "не", "ни", "но", "да", "же", "ли", "бы", "то", "ну", "ах", "ох", "эх", "ух",
        "мы", "ты", "вы", "он", "она", "оно", "они", "кто", "что", "как", "где", "тут", "там",
        "вот", "все", "всё", "так", "уже", "еще", "ещё", "или", "для", "при", "про", "без",
        "над", "под", "это", "чем", "тем", "мой", "моя", "моё", "мои", "сам", "сама", "сами",
        "его", "ее", "её", "их", "нам", "вам", "им", "нас", "вас", "мне", "дни", "три", "два",
    ];

    // Безусловные морфологические хвосты русских слов (глагольные окончания, падежные суффиксы),
    // которые не употребляются как самостоятельные слова в начале фразы
    let bound_tails = [
        "ать", "ять", "ить", "еть", "уть", "ться", "тся", "аться", "яться", "иться", "еться",
        "аем", "яем", "имся", "емся", "ите", "ете", "ешь", "ишь", "ают", "яют", "уют", "юют",
        "ивают", "ывают", "аются", "яются", "ится", "ятся", "ается", "яется", "уется", "аешься",
        "ала", "яла", "ила", "ела", "ало", "яло", "ило", "ело", "али", "яли", "или", "ели",
        "ался", "ялся", "ился", "елся", "алась", "ялась", "илась", "елась", "ались", "ялись", "ились",
        "кий", "гой", "хой", "ный", "ной", "овой", "евой", "ового", "евого", "ленное", "асном",
        "ережнее", "овочку", "ойдет", "ено", "ерплохо", "ительности", "ельности", "равляющий",
        "равляющих", "ция", "новато", "ичек", "ности", "ерником", "ель", "удние", "орые", "роил",
        "орах", "уживающих", "ений", "торов", "астую", "ываетесь", "обрить", "ания", "плей",
        "роются", "ывать", "ся", "сь", "му", "цу", "ма", "уш", "ий", "ая", "яя", "ое", "ее",
        "ые", "ие", "ого", "ому", "ему", "ыми", "ими", "ях", "ом", "ем", "ам", "ям", "ов", "ев", "ей",
    ];

    // Особый случай: предлог «в» или «с» перед словом, но иногда whisper режет «вз|носы», «в|ойдет», «л|овочку», «п|унктуальность»
    if (w_prev_lower == "в" && w_next_alpha_lower == "ойдет")
        || (w_prev_lower == "л" && w_next_alpha_lower == "овочку")
        || (w_prev_lower == "п" && w_next_alpha_lower.starts_with("унктуальн"))
        || (w_prev_lower == "ф" && w_next_alpha_lower.starts_with("анатом"))
        || (w_prev_lower == "к" && w_next_alpha_lower == "уш")
        || (w_prev_lower == "м" && w_next_alpha_lower == "ена")
    {
        return true;
    }

    if bound_tails.contains(&w_next_alpha_lower) {
        return true;
    }

    // Если первое слово не входит в список самостоятельных коротких слов и обрывается на согласную,
    // а вторая часть — короткий осколок (<= 4 букв), не являющийся самостоятельным словом
    if !standalone_short_words.contains(&w_prev_lower)
        && !standalone_short_words.contains(&w_next_alpha_lower)
        && w_next_alpha_lower.chars().count() <= 4
    {
        return true;
    }

    false
}

/// Восстанавливает:
/// 1) слова и знаки препинания, разорванные на границе соседних SRT-блоков;
/// 2) короткие оторванные хвосты предложений (1–3 слова вроде «ты так.», «бою.», «нужна сила.», «вооружие.», «бойцом.»),
///    которые whisper.cpp отрезает в микро-реплику длительностью 0.3–0.9с из-за байтового лимита `-ml`.
fn repair_split_words_and_orphans(
    fragments: Vec<crate::text_analyzer::SubtitleFragment>,
) -> (Vec<crate::text_analyzer::SubtitleFragment>, usize) {
    if fragments.len() < 2 {
        return (fragments, 0);
    }

    let mut out: Vec<crate::text_analyzer::SubtitleFragment> = Vec::with_capacity(fragments.len());
    let mut repaired_count = 0usize;

    for frag in fragments {
        let mut curr_text = frag.text.trim().to_string();
        if curr_text.is_empty() {
            continue;
        }

        if let Some(prev) = out.last_mut() {
            let gap = (frag.start - prev.end).max(0.0);

            // 1. Если новая строка начинается с оторванного знака препинания (`.`, `!`, `?`, `,`, `;`, `:`, `…`)
            if let Some(first_ch) = curr_text.chars().next() {
                if matches!(first_ch, '.' | '!' | '?' | ',' | ';' | ':' | '…') && gap <= 0.35 {
                    prev.text.push(first_ch);
                    curr_text = curr_text[first_ch.len_utf8()..].trim_start().to_string();
                    repaired_count += 1;
                    if curr_text.is_empty() {
                        prev.end = prev.end.max(frag.end);
                        continue;
                    }
                }
            }

            // 2. Проверяем, не разрезано ли русское слово пополам на стыке `prev` и `curr` (gap <= 0.12с)
            let mut word_stitched = false;
            if gap <= 0.12 {
                let prev_ends_alpha = prev
                    .text
                    .chars()
                    .last()
                    .map(|c| ('\u{0400}'..='\u{04FF}').contains(&c) && c.is_alphabetic())
                    .unwrap_or(false);
                let curr_starts_lower_cyr = curr_text
                    .chars()
                    .next()
                    .map(|c| ('\u{0400}'..='\u{04FF}').contains(&c) && c.is_lowercase())
                    .unwrap_or(false);

                if prev_ends_alpha && curr_starts_lower_cyr {
                    let w_prev = prev
                        .text
                        .split_whitespace()
                        .last()
                        .unwrap_or("")
                        .trim_start_matches(['-', '—', '"', '«', '('])
                        .to_lowercase();
                    let w_next_raw = curr_text.split_whitespace().next().unwrap_or("").to_string();
                    let w_next_alpha: String = w_next_raw
                        .chars()
                        .take_while(|c| c.is_alphabetic())
                        .collect::<String>()
                        .to_lowercase();

                    if is_bound_russian_word_tail(&w_prev, &w_next_alpha) {
                        repaired_count += 1;
                        word_stitched = true;
                        let remainder = curr_text[w_next_raw.len()..].trim_start().to_string();
                        let combined_chars = prev.text.chars().count() + curr_text.chars().count();

                        // Если после склейки слова вторая строка пустая или короткая (общая длина <= 85 символов) —
                        // объединяем оба блока в одну цельную фразу!
                        if remainder.is_empty() || combined_chars <= 85 {
                            prev.text.push_str(&w_next_raw);
                            if !remainder.is_empty() {
                                prev.text.push(' ');
                                prev.text.push_str(&remainder);
                            }
                            prev.end = prev.end.max(frag.end);
                            continue;
                        } else {
                            // Иначе приклеиваем оторванный хвост слова к `prev`, а остаток фразы оставляем в `curr`
                            prev.text.push_str(&w_next_raw);
                            curr_text = remainder;
                        }
                    }
                }
            }

            // 3. Склеиваем короткие оторванные хвосты предложений (1–3 слова, до 24 символов, например «ты так.», «бою.», «нужна сила.», «вооружие.», «бойцом.»),
            // которые whisper.cpp перенес в конец сегмента при срабатывании лимита длины строки `-ml`
            if !word_stitched && gap <= 0.30 && !crate::text_analyzer::has_terminal_punctuation(&prev.text) {
                let curr_words = curr_text.split_whitespace().count();
                let curr_chars = curr_text.chars().count();
                let curr_dur = (frag.end - frag.start).max(0.0);
                let starts_dialog = curr_text.starts_with('-') || curr_text.starts_with('—');
                let starts_lower = curr_text
                    .chars()
                    .next()
                    .map(|c| c.is_lowercase())
                    .unwrap_or(false);

                let is_short_orphan_tail = !starts_dialog
                    && ((starts_lower && curr_words <= 3 && curr_chars <= 24 && curr_dur <= 1.6)
                        || (curr_words == 1 && curr_chars <= 14 && curr_dur <= 1.0 && gap <= 0.08));

                if is_short_orphan_tail && (prev.text.chars().count() + 1 + curr_chars) <= 95 {
                    prev.text.push(' ');
                    prev.text.push_str(&curr_text);
                    prev.end = prev.end.max(frag.end);
                    repaired_count += 1;
                    continue;
                }
            }
        }

        out.push(crate::text_analyzer::SubtitleFragment {
            start: frag.start,
            end: frag.end,
            text: curr_text,
        });
    }

    (out, repaired_count)
}

/// Очищает SRT-файл от мусорных галлюцинаций Whisper, зацикленных повторов и разорванных полуслов/хвостов фраз
pub async fn sanitize_srt_file(srt_path: &Path, app_handle: &AppHandle) -> AppResult<usize> {
    let content = tokio::fs::read_to_string(srt_path).await.map_err(AppError::Io)?;
    let raw_fragments = crate::text_analyzer::parse_srt(&content);
    let raw_count = raw_fragments.len();

    let mut stage1: Vec<crate::text_analyzer::SubtitleFragment> = Vec::with_capacity(raw_count);
    for mut frag in raw_fragments {
        let dur = frag.end - frag.start;
        if dur <= 0.10 {
            continue;
        }

        let trimmed = frag.text.trim();
        if is_whisper_hallucination(trimmed) {
            continue;
        }

        let cleaned_text = clean_stretched_text(trimmed);
        if cleaned_text.is_empty() || is_whisper_hallucination(&cleaned_text) {
            continue;
        }
        frag.text = cleaned_text;
        stage1.push(frag);
    }

    // Фильтруем зацикленные повторы Whisper (например 6x «Я не могу сказать, что ты нанесешься» на музыке эндинга
    // или 5x «У нас не штатная ситуация!» подряд)
    let mut clean_fragments: Vec<crate::text_analyzer::SubtitleFragment> = Vec::with_capacity(stage1.len());
    let mut i = 0;
    while i < stage1.len() {
        let norm_i = stage1[i].text.trim().to_lowercase();
        let prefix_i: String = norm_i.chars().take(18).collect();
        let words_i = norm_i.split_whitespace().count();

        // Считаем, сколько раз подряд повторяется эта же фраза (или 18-символьный префикс для длинных галлюцинаций песни)
        let mut run_end = i + 1;
        while run_end < stage1.len() {
            let norm_k = stage1[run_end].text.trim().to_lowercase();
            let same_exact = norm_k == norm_i;
            let same_long_prefix = words_i >= 4
                && prefix_i.chars().count() >= 18
                && norm_k.starts_with(&prefix_i);
            if same_exact || same_long_prefix {
                run_end += 1;
            } else {
                break;
            }
        }

        let run_len = run_end - i;
        if run_len >= 5 && words_i >= 4 {
            // Если длинная фраза (>= 4 слов) зациклилась 5+ раз подряд (как «Я не могу сказать, что ты нанесешься» x7 в эндинге) —
            // это 100% галлюцинация Whisper поверх вокала песни, отбрасываем весь зацикленный кластер!
            i = run_end;
            continue;
        } else if run_len >= 2 {
            // Для обычных фраз оставляем только 1 экземпляр без дублирования
            clean_fragments.push(stage1[i].clone());
            i = run_end;
            continue;
        } else {
            clean_fragments.push(stage1[i].clone());
            i += 1;
        }
    }

    let clean_fragments = deduplicate_multiline_loops(clean_fragments);
    let removed = raw_count.saturating_sub(clean_fragments.len());
    let (clean_fragments, stitched_words) = repair_split_words_and_orphans(clean_fragments);

    if removed > 0 || stitched_words > 0 {
        crate::logger::emit_log(
            app_handle,
            "Whisper",
            "info",
            &format!(
                "Очистка субтитров: удалено {} мусорных/дублирующихся строк, склеено {} разорванных полуслов/фраз, итого {} чистых реплик.",
                removed,
                stitched_words,
                clean_fragments.len()
            ),
        );
    }

    let mut out = String::new();
    for (idx, frag) in clean_fragments.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            idx + 1,
            format_srt_timestamp(frag.start),
            format_srt_timestamp(frag.end),
            frag.text.trim()
        ));
    }

    tokio::fs::write(srt_path, out).await.map_err(AppError::Io)?;
    Ok(clean_fragments.len())
}

/// Убирает многострочные зацикленные повторы Whisper:
/// 1) Блочные повторы ABAB / ABCABC из 2..=6 строк подряд (например, когда 5 реплик диалога повторяются дважды подряд).
/// 2) Слитные эхо-повторы, когда Whisper объединяет предыдущие 2–3 короткие реплики в одну слитную строку-дубликат.
fn deduplicate_multiline_loops(
    fragments: Vec<crate::text_analyzer::SubtitleFragment>,
) -> Vec<crate::text_analyzer::SubtitleFragment> {
    if fragments.len() < 4 {
        return fragments;
    }

    fn norm_alpha(s: &str) -> String {
        s.chars()
            .filter(|c| c.is_alphabetic() || c.is_whitespace())
            .collect::<String>()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    let norms: Vec<String> = fragments.iter().map(|f| norm_alpha(&f.text)).collect();
    let mut out: Vec<crate::text_analyzer::SubtitleFragment> = Vec::with_capacity(fragments.len());
    let mut out_norms: Vec<String> = Vec::with_capacity(fragments.len());

    let mut i = 0usize;
    while i < fragments.len() {
        // 1. Проверяем блочный повтор длины k (от 2 до 6 строк)
        let mut skipped_block_len = 0usize;
        for k in (2..=6).rev() {
            if out_norms.len() >= k && i + k <= fragments.len() {
                let prev_block = &out_norms[out_norms.len() - k..];
                let curr_block = &norms[i..i + k];
                let all_match = prev_block
                    .iter()
                    .zip(curr_block.iter())
                    .all(|(a, b)| {
                        !a.is_empty()
                            && (a == b
                                || (a.chars().count() >= 10
                                    && b.chars().count() >= 10
                                    && a.chars().take(10).eq(b.chars().take(10))))
                    });
                if all_match {
                    skipped_block_len = k;
                    break;
                }
            }
        }
        if skipped_block_len > 0 {
            i += skipped_block_len;
            continue;
        }

        // 2. Проверяем слитный эхо-повтор: когда текущая длинная строка повторяет подряд склеенные последние 2 или 3 реплики
        let curr_words: Vec<&str> = norms[i].split_whitespace().collect();
        if curr_words.len() >= 6 && out_norms.len() >= 2 {
            let mut is_merged_echo = false;
            for m in 2..=3 {
                if out_norms.len() >= m {
                    let combined_prev = out_norms[out_norms.len() - m..].join(" ");
                    let prev_words: Vec<&str> = combined_prev.split_whitespace().collect();
                    if prev_words.len() >= 5 {
                        let matched = curr_words
                            .iter()
                            .filter(|&&cw| {
                                prev_words.iter().any(|&pw| {
                                    cw == pw
                                        || (cw.chars().count() >= 4
                                            && pw.chars().count() >= 4
                                            && cw.chars().take(4).eq(pw.chars().take(4)))
                                })
                            })
                            .count();
                        if matched * 10 >= curr_words.len() * 7 {
                            is_merged_echo = true;
                            break;
                        }
                    }
                }
            }
            if is_merged_echo {
                i += 1;
                continue;
            }
        }

        out.push(fragments[i].clone());
        out_norms.push(norms[i].clone());
        i += 1;
    }

    out
}

pub fn format_srt_timestamp(seconds: f64) -> String {
    let clamped = seconds.max(0.0);
    let total_ms = (clamped * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_secs = total_ms / 1000;
    let s = total_secs % 60;
    let m = (total_secs / 60) % 60;
    let h = total_secs / 3600;
    format!("{:02}:{:02}:{:02},{:03}", h, m, s, ms)
}

/// Умное получение или создание субтитров для видеофайла с кешированием в папке проекта.
/// Предотвращает повторный запуск FFmpeg extract_audio и Whisper при переходе от анализа к экспорту клипов!
pub async fn get_or_create_subtitles(
    video_path: PathBuf,
    model_path: Option<PathBuf>,
    app_handle: AppHandle,
    cancel_rx: Receiver<()>,
) -> AppResult<Option<PathBuf>> {
    let project_srt = get_project_srt_path(&video_path);

    // 1. Проверяем, есть ли уже готовые субтитры для этого видео в папке проекта
    if project_srt.exists() {
        // Всегда прогоняем быструю санитизацию кэшированного SRT (удаляет [музыка], титры редакторов и растяжки букв)
        let _ = sanitize_srt_file(&project_srt, &app_handle).await;
        if let Ok(content) = tokio::fs::read_to_string(&project_srt).await {
            if !content.trim().is_empty() && !has_hallucination_loops(&content) {
                let count = crate::text_analyzer::parse_srt(&content).len();
                if count > 0 {
                    crate::logger::emit_log(
                        &app_handle,
                        "Whisper",
                        "info",
                        &format!(
                            "✓ Найдены готовые чистые субтитры в папке проекта ({} реплик): '{}'. Повторная транскрибация не требуется!",
                            count,
                            project_srt.display()
                        ),
                    );
                    let _ = app_handle.emit(
                        "whisper-progress",
                        WhisperProgressPayload {
                            percent: 100.0,
                            stage: "cached".to_string(),
                        },
                    );
                    return Ok(Some(project_srt));
                }
            } else if has_hallucination_loops(&content) {
                crate::logger::emit_log(
                    &app_handle,
                    "Whisper",
                    "warn",
                    "В старом файле субтитров обнаружены зацикленные галлюцинации. Выполняется чистая повторная транскрибация (-mc 0)...",
                );
            }
        }
    }

    // 2. Если готовых субтитров нет — извлекаем временное аудио 16kHz WAV и запускаем Whisper
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let audio_output = std::env::temp_dir().join(format!("vc_audio_{}_{}.wav", std::process::id(), ts));

    crate::ffmpeg::extract_audio(
        video_path.clone(),
        audio_output.clone(),
        app_handle.clone(),
        cancel_rx.resubscribe(),
    )
    .await?;

    let transcribe_res = transcribe_audio(
        audio_output.clone(),
        model_path,
        app_handle.clone(),
        cancel_rx.resubscribe(),
    )
    .await;

    // Сразу удаляем временный WAV-файл
    let _ = tokio::fs::remove_file(&audio_output).await;

    match transcribe_res {
        Ok(temp_srt) => {
            if let Some(parent) = project_srt.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            if tokio::fs::copy(&temp_srt, &project_srt).await.is_ok() {
                let _ = tokio::fs::remove_file(&temp_srt).await;
                crate::logger::emit_log(
                    &app_handle,
                    "Whisper",
                    "info",
                    &format!("Субтитры сохранены в кэш проекта: '{}'", project_srt.display()),
                );
                Ok(Some(project_srt))
            } else {
                Ok(Some(temp_srt))
            }
        }
        Err(e) => {
            crate::logger::emit_log(
                &app_handle,
                "Whisper",
                "warn",
                &format!("Whisper ASR завершился с предупреждением: {}", e),
            );
            Ok(None)
        }
    }
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

    // Ищем выбранную модель Whisper или любую установленную в папке models
    let preferred_path = model_path.as_ref().map(|p| p.to_string_lossy().to_string());
    let model = crate::models::resolve_any_whisper_model(&app_handle, preferred_path.as_deref());
    
    let model = match model {
        Some(m) if m.exists() => {
            crate::logger::emit_log(
                &app_handle,
                "Whisper",
                "info",
                &format!("Используется модель распознавания речи: '{}'", m.display()),
            );
            m
        },
        _ => {
            let err_msg = "Модель распознавания речи Whisper не найдена в папке models. Пожалуйста, скачайте любую модель Whisper в Менеджере ИИ-Моделей.".to_string();
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
        &format!("Запуск whisper.cpp (модель: {}, язык: auto, anti-loop: -mc 0, вывод: {}.srt)...", model.display(), base_out),
    );

    // Передаем:
    // -l auto: автоопределение русского/любого языка
    // -mc 0: отключение переноса текстового контекста между 30с окнами (предотвращает 7-минутные циклы «Корректор А. Кулак» после музыки!)
    // -et 2.2: порог энтропии для подавления повторов
    // -ml 150 -sow: в whisper.cpp -ml измеряется в байтах UTF-8 (150 байт = ~75 русских букв) + разбиение строго по целым словам (-sow)
    // -pp: вывод процентов прогресса
    let command = command
        .env("PATH", crate::logger::build_sidecar_env_path(&app_handle))
        .args([
            "-m", &model.to_string_lossy(),
            "-l", "auto",
            "-mc", "0",
            "-et", "2.2",
            "-ml", "150",
            "-sow",
            "-pp",
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
    let expected_srt_clone = expected_srt_output.clone();
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
                        // Очищаем сгенерированный SRT от галлюцинаций и повторов
                        if expected_srt_clone.exists() {
                            let _ = sanitize_srt_file(&expected_srt_clone, &app_handle_clone).await;
                        }
                        crate::logger::emit_log(
                            &app_handle_clone,
                            "Whisper",
                            "info",
                            &format!("Распознавание речи успешно завершено! Субтитры сохранены в '{}'", expected_srt_clone.display()),
                        );
                        let _ = app_handle_clone.emit("whisper-progress", WhisperProgressPayload { 
                            percent: 100.0, 
                            stage: "saving".to_string() 
                        });
                        return Ok(expected_srt_clone);
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
