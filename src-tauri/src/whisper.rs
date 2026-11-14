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

///// Известные галлюцинации Whisper на музыке, титрах, заставках и тишине
pub fn is_whisper_hallucination(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }

    let lower = trimmed.to_lowercase().replace('ё', "е");

    // 0. Детектор повторов одной буквы и галлюцинаций с низкой алфавитной вариативностью (например «ппппп...», «ааааа...», «ммммм...»)
    let alpha_chars: Vec<char> = lower.chars().filter(|c| c.is_alphabetic()).collect();
    if alpha_chars.len() >= 8 {
        let mut unique_set = std::collections::HashSet::new();
        for &c in &alpha_chars {
            unique_set.insert(c);
        }
        if unique_set.len() <= 2 {
            return true;
        }
    }
    // Проверка на 7+ одинаковых букв подряд внутри одного слова
    for word in lower.split_whitespace() {
        let mut max_run = 0usize;
        let mut prev_c = None;
        let mut current_run = 0usize;
        for c in word.chars().filter(|c| c.is_alphabetic()) {
            if Some(c) == prev_c {
                current_run += 1;
            } else {
                prev_c = Some(c);
                current_run = 1;
            }
            if current_run > max_run {
                max_run = current_run;
            }
        }
        if max_run >= 7 {
            return true;
        }
    }

    // 1. Технические теги звуков, действий, музыки и субтитров вида [музыка], (музыка), *музыка*, (Звук отрыва), *покатывается*, ♪ и т.д.
    let stripped_symbols = lower.trim_matches(['.', '!', '?', ',', '-', '—', '"', '\'', '«', '»', ' ']);
    if (stripped_symbols.starts_with('[') && stripped_symbols.ends_with(']'))
        || (stripped_symbols.starts_with('(') && stripped_symbols.ends_with(')'))
        || (stripped_symbols.starts_with('*') && stripped_symbols.ends_with('*'))
        || lower.contains('♪')
        || lower.contains('♫')
        || lower.contains('🎵')
    {
        return true;
    }

    // Акустические ремарки и действия в скобках / со звездочками в любой части строки
    if (lower.contains('(') || lower.contains('[') || lower.contains('*'))
        && (lower.contains(')') || lower.contains(']') || lower.contains('*'))
    {
        if lower.contains("звук")
            || lower.contains("музык")
            || lower.contains("шум")
            || lower.contains("покатывает")
            || lower.contains("вздох")
            || lower.contains("смех")
            || lower.contains("крик")
            || lower.contains("оттенк")
            || lower.contains("битв")
            || lower.contains("гонко")
            || lower.contains("гонк")
            || lower.contains("весел")
            || lower.contains("шип")
            || lower.contains("рыч")
        {
            return true;
        }
    }

    // Одиночные звуковые ремарки со знаками препинания или без
    let clean_exact = lower.trim_matches(['.', '!', '?', ',', '-', '—', '*', '"', '\'', '«', '»', '(', ')', '[', ']', ' ']);

    let exact_tags = [
        "музыка",
        "звучит музыка",
        "музыка звучит",
        "играет музыка",
        "фоновая музыка",
        "спокойная музыка",
        "динамичная музыка",
        "тревожная музыка",
        "веселая музыка",
        "печальная музыка",
        "грустная музыка",
        "громкая музыка",
        "тихая музыка",
        "эпичная музыка",
        "музыкальная заставка",
        "музыкальная пауза",
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
        "крик",
        "крики",
        "поет",
        "поёт",
        "звук",
        "звуки",
        "шум",
        "шумы",
        "тишина",
        "music",
        "applause",
        "выстрелы",
        "выстрел",
        "перестрелка",
        "сигналы",
        "сигнал",
        "сирена",
        "сирены",
        "стук в дверь",
        "стук",
        "шаги",
        "взрыв",
        "взрывы",
        "стон",
        "стоны",
        "всхлип",
        "всхлипы",
        "всхлипывает",
        "песня",
        "песня дверь",
        "дима клан",
        "потворать жисерце",
        "неразборчиво",
        "не разборчиво",
        "показывает",
        "слышен",
        "слышно",
        "шепот",
        "вздохи",
        "тихие звуки",
    ];
    if exact_tags.contains(&clean_exact) {
        return true;
    }

    // Звуковые префиксы (до 4 слов): «звук сирены», «звучит мелодия», «слышен шум»
    let word_count = clean_exact.split_whitespace().count();
    if word_count <= 4 {
        if clean_exact.starts_with("звук ")
            || clean_exact.starts_with("звуки ")
            || clean_exact.starts_with("звучит ")
            || clean_exact.starts_with("звучат ")
            || clean_exact.starts_with("играет ")
            || clean_exact.starts_with("слышен ")
            || clean_exact.starts_with("слышны ")
            || clean_exact.starts_with("шум ")
            || clean_exact.contains("музык")
            || clean_exact.contains("мелоди")
            || clean_exact.starts_with("песня ")
        {
            return true;
        }
    }

    // 2. Универсальные титры переводчиков, редакторов и вокальный бред на заставках
    let subtitle_credits = [
        "редактор субтитров",
        "корректор субтитров",
        "корректор ",
        "субтитры субтитров",
        "субтитры сделал",
        "субтитры создавал",
        "субтитры подготовил",
        "субтитры подогнал",
        "субтитры добав",
        "субтитры:",
        "перевод подготовил",
        "роли дублировали",
        "озвучка:",
        "валерий семьер",
        "дима клан",
        "потворать жисерце",
        "песня дверь",
        "тап-дрипт",
        "тап дрипт",
        "в китченке",
        "веки и фуар",
        "сыграя луза",
        "звуки грибов",
        "звук шерпи",
        "звук раунд",
        "звук левого раунга",
        "звук от джунга",
        "звук от джанра",
        "звук оттуда",
        "звук локации",
        "звук пива",
        "звучат пиво",
        "звучат легко",
        "по-симки",
        "звук шлифов",
        "играет тревожная музыка",
        "amara.org",
        "амара.орг",
        "opensubtitles",
    ];
    for cred in &subtitle_credits {
        if lower.contains(cred) {
            return true;
        }
    }

    // 3. Универсальные призывы (Call-to-Action) для видео, стримов и YouTube
    let cta_patterns = [
        "подпишись на канал",
        "подписывайтесь на канал",
        "ставьте лайк",
        "ставьте лайки",
        "жмите колокольчик",
        "ссылка в описании",
        "ссылка под видео",
        "продолжение следует",
        "to be continued",
        "приятного просмотра",
        "не забудьте подписаться",
        "делитесь этим видео",
        "оставляйте комментарии",
    ];
    for cta in &cta_patterns {
        if lower.contains(cta) {
            return true;
        }
    }

    // 4. Шаблонные титры песен вида «Исполнитель - "Название"» на заставках
    if (trimmed.contains(" - \"") || trimmed.contains(" — \"")) && trimmed.ends_with('"') {
        return true;
    }

    // 5. Капс-ремарки звукового описания из 2–4 слов (например «ПОЗИТИВНЫЙ ГОЛОС», «ГРОМКИЙ ШУМ», «ТИХАЯ МУЗЫКА»)
    let has_upper_cyr = trimmed.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c) && c.is_uppercase());
    let has_lower_cyr = trimmed.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c) && c.is_lowercase());
    if has_upper_cyr && !has_lower_cyr && word_count >= 2 && word_count <= 4 {
        if clean_exact.contains("голос")
            || clean_exact.contains("звук")
            || clean_exact.contains("шум")
            || clean_exact.contains("песня")
            || clean_exact.contains("музыка")
            || clean_exact.contains("мелодия")
        {
            return true;
        }
    }

    // 6. Детектор вокальных зацикливаний из коротких гласных и междометий на музыке/тишине
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
                    "а" | "о" | "у" | "э" | "и" | "ы" | "я" | "е" | "м" | "н" | "аа" | "оо" | "ах" | "ох" | "эх" | "ла" | "на"
                )
            })
            .count();
        if short_vowel_tokens >= 5
            || (alpha_tokens.len() >= 2 && short_vowel_tokens == alpha_tokens.len())
            || (alpha_tokens.len() >= 3 && short_vowel_tokens + 1 >= alpha_tokens.len() && alpha_tokens.last().map_or(false, |w| w == "я"))
        {
            return true;
        }
    }

    // 7. Слоговая псевдоречевая нарезка с аномальным числом дефисов/запятых
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

    let has_multi_hyphen_song_syllables = lower.split_whitespace().any(|w| {
        let tw = w.trim_matches([',', '.', '!', '?', '-', '—', '"', '«', '»']);
        let h_cnt = tw.matches('-').count();
        if h_cnt >= 3 {
            !tw.starts_with("ха-ха")
                && !tw.starts_with("хе-хе")
                && !tw.starts_with("хи-хи")
                && !tw.starts_with("хо-хо")
                && !tw.starts_with("ай-ай")
                && !tw.starts_with("ой-ой")
        } else if h_cnt == 2 {
            !matches!(
                tw,
                "точь-в-точь"
                    | "бок-о-бок"
                    | "один-на-один"
                    | "крест-на-крест"
                    | "с-глазу-на-глаз"
                    | "изо-дня-в-день"
                    | "ха-ха-ха"
                    | "хе-хе-хе"
                    | "хи-хи-хи"
                    | "хо-хо-хо"
                    | "ай-ай-ай"
                    | "ой-ой-ой"
                    | "э-э-э"
                    | "а-а-а"
                    | "о-о-о"
                    | "у-у-у"
            )
        } else {
            false
        }
    });
    if has_multi_hyphen_song_syllables {
        return true;
    }

    if lower.matches("- ").count() >= 3 && lower.chars().count() <= 55 {
        return true;
    }

    false
}

/// Алгоритмическая нормализация русских частиц, приставок и союзов:
/// - Дефисное написание частиц: кто-то, что-то, где-то, как-то, кто-либо, что-нибудь, кое-кто, всё-таки и др.
/// - Дефисное написание наречий: по-моему, по-твоему, по-прежнему, по-новому и др.
/// - Дефисные предлоги: из-за, из-под.
/// - Раздельное написание устойчивых сочетаний: вряд ли, как будто, в общем, то есть.
pub fn normalize_russian_particles(text: &str) -> String {
    // 1. Быстрые замены для слитных/ошибочных написаний устойчивых союзов и частиц
    let fixed_compounds: &[(&str, &str)] = &[
        ("врядли", "вряд ли"),
        ("вряд-ли", "вряд ли"),
        ("наврядли", "навряд ли"),
        ("навряд-ли", "навряд ли"),
        ("какбудто", "как будто"),
        ("как-будто", "как будто"),
        ("вобщем", "в общем"),
        ("в-общем", "в общем"),
        ("тоесть", "то есть"),
        ("то-есть", "то есть"),
        ("из за", "из-за"),
        ("из под", "из-под"),
        ("по видимому", "по-видимому"),
        ("по прежнему", "по-прежнему"),
        ("по новому", "по-новому"),
        ("по старому", "по-старому"),
        ("по моему", "по-моему"),
        ("по твоему", "по-твоему"),
        ("по нашему", "по-нашему"),
        ("по вашему", "по-вашему"),
        ("точь в точь", "точь-в-точь"),
        ("мало помалу", "мало-помалу"),
        ("бок о бок", "бок о бок"),
        ("с глазу на глаз", "с глазу на глаз"),
    ];

    let mut result = text.to_string();
    for &(wrong, correct) in fixed_compounds {
        let mut loop_count = 0;
        while loop_count < 6 {
            let lower = result.to_lowercase().replace('ё', "е");
            let wrong_norm = wrong.replace('ё', "е");
            if let Some(pos) = lower.find(&wrong_norm) {
                if pos <= result.len() && pos + wrong.len() <= result.len() {
                    let is_cap = result[pos..].chars().next().map_or(false, |c| c.is_uppercase());
                    let rep = if is_cap {
                        let mut chs = correct.chars();
                        match chs.next() {
                            Some(f) => f.to_uppercase().collect::<String>() + chs.as_str(),
                            None => correct.to_string(),
                        }
                    } else {
                        correct.to_string()
                    };
                    result = result[..pos].to_string() + &rep + &result[pos + wrong.len()..];
                    loop_count += 1;
                    continue;
                }
            }
            break;
        }
    }

    // 2. Алгоритмическое присоединение постфиксных частиц -то, -либо, -нибудь через дефис
    let post_particle_stems = [
        "кто", "кого", "кому", "кем", "ком",
        "что", "чего", "чему", "чем", "чом",
        "где", "куда", "когда", "откуда", "докуда",
        "как", "почему", "зачем", "отчего",
        "какой", "какая", "какое", "какие", "какого", "какому", "каким", "каких", "какую", "каком", "какими",
        "чей", "чья", "чье", "чьи", "чьего", "чьей", "чьему", "чьим", "чьих", "чью", "чьем",
        "сколько", "скольких", "скольким", "сколькими",
        "так", "тут", "там",
        "он", "она", "оно", "они", "мы", "вы", "я",
    ];

    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut is_alphanumeric = false;

    for ch in result.chars() {
        let is_alpha = ch.is_alphabetic() || ch == 'ё' || ch == 'Ё';
        if tokens.is_empty() && current.is_empty() {
            is_alphanumeric = is_alpha;
            current.push(ch);
        } else if is_alpha == is_alphanumeric {
            current.push(ch);
        } else {
            tokens.push(current);
            current = String::new();
            is_alphanumeric = is_alpha;
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    let mut i = 0;
    let mut normalized_tokens: Vec<String> = Vec::with_capacity(tokens.len());
    while i < tokens.len() {
        if i + 2 < tokens.len() && tokens[i + 1] == " " {
            let prev_norm = tokens[i].to_lowercase().replace('ё', "е");
            let next_norm = tokens[i + 2].to_lowercase().replace('ё', "е");

            if post_particle_stems.contains(&prev_norm.as_str())
                && matches!(next_norm.as_str(), "то" | "либо" | "нибудь")
            {
                normalized_tokens.push(tokens[i].clone());
                normalized_tokens.push("-".to_string());
                normalized_tokens.push(tokens[i + 2].to_lowercase());
                i += 3;
                continue;
            }

            if prev_norm == "кое"
                && (post_particle_stems.contains(&next_norm.as_str()) || next_norm == "как")
            {
                normalized_tokens.push(tokens[i].clone());
                normalized_tokens.push("-".to_string());
                normalized_tokens.push(tokens[i + 2].to_lowercase());
                i += 3;
                continue;
            }

            if matches!(prev_norm.as_str(), "все" | "так" | "опять") && next_norm == "таки" {
                let stem = if prev_norm == "все" {
                    if tokens[i].chars().next().map_or(false, |c| c.is_uppercase()) {
                        "Всё".to_string()
                    } else {
                        "всё".to_string()
                    }
                } else {
                    tokens[i].clone()
                };
                normalized_tokens.push(stem);
                normalized_tokens.push("-".to_string());
                normalized_tokens.push("таки".to_string());
                i += 3;
                continue;
            }
        }

        normalized_tokens.push(tokens[i].clone());
        i += 1;
    }

    normalized_tokens.concat()
}

/// Применяет список пользовательских замен к тексту с сохранением регистра первой буквы
pub fn apply_custom_replacements(text: &str, replacements: &[(String, String)]) -> String {
    if replacements.is_empty() {
        return text.to_string();
    }
    let mut result = text.to_string();
    for (from, to) in replacements {
        let from_norm = from.to_lowercase().replace('ё', "е");
        let mut loop_count = 0;
        while loop_count < 6 {
            let lower = result.to_lowercase().replace('ё', "е");
            if let Some(pos) = lower.find(&from_norm) {
                if pos <= result.len() && pos + from.len() <= result.len() {
                    let is_cap = result[pos..].chars().next().map_or(false, |c| c.is_uppercase());
                    let rep = if is_cap {
                        let mut chs = to.chars();
                        match chs.next() {
                            Some(first) => first.to_uppercase().collect::<String>() + chs.as_str(),
                            None => to.to_string(),
                        }
                    } else {
                        to.to_string()
                    };
                    result = result[..pos].to_string() + &rep + &result[pos + from.len()..];
                    loop_count += 1;
                    continue;
                }
            }
            break;
        }
    }
    result
}

/// Загружает пользовательский словарь замен (replacements.json / dictionary.txt / словарь.txt)
/// из папки субтитров, папки проекта или папки видео
pub fn load_project_replacements(srt_path: &Path) -> Vec<(String, String)> {
    let mut search_dirs = Vec::new();
    if let Some(p1) = srt_path.parent() {
        search_dirs.push(p1.to_path_buf());
        if let Some(p2) = p1.parent() {
            search_dirs.push(p2.to_path_buf());
            if let Some(p3) = p2.parent() {
                search_dirs.push(p3.to_path_buf());
            }
        }
    }

    let candidate_names = [
        "replacements.json",
        "dictionary.json",
        "словарь.json",
        "replacements.txt",
        "dictionary.txt",
        "словарь.txt",
    ];

    for dir in &search_dirs {
        for name in &candidate_names {
            let file_path = dir.join(name);
            if file_path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    let mut list = Vec::new();
                    if name.ends_with(".json") {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(obj) = val.as_object() {
                                for (k, v) in obj {
                                    if let Some(s) = v.as_str() {
                                        list.push((k.to_string(), s.to_string()));
                                    }
                                }
                            } else if let Some(arr) = val.as_array() {
                                for item in arr {
                                    if let Some(pair) = item.as_array() {
                                        if pair.len() >= 2 {
                                            if let (Some(k), Some(v)) = (pair[0].as_str(), pair[1].as_str()) {
                                                list.push((k.to_string(), v.to_string()));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        for line in content.lines() {
                            let line = line.trim();
                            if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                                continue;
                            }
                            let parts: Option<(&str, &str)> = if let Some((k, v)) = line.split_once("->") {
                                Some((k.trim(), v.trim()))
                            } else if let Some((k, v)) = line.split_once('=') {
                                Some((k.trim(), v.trim()))
                            } else if let Some((k, v)) = line.split_once(':') {
                                Some((k.trim(), v.trim()))
                            } else {
                                None
                            };
                            if let Some((k, v)) = parts {
                                if !k.is_empty() && !v.is_empty() {
                                    list.push((k.to_string(), v.to_string()));
                                }
                            }
                        }
                    }
                    if !list.is_empty() {
                        return list;
                    }
                }
            }
        }
    }

    Vec::new()
}

/// Исправляет слипшиеся слова, предлоги и устойчивые фонетические искажения Whisper в разговорной русской речи
pub fn fix_common_whisper_glued_words(text: &str) -> String {
    let glued_patterns: &[(&str, &str)] = &[
        // 1. Слипшиеся служебные части речи и устойчивые обороты
        ("недостаточносил", "недостаточно сил"),
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
        ("незнают", "не знают"),
        ("небойся", "не бойся"),
        ("ненадо", "не надо"),
        ("ненужно", "не нужно"),
        ("невидно", "не видно"),
        ("неслышно", "не слышно"),
        ("нестоит", "не стоит"),
        ("небыло", "не было"),
        ("небудет", "не будет"),
        ("неодин", "не один"),
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
        ("низачто", "ни за что"),
        ("незачто", "не за что"),
        ("самособой", "само собой"),
        ("совершеноверно", "совершенно верно"),
        ("доздравствует", "да здравствует"),

        // 2. Слипшиеся предлоги с местоимениями и существительными
        ("втот", "в тот"),
        ("втом", "в том"),
        ("вэтом", "в этом"),
        ("вэтих", "в этих"),
        ("вэтой", "в этой"),
        ("втех", "в тех"),
        ("споля", "с поля"),
        ("спола", "с пола"),
        ("содного", "с одного"),
        ("содной", "с одной"),
        ("кнам", "к нам"),
        ("квам", "к вам"),
        ("отом", "о том"),
        ("обэтом", "об этом"),
        ("нанас", "на нас"),
        ("навас", "на вас"),
        ("изза", "из-за"),
        ("изпод", "из-под"),

        // 3. Типичные акустические опечатки и искажения микрофонной речи
        ("цырум", "труп"),
        ("труб!", "труп!"),
        ("обостоверение", "удостоверение"),
        ("сверхуручно", "сверхурочно"),
        ("прилютно", "прилюдно"),
        ("пазор", "позор"),
        ("пророклив", "проклят"),
        ("вдарц", "в дартс"),
        ("в дарц", "в дартс"),
        ("своял тёплёп", "сваял тяп-ляп"),
        ("своял теплеп", "сваял тяп-ляп"),
        ("замби", "зомби"),
        ("настамнин", "наставник"),
        ("настагоник", "наставник"),
        ("боженько", "боженька"),
        ("едвале", "едва ли"),
        ("стоятно признаться", "стыдно признаться"),
        ("в роль бжица", "в роль вжиться"),
        ("грязные бонючие", "грязные вонючие"),
        ("бонючие", "вонючие"),
        ("горбатова могила исправить", "горбатого могила исправит"),
        ("во все нет", "вовсе нет"),
        ("я от тебя понял", "я тебя понял"),
        ("ипланты", "импланты"),
        ("иплантов", "имплантов"),
        ("иплантам", "имплантам"),
        ("иплант", "имплант"),
        ("новый имплан,", "новый имплант,"),
        ("новый имплан.", "новый имплант."),
        ("кибропсихоз", "киберпсихоз"),
        ("гиперпсихоз", "киберпсихоз"),
        ("киперпсихоз", "киберпсихоз"),
        ("киберпсихос", "киберпсихоз"),
        ("кибропсих", "киберпсих"),
        ("киперпсих", "киберпсих"),
        ("типо киперпалка", "типа киберпанка"),
        ("киперпалка", "киберпанка"),
        ("да чум бы, по типу горевший", "да, чумба, походу перегоревший"),
        ("чум бы,", "чумба,"),
        ("о, же здесь из-за тебя", "он же здесь из-за тебя"),
        ("что ганешь", "что гонишь"),
        ("с освоем храмом сборохолки", "со своим хромом с барахолки"),
        ("апасно", "опасно"),
        ("кантау", "Кантао"),
        ("ты че сейчас веркнула", "ты чё сейчас вякнула"),
        ("не реши мне тот, мажорька", "не лечи меня тут, мажорка"),
        ("балалетняя", "малолетняя"),
        ("не броцелиться мне", "не прицелиться мне"),
        ("тупая цука", "тупая сука"),
        ("стопая, сука", "тупая сука"),
        ("психовная сука", "психованная сука"),
        ("сдохнет, психованная", "сдохни, психованная"),
        ("медкость еще при мне", "меткость ещё при мне"),
        ("отойди оттали", "отойди от Талии"),
        ("я ничего не угрожаю", "я ничем не угрожаю"),
        ("отвезем тебя, кавидя", "отвезём тебя к Авиде"),
        ("отвезем тебя кавидя", "отвезём тебя к Авиде"),
        ("за получим мы чип", "заполучи мы чип"),
        ("с лиом даны по безопасности", "со сливом данных по безопасности"),
        ("если смешно, посмитесь", "если смешно, посмейтесь"),
        ("махстаг", "Макс-Так"),
        ("еросака", "Арасака"),
        ("оросаке", "Арасаке"),
        ("птверждено", "подтверждено"),
        ("я чнитесь вы из лаборатории", "очнитесь вы, из лаборатории"),
        ("мы против по этому зданию", "мы бродим по этому зданию"),
        ("-подожди,ди!", "- Подожди, Ди!"),
        ("подожди,ди!", "подожди, Ди!"),
        ("прилебала", "прилипала"),
        ("возмращай давай", "возвращай давай"),
        ("паран, говорил же с медицей за дорогой", "баран, говорил же следить за дорогой"),
        ("говорил же с медицей за дорогой", "говорил же следить за дорогой"),
        ("не оплевать", "не плевать"),
        ("ротан!", "братан!"),
        ("стох, не ублюдок", "сдохни, ублюдок"),
        ("урасправился", "расправился"),
        ("когнешь атаково", "как насчёт такого"),
        ("тогда ваим тоже обеспечим", "тогда давай им тоже обеспечим"),
        ("с ави-дой", "с Авидой"),
        ("ты зела, мне ранена", "ты цела, не ранена"),
        ("косиктанской идеологии", "сектантской идеологии"),
        ("буду мстеть", "буду мстить"),
        ("проснуся", "проснулся"),
        ("асторожно", "осторожно"),
        ("заимно.", "взаимно."),
        ("заимно!", "взаимно!"),
        ("сарян,", "сорян,"),
        ("сарян.", "сорян."),
        ("свежи с тобой", "свяжусь с тобой"),
        ("еда это не отравлена", "еда эта не отравлена"),
    ];

    let mut result = text.to_string();
    for &(glued, fixed) in glued_patterns {
        let mut loop_count = 0;
        while loop_count < 6 {
            let lower = result.to_lowercase().replace('ё', "е");
            let glued_norm = glued.replace('ё', "е");
            if let Some(pos) = lower.find(&glued_norm) {
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
                    loop_count += 1;
                    continue;
                }
            }
            break;
        }
    }

    normalize_russian_particles(&result)
}

/// Устраняет артефакты заикания / вдоха Whisper с висячей запятой, дефисом или многоточием перед словом
/// (например «Г, госпожа Бреслий!» -> «Госпожа Бреслий!», «В… Воды?» -> «Воды?», «П-поверьте...» -> «Поверьте...», «Н-н-н-ничего не понимаю» -> «Ничего не понимаю», «Н-н-н-н-н-н, что» -> «На что»)
pub fn fix_whisper_stutters(text: &str) -> String {
    let mut words: Vec<String> = text.split_whitespace().map(|w| w.to_string()).collect();
    if words.is_empty() {
        return text.to_string();
    }

    let mut changed = false;

    // 1. Внутрисловные дефисные заикания («П-поверьте...» -> «Поверьте...», «Н-н-н-ничего» -> «Ничего», «Э-э-э-э-э-э-э!» -> «Э-э-э!», «Н-н-н-н-н-н,» -> удаляем или схлопываем)
    let mut k = 0;
    while k < words.len() {
        let w = words[k].clone();
        let lead_punct: String = w.chars().take_while(|c| matches!(c, '-' | '—' | '«' | '"' | '(')).collect();
        let body = &w[lead_punct.len()..];

        if body.contains('-') {
            let parts: Vec<&str> = body.split('-').collect();
            if parts.len() >= 2 {
                let first_alpha: String = parts[0].chars().filter(|c| c.is_alphabetic()).collect();
                // Многократное заикание одной буквы через дефис: «Н-н-н-н-н-н,» или «Э-э-э-э-э-э-э-э-э-э-э!»
                let all_single_same = first_alpha.chars().count() == 1
                    && parts.iter().all(|p| {
                        let pa: String = p.chars().filter(|c| c.is_alphabetic()).collect();
                        pa.chars().count() == 1 && pa.to_lowercase() == first_alpha.to_lowercase()
                    });

                let last_part = parts.last().unwrap_or(&"");
                let last_first_ch = last_part.chars().find(|c| c.is_alphabetic()).unwrap_or(' ');
                let last_has_word = last_part.chars().filter(|c| c.is_alphabetic()).count() >= 3;
                let all_lead_are_single_match = parts.len() >= 2
                    && last_has_word
                    && parts[..parts.len() - 1].iter().all(|p| {
                        let pa: String = p.chars().filter(|c| c.is_alphabetic()).collect();
                        pa.chars().count() == 1 && pa.to_lowercase() == last_first_ch.to_lowercase().to_string()
                    });

                if all_lead_are_single_match {
                    // Многократный повтор первой буквы перед полным словом: «Н-н-н-ничего» -> «Ничего», «П-поверьте» -> «Поверьте»
                    let is_cap = parts[0].chars().next().map_or(false, |c| c.is_uppercase());
                    let mut sec_chars = last_part.chars();
                    let rebuilt = if is_cap {
                        match sec_chars.next() {
                            Some(fc) => fc.to_uppercase().collect::<String>() + sec_chars.as_str(),
                            None => last_part.to_string(),
                        }
                    } else {
                        last_part.to_string()
                    };
                    words[k] = format!("{}{}", lead_punct, rebuilt);
                    changed = true;
                } else if all_single_same && parts.len() >= 3 {
                    let is_vowel = first_alpha
                        .chars()
                        .next()
                        .map(|c| matches!(c.to_lowercase().next().unwrap_or(' '), 'а' | 'е' | 'ё' | 'и' | 'о' | 'у' | 'ы' | 'э' | 'ю' | 'я'))
                        .unwrap_or(false);
                    let tail_punct: String = body.chars().rev().take_while(|c| !c.is_alphabetic()).collect::<String>().chars().rev().collect();

                    if k + 1 < words.len() && first_alpha.to_lowercase() == "н" && words[k + 1].to_lowercase().starts_with("что") {
                        words[k] = format!("{}На", lead_punct);
                    } else if is_vowel && words.len() == 1 {
                        let low_v = first_alpha.to_lowercase();
                        let cap_v = first_alpha.to_uppercase();
                        let end_p = if tail_punct.is_empty() { "!" } else { tail_punct.as_str() };
                        words[k] = format!("{}{}-{}-{}{}", lead_punct, cap_v, low_v, low_v, end_p);
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

        let w1_alpha_cnt = w1_alpha.chars().count();

        // Случай 1–2 букв с запятой, точкой или многоточием: «Г,», «В…», «П...», «По… Постойте»
        if (1..=2).contains(&w1_alpha_cnt)
            && matches!(w1_punct, "," | "…" | "..." | ".." | ".")
        {
            let w1_low = w1_alpha.to_lowercase();
            let w2_alpha: String = w2
                .chars()
                .skip_while(|c| !c.is_alphabetic())
                .take_while(|c| c.is_alphabetic())
                .collect();
            let w2_low = w2_alpha.to_lowercase();

            let is_stutter_match = if w1_alpha_cnt == 1 {
                !w1_low.is_empty() && w2_low.starts_with(&w1_low)
            } else {
                matches!(w1_punct, "…" | "..." | "..")
                    && w2_low.starts_with(&w1_low)
                    && w2_alpha.chars().count() >= 5
            };

            if is_stutter_match {
                let first_is_upper = w1_alpha.chars().next().map_or(false, |c| c.is_uppercase());
                if first_is_upper && !w2.is_empty() {
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

    // Приставки перед разорванным словом: «не|дооценить», «по|скольку», «на|верняка»
    if (w_prev_lower == "не"
        && (w_next_alpha_lower.starts_with("дооцен")
            || w_next_alpha_lower == "ужели"
            || w_next_alpha_lower == "смотря"
            || w_next_alpha_lower.starts_with("достаточ")
            || w_next_alpha_lower.starts_with("возмож")
            || w_next_alpha_lower.starts_with("вероят")))
        || (w_prev_lower == "по"
            && (w_next_alpha_lower == "скольку"
                || w_next_alpha_lower == "просту"
                || w_next_alpha_lower == "всюду"))
        || (w_prev_lower == "на"
            && (w_next_alpha_lower.starts_with("стойчив")
                || w_next_alpha_lower == "верняка"
                || w_next_alpha_lower == "сквозь"
                || w_next_alpha_lower == "пролет"))
        || (w_prev_lower == "до" && w_next_alpha_lower.starts_with("статоч"))
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
        "ережнее", "ено", "ительности", "ельности", "равляющий",
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

            // 0.5. Дедупликация 1–4 слов, повторенных на стыке двух соседних блоков одной незаконченной фразы
            // (например «...случайно найти в его вещичках» + «В его вещичках нужны документы...» -> убираем повтор «В его вещичках» из начала второй строки)
            if gap <= 0.35 && !crate::text_analyzer::has_terminal_punctuation(&prev.text) {
                let prev_tokens: Vec<&str> = prev.text.split_whitespace().collect();
                let curr_tokens: Vec<&str> = curr_text.split_whitespace().collect();
                let max_k = 4.min(prev_tokens.len()).min(curr_tokens.len().saturating_sub(1));
                let mut matched_k = 0usize;
                for k in (1..=max_k).rev() {
                    let prev_slice = &prev_tokens[prev_tokens.len() - k..];
                    let curr_slice = &curr_tokens[..k];
                    let all_match = prev_slice.iter().zip(curr_slice.iter()).all(|(a, b)| {
                        let na: String = a
                            .chars()
                            .filter(|c| c.is_alphabetic())
                            .collect::<String>()
                            .to_lowercase()
                            .replace('ё', "е");
                        let nb: String = b
                            .chars()
                            .filter(|c| c.is_alphabetic())
                            .collect::<String>()
                            .to_lowercase()
                            .replace('ё', "е");
                        !na.is_empty() && na == nb
                    });
                    // Для одиночного слова (`k == 1`) требуем длину >= 4 букв, чтобы случайно не удалить парный союз
                    if all_match {
                        if k == 1 {
                            let w_len = curr_slice[0].chars().filter(|c| c.is_alphabetic()).count();
                            if w_len >= 4 {
                                matched_k = 1;
                                break;
                            }
                        } else {
                            matched_k = k;
                            break;
                        }
                    }
                }
                if matched_k > 0 {
                    curr_text = curr_tokens[matched_k..].join(" ");
                    repaired_count += 1;
                }
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

                    let is_koe_compound = w_prev == "кое"
                        && matches!(
                            w_next_alpha.as_str(),
                            "что" | "кто" | "где" | "как" | "куда" | "кому" | "кем" | "чем" | "какой" | "какая" | "какие"
                        );

                    if is_koe_compound || is_bound_russian_word_tail(&w_prev, &w_next_alpha) {
                        repaired_count += 1;
                        word_stitched = true;
                        let joiner = if is_koe_compound { "-" } else { "" };
                        let remainder = curr_text[w_next_raw.len()..].trim_start().to_string();
                        let combined_chars = prev.text.chars().count() + curr_text.chars().count();

                        // Если после склейки слова вторая строка пустая или короткая (общая длина <= 95 символов) —
                        // объединяем оба блока в одну цельную фразу!
                        if remainder.is_empty() || combined_chars <= 95 {
                            prev.text.push_str(joiner);
                            prev.text.push_str(&w_next_raw);
                            if !remainder.is_empty() {
                                prev.text.push(' ');
                                prev.text.push_str(&remainder);
                            }
                            prev.text = fix_common_whisper_glued_words(&prev.text);
                            prev.end = prev.end.max(frag.end);
                            continue;
                        } else {
                            // Иначе приклеиваем оторванный хвост слова к `prev`, а остаток фразы оставляем в `curr`
                            prev.text.push_str(joiner);
                            prev.text.push_str(&w_next_raw);
                            prev.text = fix_common_whisper_glued_words(&prev.text);
                            curr_text = remainder;
                        }
                    }
                }
            }

            // 2.5. Если `prev` оборвался без финальной пунктуации (например «Довольно, Существо Икс! Я все твои невзгоды и испытания»),
            // а `curr_text` начинается со строчного завершения этой фразы (1–4 слова со знаком `.`, `!`, `?`, `…`), после которого в том же `curr_text`
            // с заглавной буквы начинается уже следующее предложение («играючи пройду! Нечего было недооценивать...») —
            // переносим этот короткий хвост в конец `prev`, чтобы оба предложения стали грамматически полными!
            if !word_stitched
                && gap <= 0.35
                && !crate::text_analyzer::has_terminal_punctuation(&prev.text)
                && curr_text.chars().next().map_or(false, |c| c.is_lowercase())
            {
                let mut split_byte_idx: Option<usize> = None;
                let char_indices: Vec<(usize, char)> = curr_text.char_indices().collect();
                for idx in 0..char_indices.len() {
                    let (byte_pos, ch) = char_indices[idx];
                    if matches!(ch, '!' | '?' | '.' | '…') {
                        // Пропускаем повторяющиеся знаки («...», «?!»)
                        let mut end_idx = idx;
                        while end_idx + 1 < char_indices.len()
                            && matches!(char_indices[end_idx + 1].1, '!' | '?' | '.' | '…')
                        {
                            end_idx += 1;
                        }
                        let after_punct_byte =
                            char_indices[end_idx].0 + char_indices[end_idx].1.len_utf8();
                        let rest_slice = curr_text[after_punct_byte..].trim_start();
                        if rest_slice.chars().next().map_or(false, |c| c.is_uppercase()) {
                            let prefix_slice = curr_text[..after_punct_byte].trim();
                            let prefix_words = prefix_slice.split_whitespace().count();
                            let prefix_chars = prefix_slice.chars().count();
                            if (1..=4).contains(&prefix_words) && prefix_chars <= 36 {
                                split_byte_idx = Some(after_punct_byte);
                            }
                        }
                        let _ = byte_pos;
                        break;
                    }
                }

                if let Some(split_at) = split_byte_idx {
                    let prefix_tail = curr_text[..split_at].trim();
                    let remainder = curr_text[split_at..].trim_start().to_string();
                    if !prefix_tail.is_empty() && !remainder.is_empty() {
                        prev.text.push(' ');
                        prev.text.push_str(prefix_tail);
                        prev.text = fix_common_whisper_glued_words(&prev.text);
                        curr_text = fix_common_whisper_glued_words(&remainder);
                        repaired_count += 1;
                    }
                }
            }

            // 3. Склеиваем короткие оторванные хвосты предложений (1–3 слова, до 28 символов, например «ты так.», «бою.», «нужна сила.», «вооружие.», «бойцом.»),
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
                    prev.text = fix_common_whisper_glued_words(&prev.text);
                    prev.end = prev.end.max(frag.end);
                    repaired_count += 1;
                    continue;
                }

                let is_short_orphan_tail = !starts_dialog
                    && (curr_has_punct || (!starts_subordinate && curr_words == 1))
                    && ((starts_lower && curr_words <= 3 && curr_chars <= 28 && curr_dur <= 2.2)
                        || (curr_words == 1 && curr_chars <= 14 && curr_dur <= 1.0 && gap <= 0.08));

                if is_short_orphan_tail && (prev.text.chars().count() + 1 + curr_chars) <= 105 {
                    prev.text.push(' ');
                    prev.text.push_str(&curr_text);
                    prev.text = fix_common_whisper_glued_words(&prev.text);
                    prev.end = prev.end.max(frag.end);
                    repaired_count += 1;
                    continue;
                }
            }
        }

        out.push(crate::text_analyzer::SubtitleFragment {
            start: frag.start,
            end: frag.end,
            text: fix_common_whisper_glued_words(&curr_text),
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

        let prev_new_end = if i > 0 { fragments[i - 1].end } else { 0.0 };

        // Самовосстановление ранее кэшированных SRT-файлов, в которых старый алгоритм уплотнил реплику с предшествующей тишиной
        // назад к `prev.end + 0.05` (например «Ли, так кого надо проверить?», «А это ещё что?», «Давайте уже перейдём к сути дела.»),
        // создав искусственную дыру 2.0–6.5с перед прямым ответом `i + 1`!
        if actual_dur <= max_valid_dur {
            if i > 0 && i + 1 < len {
                let gap_from_prev_raw = orig_start - raw_bounds[i - 1].1;
                let gap_to_next_raw = raw_bounds[i + 1].0 - orig_end;
                let curr_trim = fragments[i].text.trim();
                let next_trim = fragments[i + 1].text.trim();
                let is_question_or_initiator = curr_trim.ends_with('?')
                    || crate::text_analyzer::starts_with_dialogue_response(next_trim)
                    || curr_trim.to_lowercase().contains("перейдём к сути")
                    || curr_trim.to_lowercase().contains("перейдем к сути");

                if (gap_from_prev_raw - 0.05).abs() <= 0.025
                    && (1.8..=6.5).contains(&gap_to_next_raw)
                    && (actual_dur - spoken_dur).abs() <= 0.35
                    && is_question_or_initiator
                    && !crate::text_analyzer::is_new_scene_opener(next_trim)
                {
                    let healed_end = raw_bounds[i + 1].0;
                    let healed_start = (healed_end - actual_dur).max(prev_new_end + 0.30);
                    if (healed_start - orig_start).abs() > 0.20 {
                        fragments[i].start = healed_start;
                        fragments[i].end = healed_end;
                        tightened_count += 1;
                    }
                }
            }
            continue;
        }

        let touches_next_cue = if i + 1 < len {
            (raw_bounds[i + 1].0 - orig_end).abs() <= 0.20
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
        let prev_continues_into_curr = if i > 0 && (orig_start - raw_bounds[i - 1].1).abs() <= 0.25 {
            !crate::text_analyzer::has_terminal_punctuation(&fragments[i - 1].text)
                || crate::text_analyzer::starts_with_lowercase_continuation(&fragments[i].text)
        } else {
            false
        };

        // В декодере `whisper.cpp` при паузе между фразами новый сегмент `i` получает `orig_start = prev.end`,
        // накапливает тишину ПЕРЕД речью и закрывается (`orig_end`) ровно в момент окончания произнесения реплики `i`,
        // сразу передавая эстафету сегменту `i + 1` в точке `orig_end` (`touches_next_cue == true`).
        // Исключение (когда тишина находится ПОСЛЕ реплики `i`, несмотря на `touches_next_cue`):
        // - когда `i` является быстрым коротким ответом в уходящей сцене (`starts_with_dialogue_response`),
        //   а `i + 1` открывает совершенно новую сцену (`is_new_scene_opener`), как «Есть! Ждите с полей славных вестей!» -> «Мы приближаемся к зоне боевых действий!».
        let is_closing_reply_before_new_scene = i > 0
            && i + 1 < len
            && (orig_start - raw_bounds[i - 1].1).abs() <= 0.20
            && crate::text_analyzer::starts_with_dialogue_response(&fragments[i].text)
            && crate::text_analyzer::is_new_scene_opener(&fragments[i + 1].text);

        if touches_next_cue && !prev_continues_into_curr && !is_closing_reply_before_new_scene {
            let new_end = orig_end;
            let new_start = (new_end - spoken_dur)
                .max(orig_start)
                .max(prev_new_end + 0.05)
                .min(new_end - 0.60);
            fragments[i].start = new_start;
            fragments[i].end = new_end;
        } else if !continues_into_next || !touches_next_cue || is_closing_reply_before_new_scene {
            let new_start = orig_start.max(prev_new_end + 0.05);
            let target_end = new_start + spoken_dur;
            let new_end = target_end.min(orig_end).max(new_start + 0.60);
            fragments[i].start = new_start;
            fragments[i].end = new_end;
        } else {
            // Фраза продолжается и из `i - 1`, и вплотную упирается в `i + 1` (внутреннее звено длинной речи) —
            // сохраняем непрерывность стыков с обеих сторон
            fragments[i].start = orig_start.max(prev_new_end + 0.02);
            fragments[i].end = orig_end;
        }

        if (fragments[i].start - orig_start).abs() > 0.05
            || (fragments[i].end - orig_end).abs() > 0.05
        {
            tightened_count += 1;
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
    for idx_raw in 0..raw_fragments.len() {
        let mut frag = raw_fragments[idx_raw].clone();
        let dur = frag.end - frag.start;
        let trimmed = frag.text.trim();
        if dur <= 0.10 {
            // В whisper.cpp с флагом -ml последнее слово сегмента иногда выбрасывается в блок с нулевой длительностью
            // (например `[00:16:55.120 --> 00:16:55.120] стороны` после `...отказ от контрибуций со` или `[00:19:58.200 --> 00:19:58.200] пугает.` после `...тому, что его`).
            // Если предыдущая реплика оборвалась без знака препинания и это короткое строчное продолжение (не дублирующее начало следующей строки),
            // приклеиваем его к предыдущей реплике вместо потери слова!
            if !trimmed.is_empty()
                && !is_whisper_hallucination(trimmed)
                && trimmed.chars().next().map_or(false, |c| c.is_lowercase())
                && trimmed.split_whitespace().count() <= 2
            {
                let next_first_prefix: String = if idx_raw + 1 < raw_fragments.len() {
                    raw_fragments[idx_raw + 1]
                        .text
                        .trim()
                        .chars()
                        .filter(|c| c.is_alphabetic())
                        .take(3)
                        .collect::<String>()
                        .to_lowercase()
                } else {
                    String::new()
                };
                let curr_first_prefix: String = trimmed
                    .chars()
                    .filter(|c| c.is_alphabetic())
                    .take(3)
                    .collect::<String>()
                    .to_lowercase();
                let is_stutter_of_next = curr_first_prefix.chars().count() == 3
                    && curr_first_prefix == next_first_prefix;

                if !is_stutter_of_next {
                    if let Some(prev) = stage1.last_mut() {
                        if (frag.start - prev.end).abs() <= 0.10
                            && !crate::text_analyzer::has_terminal_punctuation(&prev.text)
                        {
                            prev.text.push(' ');
                            prev.text.push_str(trimmed);
                            prev.text = fix_common_whisper_glued_words(&prev.text);
                        }
                    }
                }
            }
            continue;
        }

        if is_whisper_hallucination(trimmed) {
            continue;
        }

        // Одиночное слово без знаков препинания («Вернёмся»), зажатое прямо между музыкальными тегами эндинга/опенинга
        if !crate::text_analyzer::has_terminal_punctuation(trimmed)
            && trimmed.split_whitespace().count() == 1
        {
            let prev_is_music = idx_raw > 0 && is_whisper_hallucination(raw_fragments[idx_raw - 1].text.trim());
            let next_is_music = idx_raw + 1 < raw_fragments.len()
                && is_whisper_hallucination(raw_fragments[idx_raw + 1].text.trim());
            if prev_is_music && next_is_music {
                continue;
            }
        }

        let cleaned_text = clean_stretched_text(trimmed);
        if cleaned_text.is_empty() || is_whisper_hallucination(&cleaned_text) {
            continue;
        }

        // Если видео русскоязычное, а строка состоит из латиницы или содержит английские слова вокала песни OP/ED («Сильвестка не азера, Your move!») — отбрасываем
        if is_predominantly_russian {
            let has_cyr = cleaned_text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
            let lat_count = cleaned_text.chars().filter(|c| c.is_ascii_alphabetic()).count();
            if !has_cyr && lat_count > 0 {
                continue;
            }
            let has_eng_word = cleaned_text
                .split(|c: char| !c.is_ascii_alphabetic())
                .any(|w| w.len() >= 3 && w.chars().any(|ch| matches!(ch.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y')));
            if has_eng_word && lat_count >= 4 {
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
            // Проверяем совпадение общего 3+ словного ядра на конце строки (например «- Так, это значит вы убийцы?», «- Ну, это значит, что вы убийцы?», «- Да, это значит, что вы убийцы?»)
            let same_core = words_i >= 3 && {
                let k_words: Vec<&str> = norm_k.split_whitespace().collect();
                let i_words: Vec<&str> = norm_i.split_whitespace().collect();
                if k_words.len() >= 3 && i_words.len() >= 3 {
                    let k_tail = k_words[k_words.len() - 3..].join(" ");
                    let i_tail = i_words[i_words.len() - 3..].join(" ");
                    k_tail == i_tail
                } else {
                    false
                }
            };
            if same_exact || same_long_prefix || same_core {
                run_end += 1;
            } else {
                break;
            }
        }

        let run_len = run_end - i;
        if run_len >= 4 || (run_len == 3 && words_i >= 5) {
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
            // Если прямо перед длинным вокальным циклом (например 11x «Вернешься.» на опенинге) стоит короткая 3-строчная вокальная петля A-B-A
            // («Не волнуйся, как я.» / «Все еще.» / «Не волнуйся, как я.»), примыкающая вплотную (<= 2.5с), удаляем и её!
            if clean_fragments.len() >= 3 {
                let n = clean_fragments.len();
                let a1 = norm_line(&clean_fragments[n - 3].text);
                let b1 = norm_line(&clean_fragments[n - 2].text);
                let a2 = norm_line(&clean_fragments[n - 1].text);
                let gap_to_loop = (stage1[i].start - clean_fragments[n - 1].end).abs();
                if !a1.is_empty()
                    && a1 == a2
                    && gap_to_loop <= 2.5
                    && a1.split_whitespace().count() <= 4
                    && b1.split_whitespace().count() <= 4
                {
                    clean_fragments.truncate(n - 3);
                }
            }
            // Любая фраза (даже из 1–3 слов, как «ПЕСНЯ ВСХЛИПЫМ» x9) повторенная 4+ раз подряд (или 3+ раз для длинной фразы >= 5 слов) —
            // это 100% зацикленная галлюцинация Whisper на музыке или шуме, отбрасываем весь кластер!
            i = run_end;
            continue;
        } else if run_len >= 2 {
            // Если это парный повтор короткого вокального хвоста в самом конце файла на титрах эндинга («Мы не можем!» x2) — отбрасываем
            if run_end == stage1.len() && norm_i == "мы не можем" {
                i = run_end;
                continue;
            }
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
    // сначала склеиваем разорванные полуслова/хвосты (пока их таймкоды еще вплотную стыкуются) и убираем многострочное эхо,
    // а затем подтягиваем растянутые таймкоды уже цельных предложений!
    for _pass in 0..4 {
        let prev_len = clean_fragments.len();
        let (repaired_frags, s_cnt) = repair_split_words_and_orphans(clean_fragments);
        let dedup_frags = deduplicate_multiline_loops(repaired_frags);
        let (final_frags, t_cnt) = tighten_subtitle_timestamps(dedup_frags);
        total_tightened_cues += t_cnt;
        total_stitched_words += s_cnt;
        let len_changed = final_frags.len() != prev_len;
        clean_fragments = final_frags;
        if t_cnt == 0 && s_cnt == 0 && !len_changed {
            break;
        }
    }

    // Загружаем и применяем пользовательский словарь проекта (если есть replacements.json или dictionary.txt)
    let project_replacements = load_project_replacements(srt_path);
    if !project_replacements.is_empty() {
        crate::logger::emit_log(
            app_handle,
            "Whisper",
            "info",
            &format!("Применён пользовательский словарь проекта: загружено {} правил автокоррекции.", project_replacements.len()),
        );
    }

    // Финальный проход словаря склеенных слов и пользовательских замен после всех объединений строк
    for frag in &mut clean_fragments {
        frag.text = fix_common_whisper_glued_words(&frag.text);
        if !project_replacements.is_empty() {
            frag.text = apply_custom_replacements(&frag.text, &project_replacements);
        }
    }

    let removed = raw_count
        .saturating_sub(clean_fragments.len())
        .saturating_sub(total_stitched_words);

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

    let text_modified = out.trim() != content.replace("\r\n", "\n").trim();

    if removed > 0 || total_stitched_words > 0 || total_tightened_cues > 0 || text_modified {
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
        if !out_norms.is_empty() {
            // Одиночный повтор короткого слова («Сакуя!», «Удачи!», «Черт!»), повторяющегося подряд или через < 3.5с
            if curr_words.len() == 1 {
                let last_idx = out_norms.len() - 1;
                let time_gap = (fragments[i].start - out[last_idx].end).max(0.0);
                if time_gap <= 3.5 && out_norms[last_idx] == norms[i] {
                    i += 1;
                    continue;
                }
            }
        }
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
                        // проверяем зазор по времени:
                        if curr_words.len() >= prev_words.len() + 2 {
                            if time_gap <= 3.0 {
                                out[prev_idx].text = fragments[i].text.clone();
                                out_norms[prev_idx] = norms[i].clone();
                                is_echo = true;
                                break;
                            } else {
                                // Предыдущий короткий фрагмент был фальш-стартом/галлюцинацией на тишине много секунд назад!
                                // Настоящая реплика произносится СЕЙЧАС (в fragments[i]). Удаляем старый фальш-старт и сохраняем текущую реплику!
                                out.remove(prev_idx);
                                out_norms.remove(prev_idx);
                                break;
                            }
                        }
                        is_echo = true;
                        break;
                    }
                }

                // Дробный эхо-повтор: текущая короткая/средняя строка входит в одну из последних 3 более длинных реплик,
                // либо является 2-словным эхо-хвостом прямо после предыдущей фразы («...тому, что его пугает.» -> «Вы пугает.»)
                if d <= 3 && prev_words.len() >= curr_words.len() && prev_words.len() >= 4 {
                    let matched = curr_words
                        .iter()
                        .filter(|&&cw| prev_words.iter().any(|&pw| words_match_fuzzy(cw, pw)))
                        .count();
                    let is_two_word_tail_echo = d == 1
                        && time_gap <= 1.5
                        && curr_words.len() == 2
                        && prev_words.len() >= 5
                        && words_match_fuzzy(
                            curr_words.last().unwrap_or(&""),
                            prev_words.last().unwrap_or(&""),
                        );
                    if matched == curr_words.len()
                        || (curr_words.len() >= 4 && matched * 5 >= curr_words.len() * 4)
                        || is_two_word_tail_echo
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

/// Конвертирует и сохраняет пользовательские готовые субтитры (ASS/SSA/SRT/VTT) в канонический SRT-файл проекта
pub fn process_custom_subtitles_into_project(
    custom_path: &Path,
    project_srt_path: &Path,
) -> AppResult<()> {
    let parse_res = crate::subtitles_parser::parse_custom_subtitles_file(custom_path)
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!(e)))?;

    if let Some(parent) = project_srt_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut srt_content = String::new();
    for (idx, line) in parse_res.lines.iter().enumerate() {
        let start_h = (line.start / 3600.0) as u32;
        let start_m = ((line.start % 3600.0) / 60.0) as u32;
        let start_s = (line.start % 60.0) as u32;
        let start_ms = ((line.start - line.start.floor()) * 1000.0) as u32;

        let end_h = (line.end / 3600.0) as u32;
        let end_m = ((line.end % 3600.0) / 60.0) as u32;
        let end_s = (line.end % 60.0) as u32;
        let end_ms = ((line.end - line.end.floor()) * 1000.0) as u32;

        let text_with_char = if line.character != "Общий диктор" && !line.text.to_lowercase().starts_with(&line.character.to_lowercase()) {
            format!("{}: {}", line.character, line.text)
        } else {
            line.text.clone()
        };

        srt_content.push_str(&format!(
            "{}\n{:02}:{:02}:{:02},{:03} --> {:02}:{:02}:{:02},{:03}\n{}\n\n",
            idx + 1,
            start_h, start_m, start_s, start_ms,
            end_h, end_m, end_s, end_ms,
            text_with_char
        ));
    }

    std::fs::write(project_srt_path, srt_content)
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("Не удалось записать готовые субтитры проекта: {}", e)))?;

    Ok(())
}
