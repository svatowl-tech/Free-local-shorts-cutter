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

    // Отсекаем технические теги музыки и фонового шума вида [музыка], (музыка), *музыка*, ♪ и т.д.
    if (lower.starts_with('[') && lower.ends_with(']'))
        || (lower.starts_with('(') && lower.ends_with(')'))
        || (lower.starts_with('*') && lower.ends_with('*'))
        || lower.contains('♪')
        || lower.contains('♫')
    {
        return true;
    }

    // Одиночные звуковые ремарки КАПСОМ, со звездочками или без скобок
    let exact_tags = [
        "музыка",
        "звучит музыка",
        "музыка звучит",
        "играет музыка",
        "аплодисменты",
        "смех",
        "смеется",
        "плачет",
        "вздох",
        "вздыхает",
        "кашель",
        "кашляет",
        "зевает",
        "храпит",
        "рычит",
        "шипит",
        "кричит",
        "поет",
        "поёт",
        "звук",
        "шум",
        "тишина",
        "music",
        "applause",
        "выстрелы",
        "выстрел",
        "перестрелка",
        "вздостный телефон",
        "сигналы",
        "сигнал",
        "стук в дверь",
        "стук",
        "шаги",
        "крик",
        "крики",
        "взрыв",
        "взрывы",
        "стон",
        "стоны",
        "всхлип",
        "всхлипы",
        "всхлипым",
        "песня всхлипым",
        "песня",
        "what is this",
    ];
    let clean_exact = lower.trim_matches(['.', '!', '?', ',', '-', '—', '*', '"', '\'', '«', '»', ' ']);
    if exact_tags.contains(&clean_exact) {
        return true;
    }

    let patterns = [
        "[музыка]",
        "(музыка)",
        "*музыка*",
        "(звучит музыка)",
        "[звучит музыка]",
        "*звучит музыка*",
        "[music]",
        "(music)",
        "[аплодисменты]",
        "аплодисменты",
        "[смех]",
        "(поет)",
        "(поёт)",
        "[поет]",
        "[поёт]",
        "*поет*",
        "*поёт*",
        "песня всхлип",
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
        "закомолдина",
        "савельева",
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
        "печальная музыка",
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
        "let's go to the play",
        "валерий курас",
        "позитивный голос",
        "позитивный позитивный",
        "speaking russian",
        "speaking foreign",
        "шови, ма",
        "шови, мол",
        "хай, баку",
        "циклсовалею",
        "кинофотезе",
        "смей за сентябрье",
    ];

    for pat in &patterns {
        if lower.contains(pat) {
            return true;
        }
    }

    // Детектор вокальных зацикливаний из однобуквенных/двухбуквенных гласных и междометий на песнях
    // (например «а, а, а, а, а, а...», «Возьмите несколько двоих, а, а, а, а, а, а, а...»)
    let alpha_tokens: Vec<String> = lower
        .split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphabetic()).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect();
    if alpha_tokens.len() >= 2 {
        let short_vowel_tokens = alpha_tokens
            .iter()
            .filter(|w| {
                matches!(
                    w.as_str(),
                    "а" | "о" | "у" | "э" | "и" | "ы" | "я" | "е" | "м" | "н" | "аа" | "оо" | "ах" | "ох" | "эх"
                )
            })
            .count();
        // Если строка целиком состоит из «а, а» / «а, а, а» (например после сжатия цикла) или содержит >= 5 таких повторов
        if short_vowel_tokens >= 5
            || (alpha_tokens.len() >= 2 && short_vowel_tokens == alpha_tokens.len())
            || (alpha_tokens.len() >= 3 && short_vowel_tokens + 1 >= alpha_tokens.len() && alpha_tokens.last().map_or(false, |w| w == "я"))
        {
            return true;
        }
    }

    // Фонетическая транслитерация японского/иностранного вокала опенинга и эндинга кириллицей
    // (например «Шови, ма, хай, баку, и сейса, то-то вага-то, мураши, са, и демур, ги-се,»):
    // характеризуется обилием запятых (>= 5) между короткими слогами и дефисными псевдословами («то-то», «вага-то», «ги-се»)
    let comma_count = lower.matches(',').count();
    let hyphen_words = lower
        .split_whitespace()
        .filter(|w| {
            let trimmed_w = w.trim_matches([',', '.', '!', '?', '-', '—']);
            trimmed_w.contains('-') && trimmed_w.chars().count() <= 8
        })
        .count();
    if (comma_count >= 5 && hyphen_words >= 2) || comma_count >= 7 {
        return true;
    }

    // Шаблонные титры песен вида «Исполнитель - "Название"» на музыке опенинга/эндинга
    if (trimmed.contains(" - \"") || trimmed.contains(" — \"")) && trimmed.ends_with('"') {
        return true;
    }

    // Любая короткая ремарка (1–3 слова), содержащая слово «музыка» или «песня» (например «ИНТРИГУЮЩАЯ МУЗЫКА», «ПЕСНЯ ВСХЛИПЫМ»)
    let word_count = clean_exact.split_whitespace().count();
    if word_count <= 3
        && (clean_exact.contains("музыка")
            || clean_exact.contains("music")
            || clean_exact.starts_with("песня "))
    {
        return true;
    }

    // Капс-ремарки звукового описания из >= 2 слов (например «ПОЗИТИВНЫЙ ГОЛОС», «ГРОМКИЙ ШУМ»)
    let has_upper_cyr = trimmed.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c) && c.is_uppercase());
    let has_lower_cyr = trimmed.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c) && c.is_lowercase());
    if has_upper_cyr
        && !has_lower_cyr
        && word_count >= 2
        && (clean_exact.contains("голос")
            || clean_exact.contains("звук")
            || clean_exact.contains("шум")
            || clean_exact.contains("песня")
            || clean_exact.contains("музыка")
            || clean_exact.contains("мелодия")
            || clean_exact.contains("позитивный"))
    {
        return true;
    }

    // Повторяющиеся капс-слова музыкального описания (например «ГРУСТНАЯ МЕЛОДИЧЕСКАЯ ДИНАМИЧЕСКАЯ...»)
    if lower.matches("динамическая").count() >= 2
        || lower.matches("мелодическая").count() >= 2
        || lower.matches("музыка").count() >= 2
    {
        return true;
    }

    // Фонетические галлюцинации на вокале опенинга/эндинга («Повторяй на Сент-Чьи», «Повторяй на Вище», «Возьми несколько зажиманий!»)
    if lower.starts_with("повторяй на сент")
        || lower.starts_with("повторяй на вищ")
        || lower.contains("возьми несколько зажиманий")
        || lower.contains("сисло, то прайд")
        || lower.contains("по, вис, о, ю, тю")
    {
        return true;
    }

    false
}

/// Исправляет слипшиеся слова и устойчивые фонетические искажения Whisper на русском дубляже
pub fn fix_common_whisper_glued_words(text: &str) -> String {
    let glued_patterns: &[(&str, &str)] = &[
        ("недостаточносил", "недостаточно сил"),
        ("вражескихсил", "вражеских сил"),
        ("боевыхсил", "боевых сил"),
        ("нашихсил", "наших сил"),
        ("вооруженныхсил", "вооружённых сил"),
        ("недостаточновремени", "недостаточно времени"),
        ("недостаточносредств", "недостаточно средств"),
        ("несмог", "не смог"),
        ("несмогла", "не смогла"),
        ("несмогли", "не смогли"),
        ("немогу", "не могу"),
        ("неможет", "не может"),
        ("неможем", "не можем"),
        ("немогут", "не могут"),
        ("незнаю", "не знаю"),
        ("незнает", "не знает"),
        ("такжекак", "так же как"),
        ("досихпор", "до сих пор"),
        ("втожевремя", "в то же время"),
        ("вконцеконцов", "в конце концов"),
        ("покрайнеймере", "по крайней мере"),
        ("другдруга", "друг друга"),
        ("другдругу", "друг другу"),
        ("другсдругом", "друг с другом"),
        ("тожесамое", "то же самое"),
        ("всееще", "всё ещё"),
        ("всёеще", "всё ещё"),
        ("ничегоне", "ничего не"),
        ("никогдане", "никогда не"),
        ("оберез лейтенант сфотографируй", "обер-лейтенант фон Дегуршаф"),
        ("оберез лейтенант фон дегуршав", "обер-лейтенант фон Дегуршаф"),
        ("оберс лейтенант фонди гуршав", "обер-лейтенант фон Дегуршаф"),
        ("уберс лейтенант фонди гуршав", "обер-лейтенант фон Дегуршаф"),
        ("оберстлетенанта фонди грушав", "обер-лейтенанта фон Дегуршаф"),
        ("тефон дегуршав", "фон Дегуршаф"),
        ("фонди гуршав", "фон Дегуршаф"),
        ("фонди грушав", "фон Дегуршаф"),
        ("дегушав", "Дегуршаф"),
        ("госпожаобес лейтенант", "госпожа обер-лейтенант"),
        ("убер с лейтенанфан югер", "оберст-лейтенант фон Угар"),
        ("оберс лейтенант фун югер", "оберст-лейтенант фон Угар"),
        ("оберез лейтенант", "обер-лейтенант"),
        ("оберс лейтенант", "обер-лейтенант"),
        ("обер с лейтенант", "обер-лейтенант"),
        ("уберс лейтенант", "обер-лейтенант"),
        ("убер, лейтенант", "обер-лейтенант"),
        ("увес, лейтенант", "обер-лейтенант"),
        ("образ, лейтенант", "обер-лейтенант"),
        ("рейтинан сью, довольна", "лейтенант Сью, довольно"),
        ("рейтинан сью", "лейтенант Сью"),
        ("прошеские колдуне", "вражеские колдуны"),
        ("ражеское подкрепление", "вражеское подкрепление"),
        ("рынского дьявола", "Рейнского дьявола"),
        ("рынский дьявол", "Рейнский дьявол"),
        ("гаинский дьевол", "Рейнский дьявол"),
        ("поохвально", "похвально"),
        ("какое тончанное предложение", "какое утончённое предложение"),
        ("я полевая командир", "я полевой командир"),
        ("какие доказательки", "какие доказательства"),
        ("к стадии... желению мирной переговоры", "к сожалению, мирные переговоры"),
        ("к стадии... желению", "к сожалению,"),
        ("желению мирной переговоры", "к сожалению, мирные переговоры"),
        ("к стадии...", "кстати..."),
        ("квот сюда", "вот сюда"),
        ("военная докприна", "военная доктрина"),
        ("запушечное мясо", "за пушечное мясо"),
    ];

    let mut result = text.to_string();
    for &(glued, fixed) in glued_patterns {
        let lower = result.to_lowercase().replace('ё', "е");
        if let Some(pos) = lower.find(glued) {
            if pos <= result.len() && pos + glued.len() <= result.len() {
                let is_capitalized = result[pos..].chars().next().map_or(false, |c| c.is_uppercase());
                let replacement = if is_capitalized {
                    let mut chars = fixed.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => fixed.to_string(),
                    }
                } else {
                    fixed.to_string()
                };
                result = result[..pos].to_string() + &replacement + &result[pos + glued.len()..];
            }
        }
    }
    result
}

/// Устраняет артефакты заикания / вдоха Whisper с висячей запятой, дефисом или многоточием перед словом
/// (например «Г, госпожа Бреслий!» -> «Госпожа Бреслий!», «В… Воды?» -> «Воды?», «П-поверьте...» -> «Поверьте...», «Н-н-н-н-н-н, что» -> «На что»)
pub fn fix_whisper_stutters(text: &str) -> String {
    let mut words: Vec<String> = text.split_whitespace().map(|w| w.to_string()).collect();
    if words.is_empty() {
        return text.to_string();
    }

    let mut changed = false;

    // 1. Внутрисловные дефисные заикания («П-поверьте...» -> «Поверьте...», «Н-н-н-н-н-н,» -> удаляем или схлопываем)
    let mut k = 0;
    while k < words.len() {
        let w = words[k].clone();
        let lead_punct: String = w.chars().take_while(|c| matches!(c, '-' | '—' | '«' | '"' | '(')).collect();
        let body = &w[lead_punct.len()..];
        if body.contains('-') {
            let parts: Vec<&str> = body.split('-').collect();
            if parts.len() >= 2 {
                let first_alpha: String = parts[0].chars().filter(|c| c.is_alphabetic()).collect();
                // Многократное заикание одной буквы через дефис: «Н-н-н-н-н-н,»
                let all_single_same = first_alpha.chars().count() == 1
                    && parts.iter().all(|p| {
                        let pa: String = p.chars().filter(|c| c.is_alphabetic()).collect();
                        pa.chars().count() == 1 && pa.to_lowercase() == first_alpha.to_lowercase()
                    });
                if all_single_same && parts.len() >= 3 {
                    if k + 1 < words.len() && first_alpha.to_lowercase() == "н" && words[k + 1].to_lowercase().starts_with("что") {
                        words[k] = format!("{}На", lead_punct);
                    } else {
                        words.remove(k);
                        changed = true;
                        continue;
                    }
                    changed = true;
                } else if first_alpha.chars().count() == 1 && parts.len() == 2 {
                    // Одиночный дефисный повтор первой буквы: «П-поверьте...» -> «Поверьте...»
                    let second_part = parts[1];
                    let second_first = second_part.chars().find(|c| c.is_alphabetic()).unwrap_or(' ');
                    if first_alpha.to_lowercase() == second_first.to_lowercase().to_string() && second_part.chars().count() >= 3 {
                        let is_cap = first_alpha.chars().next().map_or(false, |c| c.is_uppercase());
                        let mut sec_chars = second_part.chars();
                        let rebuilt_second = if is_cap {
                            match sec_chars.next() {
                                Some(fc) => fc.to_uppercase().collect::<String>() + sec_chars.as_str(),
                                None => second_part.to_string(),
                            }
                        } else {
                            second_part.to_string()
                        };
                        words[k] = format!("{}{}", lead_punct, rebuilt_second);
                        changed = true;
                    }
                }
            }
        }
        k += 1;
    }

    if words.len() < 2 {
        return if changed { words.join(" ") } else { text.to_string() };
    }

    let mut i = 0;
    while i + 1 < words.len() {
        let w1 = &words[i];
        let w2 = &words[i + 1];

        let w1_trimmed = w1.trim_start_matches(['-', '—', '«', '"']);
        let w1_alpha: String = w1_trimmed.chars().take_while(|c| c.is_alphabetic()).collect();
        let w1_punct = &w1_trimmed[w1_alpha.len()..];

        // Случай одиночной буквы с запятой, точкой или многоточием: «Г,», «В…», «П...»
        if w1_alpha.chars().count() == 1
            && matches!(w1_punct, "," | "…" | "..." | ".." | ".")
        {
            let letter1 = w1_alpha.chars().next().unwrap_or(' ');
            let letter2 = w2.chars().find(|c| c.is_alphabetic()).unwrap_or(' ');

            if letter1.is_alphabetic()
                && letter1.to_lowercase().to_string() == letter2.to_lowercase().to_string()
            {
                if letter1.is_uppercase() && !w2.is_empty() {
                    let mut chars = w2.chars();
                    if let Some(first) = chars.next() {
                        words[i + 1] = first.to_uppercase().collect::<String>() + chars.as_str();
                    }
                }
                words.remove(i);
                changed = true;
                continue;
            }
        }
        i += 1;
    }

    if changed {
        words.join(" ")
    } else {
        text.to_string()
    }
}

/// Очищает растянутые циклические гласные и слоги Whisper (например «Ооооооо...tragenа» или 20x «Ха-ха-ха...»),
/// сжимает зацикленные повторы одного и того же слова внутри одной реплики (например «я иду, иду, иду, иду...» x18),
/// а также исправляет слипшиеся слова и артефакты заикания.
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

    // 3. Сжимаем внутристрочные зацикливания одного слова (например «И сразу же, я иду, иду, иду, иду...» 15 раз подряд или «Рену! Рену! Рену! Рену!»)
    let raw_tokens: Vec<&str> = compressed.split_whitespace().collect();
    if raw_tokens.len() >= 4 {
        let mut dedup_tokens: Vec<&str> = Vec::with_capacity(raw_tokens.len());
        let mut last_norm = String::new();
        let mut word_run = 0usize;
        let mut had_word_loop = false;

        for tok in raw_tokens {
            let norm: String = tok
                .chars()
                .filter(|c| c.is_alphabetic())
                .collect::<String>()
                .to_lowercase();
            if !norm.is_empty() && norm == last_norm {
                word_run += 1;
                // Для однобуквенных гласных («а, а, а, а...») отсекаем уже со 2-го повтора, для обычных слов — с 3-го
                let max_allowed = if norm.chars().count() == 1 { 1 } else { 2 };
                if word_run > max_allowed {
                    had_word_loop = true;
                    continue;
                }
            } else {
                last_norm = norm;
                word_run = 1;
            }
            dedup_tokens.push(tok);
        }

        if had_word_loop {
            let mut rebuilt = dedup_tokens.join(" ");
            if rebuilt.ends_with(',') {
                rebuilt.pop();
                rebuilt.push('!');
            }
            compressed = rebuilt;
        }
    }

    // 4. Нормализация избыточных точек и артефактов пунктуации
    let mut normalized = compressed
        .replace(".....", "...")
        .replace("....", "...")
        .replace(",.", ".")
        .replace(",!", "!")
        .replace(",?", "?");

    // 5. Устраняем артефакты заикания («Г, госпожа» -> «Госпожа»)
    normalized = fix_whisper_stutters(&normalized);

    // 6. Исправляем слипшиеся слова («недостаточносил» -> «недостаточно сил»)
    fix_common_whisper_glued_words(&normalized)
}

/// Проверяет, содержит ли существующий SRT-файл зацикленные галлюцинации от старых сборок
/// или ошибочную англоязычную транскрибацию (когда `-l auto` определял `en` по песне опенинга и переводил русский дубляж на английский)
fn check_cached_srt_validity(srt_content: &str) -> (bool, bool) {
    let fragments = crate::text_analyzer::parse_srt(srt_content);
    if fragments.is_empty() {
        return (true, false);
    }

    let lower_all = srt_content.to_lowercase();
    if lower_all.contains("[speaking russian]") || lower_all.contains("(speaking russian)") {
        return (false, true);
    }

    let mut hallucination_count = 0usize;
    let mut consecutive_dups = 0usize;
    let mut max_consecutive_dups = 0usize;
    let mut cyrillic_lines = 0usize;
    let mut prev_norm = String::new();

    for frag in &fragments {
        if is_whisper_hallucination(&frag.text) {
            hallucination_count += 1;
        }
        if frag.text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)) {
            cyrillic_lines += 1;
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

    let has_loops = hallucination_count * 2 > fragments.len() || max_consecutive_dups >= 6;
    // Если в кэшированном SRT >= 5 строк, но менее 20% содержат кириллицу (например 0 из 237 после `-l auto` на английском опенинге),
    // считаем кэш испорченным неверным автоопределением языка и перезапускаем транскрибацию с `-l ru`!
    let wrong_lang = fragments.len() >= 5 && cyrillic_lines * 5 < fragments.len();

    (has_loops, wrong_lang)
}

/// Проверяет, является ли токен типичным оторванным окончанием/суффиксом русского слова
/// (возникает, когда whisper.cpp с флагом -ml без -sow разрезает длинное слово пополам)
fn is_bound_russian_word_tail(w_prev_lower: &str, w_next_alpha_lower: &str) -> bool {
    if w_prev_lower.is_empty() || w_next_alpha_lower.is_empty() {
        return false;
    }

    // Особый случай: предлог/первая буква перед разорванным словом («в|ойдет», «л|овочку», «п|унктуальность», «ф|анатом»)
    if (w_prev_lower == "в" && w_next_alpha_lower == "ойдет")
        || (w_prev_lower == "л" && w_next_alpha_lower == "овочку")
        || (w_prev_lower == "п" && w_next_alpha_lower.starts_with("унктуальн"))
        || (w_prev_lower == "ф" && w_next_alpha_lower.starts_with("анатом"))
        || (w_prev_lower == "к" && w_next_alpha_lower == "уш")
        || (w_prev_lower == "м" && w_next_alpha_lower == "ена")
    {
        return true;
    }

    // Ни одно настоящее русское слово не начинается с ь, ъ или ы
    if w_next_alpha_lower.starts_with('ь')
        || w_next_alpha_lower.starts_with('ъ')
        || w_next_alpha_lower.starts_with('ы')
    {
        return true;
    }

    // Короткие самостоятельные предлоги, союзы, частицы, местоимения и существительные,
    // которые НИКОГДА не должны склеиваться без пробела с предыдущим словом (предотвращает «вражескихсил»)
    let standalone_short_words = [
        "а", "и", "о", "у", "я", "в", "с", "к",
        "на", "по", "из", "за", "от", "до", "об", "во", "со", "ко",
        "не", "ни", "но", "да", "же", "ли", "бы", "то", "ну", "ах", "ох", "эх", "ух",
        "мы", "ты", "вы", "он", "она", "оно", "они", "кто", "что", "как", "где", "тут", "там",
        "вот", "все", "всё", "так", "уже", "еще", "ещё", "или", "для", "при", "про", "без",
        "над", "под", "это", "чем", "тем", "мой", "моя", "моё", "мои", "сам", "сама", "сами",
        "его", "ее", "её", "ей", "ему", "ими", "их", "нам", "вам", "им", "нас", "вас", "мне",
        "дни", "дня", "три", "два", "сил", "боя", "бою", "бой", "раз", "год", "лет", "шаг",
        "дом", "мир", "путь", "враг", "полк", "штаб", "фронт", "тыл", "план", "цель", "речь",
        "меч", "щит", "зал", "род", "вид", "ряд", "час", "век", "дух", "чай", "чая",
    ];

    if standalone_short_words.contains(&w_prev_lower)
        || standalone_short_words.contains(&w_next_alpha_lower)
    {
        return false;
    }

    // Если предыдущее слово уже имеет законченное прилагательное/причастное окончание («вражеских», «боевых», «новый»),
    // то следующее слово — это отдельное существительное, а не хвост того же слова!
    let complete_word_endings = [
        "их", "ых", "ого", "его", "ому", "ему", "ыми", "ими", "ая", "яя", "ое", "ее", "ые", "ие",
    ];
    if w_prev_lower.chars().count() >= 5
        && complete_word_endings.iter().any(|ext| w_prev_lower.ends_with(ext))
    {
        return false;
    }

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
        "роются", "ывать", "ся", "сь", "му", "цу", "ма", "уш", "ий", "ая", "яя", "ое",
        "ые", "ие", "ого", "ому", "ыми", "ях", "ом", "ам", "ям", "ов", "ев",
    ];

    if bound_tails.contains(&w_next_alpha_lower) {
        return true;
    }

    // Если первая часть — короткий осколок из одних согласных без гласных (например «вз» + «носы», «сп» + «рашивать»)
    let has_vowel = w_prev_lower
        .chars()
        .any(|c| matches!(c, 'а' | 'е' | 'ё' | 'и' | 'о' | 'у' | 'ы' | 'э' | 'ю' | 'я'));
    if !has_vowel && w_prev_lower.chars().count() <= 4 {
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

            // 0. Специальная склейка разорванного вводного оборота «К стадии...» + «Желению мирной переговоры...» -> «К сожалению, мирные переговоры...»
            let prev_low = prev.text.trim().to_lowercase();
            let curr_low = curr_text.to_lowercase();
            if gap <= 0.30
                && (prev_low == "к стадии..." || prev_low == "кстати..." || prev_low == "к стадии")
                && (curr_low.starts_with("желению ") || curr_low.starts_with("к сожалению"))
            {
                let rest = if curr_low.starts_with("желению ") {
                    &curr_text["желению ".len()..]
                } else {
                    curr_text.as_str()
                };
                prev.text = if curr_low.starts_with("желению ") {
                    fix_common_whisper_glued_words(&format!("К сожалению, {}", rest.trim_start()))
                } else {
                    fix_common_whisper_glued_words(&curr_text)
                };
                prev.end = prev.end.max(frag.end);
                repaired_count += 1;
                continue;
            }

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
            // которые whisper.cpp перенес в конец сегмента при срабатывании лимита длины строки `-ml`.
            // ВАЖНО: Если короткий кусок НЕ имеет финальной точки/!/? и начинается с подчинительного союза («что для меня», «если я», «когда он»),
            // то это НЕ хвост предыдущей фразы, а оторванное начало следующей клаузы — его нельзя оставлять висящим на конце `prev`!
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
                let curr_has_punct = crate::text_analyzer::has_terminal_punctuation(&curr_text);
                let curr_lower = curr_text.to_lowercase().replace('ё', "е");
                let starts_subordinate = [
                    "что ", "чтобы ", "как ", "когда ", "если ", "потому ", "так как ",
                    "хотя ", "пусть ", "словно ", "будто ", "где ", "куда ", "кто ", "чей ",
                ]
                .iter()
                .any(|p| curr_lower.starts_with(p));

                // Если `prev` сам является коротким оторванным зачином клаузы (1–3 слова без точки, например «что для меня» или «то не дядя,»),
                // а `curr_text` — его прямое строчное продолжение («всё не по чём...»), склеиваем их в единую фразу!
                let prev_words = prev.text.split_whitespace().count();
                if starts_lower
                    && !starts_dialog
                    && prev_words <= 3
                    && (prev.text.chars().count() + 1 + curr_chars) <= 105
                {
                    prev.text.push(' ');
                    prev.text.push_str(&curr_text);
                    prev.end = prev.end.max(frag.end);
                    repaired_count += 1;
                    continue;
                }

                let is_short_orphan_tail = !starts_dialog
                    && (curr_has_punct || (!starts_subordinate && curr_words == 1))
                    && ((starts_lower && curr_words <= 3 && curr_chars <= 24 && curr_dur <= 1.6)
                        || (curr_words == 1 && curr_chars <= 14 && curr_dur <= 1.0 && gap <= 0.08));

                if is_short_orphan_tail && (prev.text.chars().count() + 1 + curr_chars) <= 105 {
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

/// Двунаправленная идемпотентная подтяжка аномально растянутых таймкодов Whisper:
/// - В `whisper.cpp` растяжка таймкода (`actual_dur > max_valid_dur`) возникает в двух случаях:
///   1) **Тишина ПЕРЕД фразой** (смена сцены): `orig_start` прилип к концу предыдущей сцены, а сама фраза произносится в конце интервала,
///      вплотную стыкуясь со следующей репликой (`raw_bounds[i + 1].0 - orig_end <= 0.15с`, например первая половина фразы переходит во вторую).
///      В этом случае конец фразы `orig_end` фиксирован точно, а `start` подтягивается вперед к `(orig_end - spoken_dur)`!
///   2) **Тишина/музыка ПОСЛЕ фразы**: после `orig_end` идет пауза или удаленный музыкальный тег (`gap_to_next > 0.15с`).
///      В этом случае начало фразы `orig_start` фиксировано, а `end` подтягивается назад к `(orig_start + spoken_dur)`!
/// - Повторный вызов на уже очищенном SRT гарантированно дает `tightened_count == 0` (100% идемпотентность).
pub fn tighten_subtitle_timestamps(
    mut fragments: Vec<crate::text_analyzer::SubtitleFragment>,
) -> (Vec<crate::text_analyzer::SubtitleFragment>, usize) {
    if fragments.is_empty() {
        return (fragments, 0);
    }

    let len = fragments.len();
    let raw_bounds: Vec<(f64, f64)> = fragments.iter().map(|f| (f.start, f.end)).collect();
    let mut tightened_count = 0usize;

    for i in 0..len {
        let (orig_start, orig_end) = raw_bounds[i];
        let actual_dur = (orig_end - orig_start).max(0.0);
        if actual_dur <= 0.80 {
            continue;
        }

        let char_count = fragments[i]
            .text
            .chars()
            .filter(|c| !c.is_whitespace())
            .count()
            .max(1);
        let word_count = fragments[i].text.split_whitespace().count().max(1);

        // Естественная длительность произнесения фразы в живой актерской речи (русский дубляж)
        // ~11-13 знаков в секунду, ~2.5-3.2 слова в секунду + 200-300мс на паузы
        let spoken_dur = (0.28 + (char_count as f64) / 11.5)
            .max((word_count as f64) * 0.40)
            .clamp(0.65, 5.50);

        // Допустимый максимум удержания титра на экране: не дольше естественной речи + 400мс
        let max_valid_dur = (spoken_dur * 1.25 + 0.35).clamp(0.90, 6.20);

        if actual_dur <= max_valid_dur {
            continue;
        }

        tightened_count += 1;

        let prev_new_end = if i > 0 { fragments[i - 1].end } else { 0.0 };
        let touches_next_cue = if i + 1 < len {
            (raw_bounds[i + 1].0 - orig_end).abs() <= 0.15
        } else {
            false
        };
        let continues_into_next = if i + 1 < len {
            !crate::text_analyzer::has_terminal_punctuation(&fragments[i].text)
                || fragments[i + 1]
                    .text
                    .trim()
                    .chars()
                    .next()
                    .map_or(false, |c| c.is_lowercase())
        } else {
            false
        };
        let prev_sentence_closed = if i > 0 {
            crate::text_analyzer::has_terminal_punctuation(&fragments[i - 1].text)
        } else {
            true
        };

        // Если конец реплики вплотную упирается в начало следующей реплики (особенно когда предложение продолжается в `i + 1`
        // или предыдущее предложение `i - 1` было завершено), значит тишина была ПЕРЕД этой репликой — сдвигаем `start` вперед к `orig_end - spoken_dur`!
        if touches_next_cue && (continues_into_next || prev_sentence_closed) {
            let new_end = orig_end;
            let new_start = (new_end - spoken_dur)
                .max(orig_start)
                .max(prev_new_end + 0.05)
                .min(new_end - 0.60);
            fragments[i].start = new_start;
            fragments[i].end = new_end;
        } else {
            let new_start = orig_start.max(prev_new_end + 0.05);
            let target_end = new_start + spoken_dur;
            let new_end = target_end.min(orig_end).max(new_start + 0.60);
            fragments[i].start = new_start;
            fragments[i].end = new_end;
        }
    }

    (fragments, tightened_count)
}

/// Очищает SRT-файл от мусорных галлюцинаций Whisper, зацикленных повторов, разорванных полуслов/хвостов фраз и растянутых таймкодов
pub async fn sanitize_srt_file(srt_path: &Path, app_handle: &AppHandle) -> AppResult<usize> {
    let content = tokio::fs::read_to_string(srt_path).await.map_err(AppError::Io)?;
    let raw_fragments = crate::text_analyzer::parse_srt(&content);
    let raw_count = raw_fragments.len();

    // Определяем, является ли дорожка русскоязычной (>= 70% реплик содержат кириллицу),
    // чтобы отсекать англоязычные галлюцинации Whisper на вокале опенинга/эндинга (например «Let's go to the play again»)
    let cyrillic_lines = raw_fragments
        .iter()
        .filter(|f| f.text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)))
        .count();
    let is_predominantly_russian = raw_count >= 10 && cyrillic_lines * 10 >= raw_count * 7;

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

        // Если видео русскоязычное, а строка состоит только из латиницы без единой русской буквы — это песня OP/ED или английский шум Whisper
        if is_predominantly_russian {
            let has_cyr = cleaned_text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
            let has_lat = cleaned_text.chars().any(|c| c.is_ascii_alphabetic());
            if !has_cyr && has_lat {
                continue;
            }
        }

        frag.text = cleaned_text;
        stage1.push(frag);
    }

    let norm_line = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphabetic() || c.is_ascii_digit() || c.is_whitespace())
            .collect::<String>()
            .to_lowercase()
            .replace('ё', "е")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };

    // Фильтруем зацикленные повторы Whisper (например 9x «ПЕСНЯ ВСХЛИПЫМ», 22x «Let's go to the play again» или «Он становится сильнее, прямо на глазах.»)
    let mut clean_fragments: Vec<crate::text_analyzer::SubtitleFragment> = Vec::with_capacity(stage1.len());
    let mut i = 0;
    while i < stage1.len() {
        let norm_i = norm_line(&stage1[i].text);
        let prefix_i: String = norm_i.chars().take(18).collect();
        let short_prefix_i: String = norm_i.chars().take(10).collect();
        let words_i = norm_i.split_whitespace().count();

        // Считаем, сколько раз подряд повторяется эта же фраза без учета знаков препинания (или 18-символьный префикс для длинных галлюцинаций песни)
        let mut run_end = i + 1;
        while run_end < stage1.len() {
            let norm_k = norm_line(&stage1[run_end].text);
            let same_exact = !norm_i.is_empty() && norm_k == norm_i;
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
        if run_len >= 4 {
            // Если прямо перед 4+-кратным зацикливанием вокала стоит одиночная строка с тем же 10-символьным префиксом
            // (например «Повторяй на Сент-Чьи» перед 8x «Повторяй на Вище»), удаляем и её!
            if !short_prefix_i.is_empty() && short_prefix_i.chars().count() >= 9 {
                if let Some(last_pushed) = clean_fragments.last() {
                    let last_norm = norm_line(&last_pushed.text);
                    if last_norm.starts_with(&short_prefix_i)
                        && (stage1[i].start - last_pushed.end).abs() <= 2.5
                    {
                        clean_fragments.pop();
                    }
                }
            }
            // Любая фраза (даже из 1–3 слов, как «ПЕСНЯ ВСХЛИПЫМ» x9), повторенная 4+ раз подряд —
            // это 100% зацикленная галлюцинация Whisper на музыке или шуме, отбрасываем весь кластер!
            i = run_end;
            continue;
        } else if run_len >= 2 {
            // Для обычного двойного/тройного эха оставляем только 1 экземпляр без дублирования
            clean_fragments.push(stage1[i].clone());
            i = run_end;
            continue;
        } else {
            clean_fragments.push(stage1[i].clone());
            i += 1;
        }
    }

    let mut clean_fragments = deduplicate_multiline_loops(clean_fragments);
    let mut total_stitched_words = 0usize;
    let mut total_tightened_cues = 0usize;

    // Многопроходная конвергенция (до достижения полной неподвижной точки):
    // сначала склеиваем разорванные полуслова/хвосты и убираем многострочное эхо, затем подтягиваем таймкоды,
    // чтобы повторный вызов `sanitize_srt_file` на кэшированном SRT всегда давал 0 изменений!
    for _pass in 0..4 {
        let prev_len = clean_fragments.len();
        let (tightened_frags, t_cnt1) = tighten_subtitle_timestamps(clean_fragments);
        let (repaired_frags, s_cnt) = repair_split_words_and_orphans(tightened_frags);
        let dedup_frags = deduplicate_multiline_loops(repaired_frags);
        let (final_frags, t_cnt2) = tighten_subtitle_timestamps(dedup_frags);
        let t_cnt = t_cnt1 + t_cnt2;
        total_tightened_cues += t_cnt;
        total_stitched_words += s_cnt;
        let len_changed = final_frags.len() != prev_len;
        clean_fragments = final_frags;
        if t_cnt == 0 && s_cnt == 0 && !len_changed {
            break;
        }
    }

    // Финальный проход словаря склеенных слов после всех объединений строк
    for frag in &mut clean_fragments {
        frag.text = fix_common_whisper_glued_words(&frag.text);
    }

    let removed = raw_count
        .saturating_sub(clean_fragments.len())
        .saturating_sub(total_stitched_words);

    if removed > 0 || total_stitched_words > 0 || total_tightened_cues > 0 {
        crate::logger::emit_log(
            app_handle,
            "Whisper",
            "info",
            &format!(
                "Очистка и синхронизация субтитров: удалено {} мусорных/дублирующихся строк, склеено {} полуслов/фраз, подтянуто {} растянутых таймкодов, итого {} точных реплик.",
                removed,
                total_stitched_words,
                total_tightened_cues,
                clean_fragments.len()
            ),
        );

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
    }

    Ok(clean_fragments.len())
}

/// Убирает многострочные зацикленные повторы Whisper:
/// 1) Блочные повторы ABAB / ABCABC из 2..=6 строк подряд (например, когда 5 реплик диалога повторяются дважды подряд).
/// 2) Слитные и дробные эхо-повторы, когда Whisper объединяет предыдущие 2–3 короткие реплики в одну строку-дубликат
///    или наоборот дробит только что сказанную длинную фразу на повторяющиеся половинки.
/// 3) Одиночные эхо-повторы через 1–4 короткие реплики (шаблоны A-B-A, A-B-C-A и A-B-C-D-A в пределах 18 секунд).
/// 4) Нахлест начала строки на хвост предыдущих 1–2 реплик («маленькую хитрость. Виноват. А что за хитрость?» -> «А что за хитрость?»).
fn deduplicate_multiline_loops(
    mut fragments: Vec<crate::text_analyzer::SubtitleFragment>,
) -> Vec<crate::text_analyzer::SubtitleFragment> {
    if fragments.len() < 2 {
        return fragments;
    }

    fn norm_alpha(s: &str) -> String {
        s.chars()
            .filter(|c| c.is_alphabetic() || c.is_whitespace())
            .collect::<String>()
            .to_lowercase()
            .replace('ё', "е")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn words_match_fuzzy(a: &str, b: &str) -> bool {
        if a == b {
            return true;
        }
        let ac: Vec<char> = a.chars().collect();
        let bc: Vec<char> = b.chars().collect();
        if ac.len() >= 4 && bc.len() >= 4 {
            // Совпадение первых 4 букв ИЛИ последних 4 букв (ловит опечатки Whisper в первой букве: «Афицеры» / «Офицеры»)
            if ac[..4] == bc[..4] || ac[ac.len() - 4..] == bc[bc.len() - 4..] {
                return true;
            }
        }
        false
    }

    /// Проверяет, есть ли у двух реплик общий непрерывный 4-словный фрагмент
    /// (например «Надеюсь, не откажешься выпить по кружечке чая после боя» и «Вы можете выпить по кружечке чая после боя»)
    fn has_shared_4gram(a_words: &[&str], b_words: &[&str]) -> bool {
        if a_words.len() < 4 || b_words.len() < 4 {
            return false;
        }
        for wa in a_words.windows(4) {
            for wb in b_words.windows(4) {
                if (0..4).all(|idx| words_match_fuzzy(wa[idx], wb[idx])) {
                    return true;
                }
            }
        }
        false
    }

    let mut norms: Vec<String> = fragments.iter().map(|f| norm_alpha(&f.text)).collect();
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

        // 2. Срезаем эхо-нахлест в начале текущей реплики, если она начинается с повтора хвоста предыдущих 1–2 реплик
        // (например «маленькую хитрость. Виноват. А что за хитрость?» -> «А что за хитрость?»,
        //  «командиров. Какие доказательства вы имеете?» -> «Какие доказательства вы имеете?»)
        if !out_norms.is_empty() && (fragments[i].start - out.last().unwrap().end) <= 6.0 {
            let lookback_n = out_norms.len().min(2);
            let recent_prev = out_norms[out_norms.len() - lookback_n..].join(" ");
            let recent_prev_words: Vec<&str> = recent_prev.split_whitespace().collect();

            if !recent_prev_words.is_empty() {
                let raw_txt = fragments[i].text.trim().to_string();
                let mut best_remainder: Option<String> = None;

                for (byte_idx, ch) in raw_txt.char_indices() {
                    if matches!(ch, '.' | '!' | '?') {
                        let split_pos = byte_idx + ch.len_utf8();
                        let prefix_part = raw_txt[..split_pos].trim();
                        let remainder_part = raw_txt[split_pos..].trim();
                        if remainder_part.is_empty() {
                            continue;
                        }
                        let prefix_norm = norm_alpha(prefix_part);
                        let prefix_words: Vec<&str> = prefix_norm.split_whitespace().collect();
                        let rem_words = norm_alpha(remainder_part).split_whitespace().count();

                        if !prefix_words.is_empty()
                            && prefix_words.len() <= 6
                            && rem_words >= 2
                            && prefix_words
                                .iter()
                                .all(|&pw| recent_prev_words.iter().any(|&rw| words_match_fuzzy(pw, rw)))
                        {
                            best_remainder = Some(remainder_part.to_string());
                        }
                    }
                }

                if let Some(clean_rem) = best_remainder {
                    fragments[i].text = clean_rem;
                    norms[i] = norm_alpha(&fragments[i].text);
                }
            }
        }

        // 3. Проверяем слитный, дробный или чередующийся (A-B-A / A-B-C-A / A-B-C-D-A) эхо-повтор относительно последних 1..=5 принятых реплик
        let curr_words: Vec<&str> = norms[i].split_whitespace().collect();
        if curr_words.len() >= 2 && !out_norms.is_empty() {
            let mut is_echo = false;

            // А) Повтор строки (или её дробного осколка) относительно любой из последних 1..=5 реплик в окне 18 секунд
            let max_lookback = out_norms.len().min(5);
            for d in 1..=max_lookback {
                let prev_idx = out_norms.len() - d;
                let time_gap = (fragments[i].start - out[prev_idx].end).max(0.0);
                if time_gap > 18.0 {
                    continue;
                }
                let prev_words: Vec<&str> = out_norms[prev_idx].split_whitespace().collect();
                if prev_words.is_empty() {
                    continue;
                }

                // Точный или нечеткий повтор целой реплики через 0–4 промежуточные вставки (включая общий 4-словный блок)
                if curr_words.len() >= 3 && prev_words.len() >= 3 {
                    let matched = curr_words
                        .iter()
                        .filter(|&&cw| prev_words.iter().any(|&pw| words_match_fuzzy(cw, pw)))
                        .count();
                    let min_len = curr_words.len().min(prev_words.len());
                    let shared_phrase = has_shared_4gram(&curr_words, &prev_words);
                    if norms[i] == out_norms[prev_idx]
                        || (min_len >= 4 && matched * 5 >= min_len * 4)
                        || shared_phrase
                    {
                        // Если новая версия реплики более полная (например «А ведь у нас идёт война, господин генерал» вместо обрывка «А ведь у нас сомнений»),
                        // обновляем текст первой реплики
                        if curr_words.len() >= prev_words.len() + 2 {
                            out[prev_idx].text = fragments[i].text.clone();
                            out_norms[prev_idx] = norms[i].clone();
                        }
                        is_echo = true;
                        break;
                    }
                }

                // Дробный эхо-повтор: текущая короткая/средняя строка входит в одну из последних 3 более длинных реплик
                if d <= 3 && prev_words.len() >= curr_words.len() && prev_words.len() >= 4 {
                    let matched = curr_words
                        .iter()
                        .filter(|&&cw| prev_words.iter().any(|&pw| words_match_fuzzy(cw, pw)))
                        .count();
                    if matched == curr_words.len()
                        || (curr_words.len() >= 4 && matched * 5 >= curr_words.len() * 4)
                    {
                        is_echo = true;
                        break;
                    }
                }
            }

            // Б) Слитный эхо-повтор: текущая длинная строка (>= 6 слов) повторяет склеенные последние 2 или 3 реплики
            if !is_echo && curr_words.len() >= 6 && out_norms.len() >= 2 {
                for m in 2..=3 {
                    if out_norms.len() >= m {
                        let combined_prev = out_norms[out_norms.len() - m..].join(" ");
                        let prev_words: Vec<&str> = combined_prev.split_whitespace().collect();
                        if prev_words.len() >= 5 {
                            let matched = curr_words
                                .iter()
                                .filter(|&&cw| prev_words.iter().any(|&pw| words_match_fuzzy(cw, pw)))
                                .count();
                            if matched * 10 >= curr_words.len() * 7 {
                                is_echo = true;
                                break;
                            }
                        }
                    }
                }
            }

            if is_echo {
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
        if let Ok(raw_before_sanitize) = tokio::fs::read_to_string(&project_srt).await {
            let (raw_loops, raw_wrong_lang) = check_cached_srt_validity(&raw_before_sanitize);
            if raw_wrong_lang {
                crate::logger::emit_log(
                    &app_handle,
                    "Whisper",
                    "warn",
                    "В кэшированном файле субтитров обнаружена ошибочная англоязычная транскрибация (из-за автоопределения языка на песне опенинга). Выполняется чистая повторная транскрибация на русском языке (-l ru)...",
                );
                let _ = tokio::fs::remove_file(&project_srt).await;
            } else if raw_loops {
                crate::logger::emit_log(
                    &app_handle,
                    "Whisper",
                    "warn",
                    "В старом файле субтитров обнаружены зацикленные галлюцинации. Выполняется чистая повторная транскрибация (-l ru -mc 0)...",
                );
                let _ = tokio::fs::remove_file(&project_srt).await;
            }
        }

        if project_srt.exists() {
            // Всегда прогоняем быструю санитизацию кэшированного SRT (удаляет [музыка], титры редакторов и растяжки букв)
            let _ = sanitize_srt_file(&project_srt, &app_handle).await;
            if let Ok(content) = tokio::fs::read_to_string(&project_srt).await {
                let (has_loops, wrong_lang) = check_cached_srt_validity(&content);
                if !content.trim().is_empty() && !has_loops && !wrong_lang {
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
                } else {
                    crate::logger::emit_log(
                        &app_handle,
                        "Whisper",
                        "warn",
                        "Кэшированный файл субтитров не прошел проверку качества языка/повторов. Выполняется повторная транскрибация (-l ru)...",
                    );
                    let _ = tokio::fs::remove_file(&project_srt).await;
                }
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
        &format!("Запуск whisper.cpp (модель: {}, язык: ru, anti-loop: -mc 0, вывод: {}.srt)...", model.display(), base_out),
    );

    // Передаем:
    // -l ru: жесткая фиксация русского языка распознавания (иначе на аниме/сериалах с англоязычной/японской песней в первые 30с
    //        флаг `-l auto` ошибочно ставит `lang = en` и переводит весь 24-минутный русский дубляж на ломаный английский!)
    // -mc 0: отключение переноса текстового контекста между 30с окнами (предотвращает 7-минутные циклы «Корректор А. Кулак» после музыки!)
    // -et 2.2: порог энтропии для подавления повторов
    // -wt 0.05: порог вероятности таймкода слова (устраняет растягивание начала субтитра на предшествующую тишину/музыку)
    // -ml 90 -sow: в whisper.cpp -ml измеряется в байтах UTF-8 (90 байт = ~45 русских букв) + разбиение строго по целым словам (-sow)
    // -pp: вывод процентов прогресса
    let command = command
        .env("PATH", crate::logger::build_sidecar_env_path(&app_handle))
        .args([
            "-m", &model.to_string_lossy(),
            "-l", "ru",
            "-mc", "0",
            "-et", "2.2",
            "-wt", "0.05",
            "-ml", "90",
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
