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
    OpenAi,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextAnalysisConfig {
    pub engine: TextEngine,
    pub model_name: String,       // for LlamaCli: path to ".gguf" or name, for Ollama: "qwen2.5:1.5b", for OpenAi: "gpt-4o-mini"
    pub ollama_url: String,       // e.g. "http://localhost:11434"
    pub openai_base_url: String,  // e.g. "https://api.openai.com/v1"
    pub openai_api_key: String,   // Bearer token / API key
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

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    temperature: f64,
    stream: bool,
}

#[derive(Deserialize)]
struct OpenAiChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiChoiceMessage,
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
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
pub fn starts_with_lowercase_continuation(text: &str) -> bool {
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
/// (например: «Поэтому он все еще сидит...», «И в моих руках он...», «К тому же...», «Потому что...», «Я как следует им закупилась...»)
pub fn starts_with_discourse_continuation(text: &str) -> bool {
    let clean = text
        .trim()
        .trim_start_matches(['-', '—', '"', '\'', '«', '(', '[', ' '])
        .to_lowercase()
        .replace('ё', "е");

    let continuation_prefixes = [
        "поэтому-то",
        "поэтому,",
        "поэтому ",
        "потому-то",
        "потому что ",
        "так что ",
        "все-таки",
        "всё-таки",
        "ведь,",
        "ведь ",
        "причем,",
        "причем ",
        "к тому же",
        "более того",
        "вдобавок",
        "да баллак",
        "кроме того",
        "тем более",
        "однако,",
        "однако ",
        "иначе,",
        "иначе говоря",
        "значит,",
        "зато ",
        "только вот",
        "и раз ",
        "раз уж ",
        "пусть это",
        "при этом",
        "после этого",
        "из-за этого",
        "в итоге",
        "в этом ",
        "для этого",
        "мне для этого",
        "на большее",
        "на большей",
        "лишь бы ",
        "лишь и быстро",
        "он еще и ",
        "она еще и ",
        "они еще и ",
        "и в моих ",
        "и поэтому ",
        "и тогда ",
        "и все же",
        "и при этом",
        "а также ",
        "а значит",
        "а потом ",
        "а еще,",
        "а еще ",
        "но и ",
        "но вот что",
        "но я ",
        "но война",
        "но что в итоге",
        "но почему",
        "да и ",
        "или же ",
        "во многом из-за",
        "вам многом из-за",
        "благодаря этому",
        "в результате ",
        "другими словами",
        "короче говоря",
        "тем не менее",
        "например,",
        "например ",
        "к примеру",
        "очень в духе",
        "в духе ",
        "впрочем,",
        "впрочем ",
        "то есть,",
        "то есть ",
        "легкая остринка",
        "легкая астринка",
        "им не нужно",
        "им не надо",
        "не то что ",
        // Анафорические зачины (местоимения 3-го лица и указательные обороты в начале фразы, требующие контекста предыдущей реплики):
        "из его ",
        "из ее ",
        "из их ",
        "у него ",
        "у нее ",
        "у них ",
        "от него ",
        "от нее ",
        "от них ",
        "к нему ",
        "к ней ",
        "к ним ",
        "с ним ",
        "с ней ",
        "с ними ",
        "за ним ",
        "за ней ",
        "за ними ",
        "над ним ",
        "над ней ",
        "о нем ",
        "о ней ",
        "о них ",
        "про него ",
        "про нее ",
        "про них ",
        "его сразу",
        "его ",
        "ее ",
        "их ",
        "он получил",
        "он, как и",
        "она стреляет",
        "она прикрыла",
        "она хроли",
        "они жаловали",
        "они решили",
        "они всерьез",
        "это явно не",
        "тот самый",
        "такого ",
        "такой ",
        "такая ",
        "такие ",
        "этого ",
        "этой ",
        "этому ",
        "этим ",
    ];

    if continuation_prefixes.iter().any(|p| clean.starts_with(p)) {
        return true;
    }

    // Анафорические местоимения 3-го лица в первых 4 словах («Я как следует им закупилась...», «Они с ним не справятся...»),
    // которые отсылают к существительному из предыдущей реплики («...настоящего кофе.»)
    let first_words: Vec<&str> = clean
        .split(|c: char| !c.is_alphabetic() && c != '-')
        .filter(|w| !w.is_empty())
        .take(5)
        .collect();
    if first_words.iter().skip(1).any(|&w| {
        matches!(
            w,
            "им" | "ими" | "них" | "ними" | "него" | "нее" | "нем" | "ней" | "этим" | "этого" | "этому"
        )
    }) {
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

/// Проверяет, является ли реплика явным открытием НОВОЙ сцены / локации / радиосвязи,
/// чтобы предыдущая сцена НЕ склеивалась с ней через границу ракурса!
pub fn is_new_scene_opener(text: &str) -> bool {
    let clean = text
        .trim()
        .trim_start_matches(['-', '—', '"', '\'', '«', '(', '[', ' '])
        .to_lowercase()
        .replace('ё', "е");

    let scene_openers = [
        "мы приближаемся к зоне",
        "мы приближаемся к сони",
        "срочное донесение",
        "командир федерации",
        "саламандра — транспортировщику",
        "саламандра-транспортировщику",
        "прибыли точно по расписанию",
        "приволеточно по расписанию",
        "сегодня проходит прощание",
        "единственный, кто мог занять место",
        "внимание! засек противника",
        "внимание! за все противника",
        "есть визуальный контакт",
        "фиксируй магический след",
        "сначала с запада на восток",
        "сначала запада на восток",
    ];

    scene_openers.iter().any(|p| clean.starts_with(p))
}

/// Проверяет, заканчивается ли текст реплики грамматически незавершенным придаточным оборотом,
/// союзом, предлогом или подлежащим без сказуемого (например: «Не сомневаюсь, что и этот путь.», «к стене, в преграде.», «что для меня»)
pub fn has_incomplete_clause(text: &str) -> bool {
    let clean = text
        .trim()
        .trim_matches(['"', '\'', '»', '«', '(', ')', '[', ']', '-', '—'])
        .trim_end_matches(['.', '!', '?', '…', ' ', ','])
        .to_lowercase()
        .replace('ё', "е");

    let tokens: Vec<&str> = clean.split_whitespace().collect();
    if tokens.is_empty() {
        return false;
    }

    let last_tok = tokens.last().unwrap_or(&"");

    // Заканчивается на союз, предлог, связку или местоимение
    let dangling_tails = [
        "что", "чтобы", "как", "если", "когда", "хотя", "пусть", "будто", "словно",
        "потому", "так", "где", "куда", "откуда", "зачем", "почему", "чей", "чья", "чье", "чьи",
        "который", "которая", "которое", "которые", "какой", "какая", "какое", "какие",
        "этот", "эта", "это", "эти", "тот", "та", "то", "те", "такой", "такая", "такое", "такие",
        "свой", "своя", "свое", "свои", "мой", "моя", "мое", "мои", "твой", "твоя", "твое", "твои",
        "наш", "наша", "наше", "наши", "ваш", "ваша", "ваше", "ваши",
        "и", "а", "но", "да", "или", "ли", "же", "в", "на", "к", "с", "из", "от", "до", "по", "для", "о", "об", "про", "без",
        "готов", "должно", "может", "могут", "будет", "стал", "был",
    ];

    if dangling_tails.contains(last_tok) {
        return true;
    }

    // Сочетания незавершенной мысли в конце фразы:
    let incomplete_tails = [
        "что и этот путь",
        "что этот путь",
        "что и этот",
        "что этот",
        "что эта",
        "что эти",
        "что данн",
        "что такой",
        "что такая",
        "что такое",
        "что такие",
        "что для меня",
        "что для нас",
        "что со мной",
        "что с ним",
        "потому что",
        "так как",
        "для того чтобы",
        "вместо того чтобы",
        "не сомневаюсь, что",
        "не сомневаюсь что",
        "уверен, что",
        "уверен что",
        "думаю, что",
        "думаю что",
        "знаю, что",
        "знаю что",
        "кажется, что",
        "кажется что",
        "кто-то готов",
        "вы хотите сказать",
    ];

    if incomplete_tails.iter().any(|t| clean.ends_with(t) || clean == *t) {
        return true;
    }

    // Если фраза содержит «что и этот ...» или «не сомневаюсь, что и ...» и в ней нет глагола действия (сказуемого)
    if clean.contains("что и этот") || clean.contains("не сомневаюсь, что и этот") {
        let has_verb = tokens.iter().any(|w| {
            w.ends_with("ет") || w.ends_with("ит") || w.ends_with("ут") || w.ends_with("ят")
                || w.ends_with("ел") || w.ends_with("ил") || w.ends_with("ал") || w.ends_with("ла")
                || w.ends_with("ло") || w.ends_with("ли") || w.ends_with("ся") || w.ends_with("сь")
                || w.ends_with("ть") || w.ends_with("ти")
        });
        if !has_verb {
            return true;
        }
    }

    false
}

/// Проверяет, начинается ли реплика с диалогового ответа / реакции или зависимого вопроса к предыдущей реплике собеседника
/// («Вы уверены?», «Хорошо, полагаюсь на тебя!», «Да, конечно», «И все же?», «Поможем союзникам отступить!»)
pub fn starts_with_dialogue_response(text: &str) -> bool {
    let clean = text
        .trim()
        .trim_start_matches(['-', '—', '"', '\'', '«', '(', '[', ' '])
        .to_lowercase()
        .replace('ё', "е");

    let responsive_prefixes = [
        "хорошо,", "хорошо!", "хорошо ",
        "ладно,", "ладно!", "ладно ",
        "да,", "да!", "да ", "да нет", "да уж",
        "нет,", "нет!", "нет ",
        "согласен,", "согласен!", "согласен ",
        "понял,", "понял!", "понял ", "я понял",
        "ясно,", "ясно!", "ясно ",
        "есть!", "так точно", "договорились",
        "благодарю,", "благодарю ", "спасибо,", "спасибо ",
        "конечно,", "конечно ", "само собой", "разумеется",
        "верно,", "верно ", "точно,", "точно ", "правда,", "правда?",
        "вы уверены", "ты уверен", "ты уверена",
        "серьезно?", "неужели?", "вот как", "в смысле", "то есть как",
        "это еще почему", "с чего бы", "и что дальше", "и что теперь",
        "и что с того", "и что же ", "и все же", "что это значит",
        "о чем ты", "о чем вы", "ты о чем", "вы о чем", "к чему ты",
        "как это понимать", "как это вообще понимать",
        "почему все так", "ну почему", "но почему",
        "так вот чего", "так вот что", "так вот почему",
        "знакомая атака", "опять она",
        "справишься", "воберешься",
        "разве мы не", "разве вы не", "а не слишком ли",
        "вы простите", "вы ж простите", "вы уж простите",
        "простите, вы хотите", "надеюсь, вопрос закрыт",
        "поможем союзникам", "лейтенант серебрякова", "остальные сопровождайте",
        "не подпускать их", "мне подпускать их",
    ];

    responsive_prefixes.iter().any(|p| clean.starts_with(p))
}

/// Проверяет, является ли реплика `idx` логическим НАЧАЛОМ нового предложения / законченной мысли
pub fn is_sentence_start(fragments: &[SubtitleFragment], idx: usize) -> bool {
    if idx == 0 || idx >= fragments.len() {
        return true;
    }
    let prev = &fragments[idx - 1];
    let curr = &fragments[idx];
    let gap = (curr.start - prev.end).max(0.0);
    let curr_trimmed = curr.text.trim();

    if is_new_scene_opener(curr_trimmed) {
        return true;
    }

    // Если текущая строка начинается с маленькой буквы (или предыдущая не имеет финальной точки) — это середина незаконченного предложения!
    if starts_with_lowercase_continuation(curr_trimmed)
        && (gap < 3.8 || !has_terminal_punctuation(&prev.text))
    {
        return false;
    }
    if !has_terminal_punctuation(&prev.text) && gap < 3.0 {
        return false;
    }

    // ВАЖНО: Проверяем причинно-следственную / анафорическую привязку («Поэтому...», «Из его кристалла...», «Все-таки...»)
    // ДО проверки паузы `gap >= 1.25`, так как после уплотнения таймкодов пауза между тесно связанными репликами одного монолога может составлять 1.3с–3.6с!
    if starts_with_discourse_continuation(curr_trimmed) && gap < 3.8 {
        return false;
    }

    // Если текущая реплика — короткий ответ или зависимый вопрос («Вы уверены?», «Поможем союзникам отступить!»),
    // примыкающий к предыдущей реплике с малой паузой (< 2.2с), не начинаем клип без предыдущей реплики!
    if starts_with_dialogue_response(curr_trimmed) && gap < 2.2 {
        return false;
    }

    // Если предыдущая реплика закончилась вопросом (`?`) и текущая идет с паузой < 2.5с — текущая реплика является ОТВЕТОМ на вопрос,
    // поэтому клип должен начинаться с самого вопроса (`idx - 1`), а не с ответа!
    if prev.text.trim().ends_with('?') && gap < 2.5 {
        return false;
    }

    // Если между репликами длинная пауза (>= 1.25с) и предыдущая завершилась знаком препинания — начало новой фразы/сцены
    if gap >= 1.25 && has_terminal_punctuation(&prev.text) && !has_incomplete_clause(&prev.text) {
        return true;
    }

    // Если между репликами заметная пауза (>= 0.70с) и предыдущая фраза закончилась знаком препинания
    if gap >= 0.70 && has_terminal_punctuation(&prev.text) && !has_incomplete_clause(&prev.text) {
        return true;
    }

    // Если текущая строка начинается с диалогового тире («- » или «— ») и предыдущая завершилась точкой/!/? — новая реплика собеседника
    if (curr_trimmed.starts_with('-') || curr_trimmed.starts_with('—')) && has_terminal_punctuation(&prev.text) {
        return true;
    }

    // Если предыдущая строка закончилась точкой, !, ? или многоточием — текущая начинает новое предложение (если у предыдущей нет незавершенного придаточного)
    if has_terminal_punctuation(&prev.text) && !has_incomplete_clause(&prev.text) {
        return true;
    }

    // Иначе предыдущая строка оборвалась на запятой или на полуслове — значит это продолжение фразы
    false
}

/// Проверяет, является ли реплика `idx` слабым, неинформативным началом для клипа
/// (короткие ответы-подтверждения на предыдущую фразу, одиночные вопросы без контекста, либо последняя фраза уходящей сцены прямо перед сменой ракурса)
pub fn is_weak_clip_start(fragments: &[SubtitleFragment], idx: usize) -> bool {
    if idx >= fragments.len() {
        return false;
    }
    let clean = fragments[idx]
        .text
        .trim()
        .trim_start_matches(['-', '—', ' '])
        .trim_end_matches(['.', '!', '?', '…', '"', '»', ' '])
        .to_lowercase()
        .replace('ё', "е");

    let weak_openers = [
        "да",
        "нет",
        "угу",
        "ага",
        "ясно",
        "понятно",
        "принято",
        "хорошо",
        "ладно",
        "точно",
        "то",
        "вот как",
        "это да",
        "ну да",
        "конечно",
        "согласен",
        "совершенно верно",
        "вы совершенно правы",
        "ничего страшного",
        "так точно",
        "вы уверены",
        "ты уверен",
        "ты уверена",
        "понимаете",
        "это срочно",
        "странное дело",
        "знакомая атака",
        "опять она",
        "не нужно",
        "ненужно",
        "я понял",
        "вылетаем",
        "стоять",
        "какого",
        "что это",
        "так точно. сразу этим займусь",
        "ох, прошу прощения, что сразу не признал",
    ];

    if weak_openers.contains(&clean.as_str()) {
        return true;
    }

    // Если сразу после текущей короткой реплики (`idx + 1`) открывается НОВАЯ сцена (`is_new_scene_opener`),
    // а перед текущей репликой была длинная пауза (`>= 2.0с`) — значит `idx` это оторванный последний выкрик предыдущей сцены
    // (например «Я тебе крылышки-то пообломаю!» за 3с до «Мы приближаемся к зоне боевых действий!»), с него НЕЛЬЗЯ начинать склеенный клип!
    if idx + 1 < fragments.len() && is_new_scene_opener(&fragments[idx + 1].text) {
        let dur_curr = fragments[idx].end - fragments[idx].start;
        if dur_curr < 5.0 {
            return true;
        }
    }

    false
}

/// Проверяет, является ли реплика `idx` слабым/оборванным финалом для клипа
/// (незакрытые вопросы `?`, короткие поддакивания, переспросы, реплики-подводки перед ответом или нулевая пауза посреди монолога)
pub fn is_weak_clip_ending(fragments: &[SubtitleFragment], idx: usize) -> bool {
    if idx >= fragments.len() {
        return false;
    }
    let curr = &fragments[idx];
    let raw_trimmed = curr.text.trim();

    // 0. Если в конце строки вообще нет завершающего знака препинания (., !, ?, …) — мысль оборвана на середине
    if !has_terminal_punctuation(raw_trimmed) {
        return true;
    }

    // Грамматически незавершенное придаточное («Не сомневаюсь, что и этот путь.», «...если кто-то готов»)
    if has_incomplete_clause(raw_trimmed) {
        return true;
    }

    let clean = raw_trimmed
        .trim_start_matches(['-', '—', ' '])
        .trim_end_matches(['.', '!', '?', '…', '"', '»', ' '])
        .to_lowercase()
        .replace('ё', "е");
    let word_count = clean.split_whitespace().count();

    // 1. Многоточие в конце («Вам многом из-за...», «Проклятым монстром...») — мысль явно не договорена
    if raw_trimmed.ends_with("...") || raw_trimmed.ends_with('…') {
        return true;
    }

    // 2. Короткие реплики-поддакивания и переходные фразы (1–3 слова), которые не несут финального панчлайна
    let weak_backchannels = [
        "да уж", "ясно", "понятно", "угу", "ага", "это да", "вот как", "точно",
        "да", "нет", "ну да", "конечно", "согласен", "интересно", "привет",
        "разрешите войти", "разрешите представиться", "прошу сюда", "вот именно",
        "ничего страшного", "принято", "совершенно верно", "вы совершенно правы",
        "например", "к примеру", "какой еще сюрприз",
        "знакомая атака", "это срочно", "странное дело", "понимаете",
        "есть визуальный контакт", "на нас напали", "вы уверены", "ты уверен",
        "простите, вы хотите сказать", "вы посетите прощения", "где генерал",
        "не нужно", "ненужно", "я понял", "вылетаем", "стоять",
    ];
    if weak_backchannels.contains(&clean.as_str()) {
        return true;
    }

    // 3. Катафорические фразы-подводки и двухшаговые зачины, которые анонсируют следующую реплику
    let setup_phrases = [
        "а зовут его",
        "зовут его",
        "вывод из нее такой",
        "добавлю еще одну деталь",
        "добавлю еще",
        "более того",
        "кое-что уточню",
        "слушай внимательно",
        "тема сегодняшнего совещания",
        "тема сегодняшнего",
        "проще говоря",
        "сначала они ",
        "сначала мы ",
        "сначала я ",
        "а потом еще и ",
        "и пока я ",
        "а раз так",
        "меня об этом не предупреждали",
        "даже наша звезда",
        "не подпускать их",
        "мне подпускать их",
        "мы приближаемся к",
    ];
    if setup_phrases.iter().any(|p| clean.starts_with(p) || clean.ends_with(p)) {
        return true;
    }

    // 4. Приветствия, воинские звания, обращения при встрече и вводные вопросы к новой теме
    if clean.starts_with("прости за ожидан")
        || clean.starts_with("значит, и есть")
        || clean.starts_with("значит, ты и есть")
        || clean.contains("есть то самое дочка")
        || clean.contains("есть та самая дочка")
        || clean.starts_with("заходи, я представл")
        || clean.starts_with("госпожа обер")
        || clean.starts_with("господин обер")
        || clean.starts_with("товарищ полковник,")
        || clean.starts_with("товарищ генерал,")
        || clean.contains("хочет подготовить сюрприз")
        || clean.contains("вы же понимаете, что мы не можем")
        || clean.contains("как считаете, на какие уступки")
        || clean.contains("могу я быть с вами честен")
        || clean.contains("я бы тоже хотел услышать ваше мнение")
        || clean.contains("воединамо не с чего начинать")
        || clean.contains("вы единым понес чего начинать")
    {
        return true;
    }

    if word_count <= 3
        && !raw_trimmed.ends_with('!')
        && [
            "генерал",
            "полковник",
            "подполковник",
            "майор",
            "капитан",
            "лейтенант",
            "оберст",
            "обер-лейтенант",
            "гауптман",
            "господин",
            "госпожа",
            "товарищ",
            "офицер",
            "здравствуйте",
            "добрый день",
            "доброе утро",
            "добрый вечер",
        ]
        .iter()
        .any(|rank| clean.starts_with(rank))
    {
        return true;
    }

    // Одиночное короткое слово-выкрик в самом конце клипа («Бой!», «Ай!», «Ой!», «Эй!», «Что?!», «Стой!»)
    if word_count == 1
        && idx > 0
        && has_terminal_punctuation(&fragments[idx - 1].text)
        && (clean.chars().count() <= 7
            || matches!(
                clean.as_str(),
                "бой" | "ай" | "ой" | "эй" | "стой" | "стоять" | "что" | "а" | "ох" | "ах" | "ух" | "хм" | "ну"
            ))
    {
        return true;
    }

    // 5. Анализ связки со СЛЕДУЮЩЕЙ репликой (`idx + 1`):
    //    - Вопрос (`?`), за которым в пределах 3.5с идет ответ собеседника («Это там... Рейнский дьявол?», «Что вообще происходит?», «Вы уверены?») —
    //      обрывать шортс на вопросе без ответа категорически нельзя!
    //    - Короткая 1–3 словная фраза без `!` перед следующей репликой («Знакомая атака.» -> «Опять она!», «Это срочно.»)
    //    - Нулевая пауза (`gap_next <= 0.25с`), когда тот же персонаж продолжает свою мысль («...не в восторге.» -> «Надеюсь, доведется ещё рассказать...»,
    //      «...полдела сделано!» -> «Ну, что? Понял, что со мной шутки плохи, Существо Икс?!»)
    if idx + 1 < fragments.len() {
        let next_trimmed = fragments[idx + 1].text.trim();
        let next_clean = next_trimmed
            .trim_start_matches(['-', '—', ' '])
            .to_lowercase()
            .replace('ё', "е");
        let gap_next = (fragments[idx + 1].start - curr.end).max(0.0);
        let next_is_valid_speech = !crate::whisper::is_whisper_hallucination(next_trimmed)
            && !is_opening_or_song_fragment(fragments, idx + 1)
            && !is_new_scene_opener(next_trimmed);

        if next_is_valid_speech {
            // Любой вопрос (кроме риторического финала перед длинной паузой > 3.2с) требует ответа!
            let is_pure_q = raw_trimmed.ends_with('?')
                && !raw_trimmed.ends_with("?!")
                && !raw_trimmed.ends_with("!?");
            if is_pure_q && gap_next <= 3.2 {
                return true;
            }
            // Эмоциональный вопрос с `?!` / `!?`, если ответ/продолжение звучит сразу же (`gap_next <= 1.5с`)
            if (raw_trimmed.ends_with("?!") || raw_trimmed.ends_with("!?")) && gap_next <= 1.5 {
                return true;
            }
            if word_count == 1 && gap_next < 2.2 {
                return true;
            }
            // Короткая 1–3 словная фраза с точкой («Знакомая атака.», «Странное дело.»), за которой в пределах 2.2с идет реакция
            if word_count <= 3 && !raw_trimmed.contains('!') && gap_next <= 2.2 {
                return true;
            }
            // Если следующая реплика начинается втугую (`gap_next <= 0.25с`) и является выводом/панчлайном или продолжением мысли того же говорящего
            if gap_next <= 0.25
                && (starts_with_discourse_continuation(next_trimmed)
                    || next_clean.starts_with("надеюсь,")
                    || next_clean.starts_with("ну, что")
                    || next_clean.starts_with("ну что")
                    || next_clean.starts_with("опять она")
                    || next_clean.starts_with("меня об этом")
                    || next_clean.starts_with("и раз я ")
                    || !raw_trimmed.contains('!'))
            {
                return true;
            }
        }

        if word_count <= 5
            && raw_trimmed.ends_with('!')
            && gap_next <= 0.30
            && !next_trimmed.starts_with('-')
            && !next_trimmed.starts_with('—')
            && (clean.starts_with("какая ")
                || clean.starts_with("какой ")
                || clean.starts_with("как, как")
                || clean.starts_with("как так")
                || clean.starts_with("вот засад")
                || clean.starts_with("да вы "))
        {
            return true;
        }
        if gap_next <= 0.55
            && (next_clean.starts_with("а еще") || next_clean.starts_with("и еще"))
        {
            return true;
        }
        if gap_next <= 0.40
            && (clean.starts_with("позвольте, но ")
                || clean.starts_with("возвольте, но ")
                || clean.contains("всего лишь "))
        {
            return true;
        }
    }

    false
}

/// Проверяет, является ли реплика `idx` логическим КОНЦОМ предложения / законченной мысли
pub fn is_sentence_end(fragments: &[SubtitleFragment], idx: usize) -> bool {
    if idx + 1 >= fragments.len() {
        return has_terminal_punctuation(&fragments[idx].text) && !has_incomplete_clause(&fragments[idx].text);
    }
    let curr = &fragments[idx];
    let next = &fragments[idx + 1];
    let gap_next = (next.start - curr.end).max(0.0);
    let next_trimmed = next.text.trim();

    // Если текущая строка вообще не имеет знака завершения предложения (., !, ?, …),
    // она НЕ может быть концом предложения
    if !has_terminal_punctuation(&curr.text) {
        return false;
    }

    // Если фраза оканчивается на грамматически незавершенное придаточное («что и этот путь», «потому что»)
    // и следующая фраза начинается вскоре (gap_next < 3.2с) — это НЕ конец предложения!
    if has_incomplete_clause(&curr.text) && gap_next < 3.2 {
        return false;
    }

    // Если следующая реплика начинается со строчной буквы — предложение ещё продолжается!
    if starts_with_lowercase_continuation(next_trimmed) && gap_next < 2.2 {
        return false;
    }

    // Если следующая реплика начинается с союза продолжения мысли или анафоры («И в моих руках...», «Поэтому...», «Так что...») с малой паузой (< 0.85с) —
    // не обрываем мысль до завершения аргумента
    if starts_with_discourse_continuation(next_trimmed) && gap_next < 0.85 && !is_new_scene_opener(next_trimmed) {
        return false;
    }

    // Если текущая строка заканчивается многоточием, а следующая идет сразу следом (< 1.2с) — мысль еще не закончена
    if (curr.text.trim().ends_with("...") || curr.text.trim().ends_with('…')) && gap_next < 1.2 {
        return false;
    }

    true
}

/// Нормализует вектор оценок от LLM так, чтобы его длина строго совпадала с длиной пакета `chunk`
fn normalize_batch_scores(parsed: &[f64], chunk: &[SubtitleFragment]) -> Vec<f64> {
    let fallback = compute_text_scores_advanced(chunk);
    let mut out = Vec::with_capacity(chunk.len());
    for i in 0..chunk.len() {
        if let Some(&val) = parsed.get(i) {
            // Если фраза является явной галлюцинацией Whisper или вокалом песни, принудительно занижаем вес
            if crate::whisper::is_whisper_hallucination(&chunk[i].text)
                || is_opening_or_song_fragment(chunk, i)
            {
                out.push(0.05);
            } else {
                // Смешиваем оценку LLM со строгим локальным анализатором эмоций:
                // если в реплике есть реальное восклицание (`!`) и детерминированный анализатор видит пик кульминации (`fallback[i] >= 0.80`),
                // не позволяем заниженной оценке LLM на коротком выкрике («НАМ КОНЕЦ!») утянуть пик вниз
                let has_exclam = chunk[i].text.contains('!');
                let effective_llm = if has_exclam && fallback[i] >= 0.78 {
                    val.clamp(0.08, 0.96).max(fallback[i] * 0.95)
                } else {
                    val.clamp(0.08, 0.96)
                };
                let raw_blend = effective_llm * 0.62 + fallback[i] * 0.38;
                let contrasted = if !has_exclam && !chunk[i].text.contains('?') {
                    (raw_blend * 0.88).clamp(0.12, 0.56)
                } else if raw_blend < 0.70 {
                    (raw_blend * 0.92).clamp(0.12, 0.68)
                } else {
                    raw_blend.clamp(0.70, 0.96)
                };
                out.push(contrasted);
            }
        } else {
            out.push(fallback[i]);
        }
    }
    out
}

/// Асинхронный запуск анализа через llama-cli с автоматическим поиском и валидацией локальной GGUF-модели
pub async fn analyze_with_llama_cli(
    fragments: &[SubtitleFragment],
    preferred_model_name: Option<&str>,
    app_handle: &AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<Vec<f64>> {
    if fragments.is_empty() {
        return Ok(Vec::new());
    }

    let exact_match = if let Some(pref) = preferred_model_name {
        let is_legacy_default = pref == "models/qwen-1.5b-chat.gguf" || pref == "qwen-1.5b-chat.gguf";
        if is_legacy_default {
            crate::models::resolve_any_llm_model(app_handle, Some(pref))
        } else if pref.ends_with(".gguf") || pref.ends_with(".bin") {
            crate::models::resolve_model_path(app_handle, pref)
        } else {
            None
        }
    } else {
        None
    };

    let mut resolved_model = exact_match
        .clone()
        .or_else(|| crate::models::resolve_any_llm_model(app_handle, preferred_model_name));

    if let Some(ref primary_path) = resolved_model {
        let primary_str = primary_path.to_string_lossy().to_string();
        if is_model_marked_broken(&primary_str) {
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

    let Some(mut model_path) = resolved_model else {
        let err_msg = "Локальная GGUF-модель для llama-cli не найдена на диске (скачайте в Менеджере ИИ-моделей).".to_string();
        crate::logger::emit_log(app_handle, "LLM", "warn", &err_msg);
        return Err(AppError::Pipeline(err_msg));
    };

    let mut model_path_str = model_path.to_string_lossy().to_string();

    if exact_match.is_none() && preferred_model_name.is_some() {
        crate::logger::emit_log(
            app_handle,
            "LLM",
            "warn",
            &format!(
                "Выбранная модель '{}' ещё не скачана в Менеджере ИИ-моделей. Найдена резервная локальная модель: '{}'",
                preferred_model_name.unwrap_or_default(),
                model_path.display()
            ),
        );
    }

    if is_model_marked_broken(&model_path_str) {
        let err_msg = format!("Модель '{}' ранее показала ошибку токенизатора в этой сессии.", model_path.display());
        crate::logger::emit_log(app_handle, "LLM", "warn", &err_msg);
        return Err(AppError::Pipeline(err_msg));
    }

    crate::logger::emit_log(
        app_handle,
        "LLM",
        "info",
        &format!("Запуск локального llama-cli с моделью: '{}' (контекст: 2048, контроль целостности фраз)", model_path.display()),
    );

    let mut all_scores = Vec::with_capacity(fragments.len());
    let batch_size = 25;
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

        let gpu_output_corrupted = match &batch_res {
            Ok(out) => parse_json_array(out, chunk.len()).is_none(),
            Err(_) => true,
        };

        if gpu_output_corrupted && use_gpu && !gpu_succeeded_once {
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

                    if !gpu_succeeded_once && (!use_gpu || consecutive_parse_failures >= 2) && batch_idx + 1 < total_batches {
                        mark_model_broken(&model_path_str);
                        crate::logger::emit_log(
                            app_handle,
                            "LLM",
                            "warn",
                            "Предохранитель LLM: выбранная GGUF-модель выдает некорректные токены. Переключение всех оставшихся пакетов на встроенный NLP-движок анализа полных фраз.",
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

    Ok(all_scores)
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
            &format!("Запуск анализа через llama-cli (модель: '{}')...", config.model_name),
        );
        match analyze_with_llama_cli(fragments, Some(&config.model_name), app_handle, cancel_rx.resubscribe()).await {
            Ok(scores) => return Ok(scores),
            Err(e) => {
                if e.to_string().contains("Отменено") {
                    return Err(e);
                }
                crate::logger::emit_log(
                    app_handle,
                    "LLM",
                    "warn",
                    &format!("llama-cli не может быть запущен ({}). Предохранитель переключает на встроенный NLP-движок.", e),
                );
                return Ok(compute_text_scores_advanced(fragments));
            }
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
                        &format!("Предохранитель Ollama: сервис недоступен ({})! Каскадный переход на llama-cli...", e),
                    );
                    let remaining = &fragments[batch_idx * batch_size..];
                    match analyze_with_llama_cli(remaining, None, app_handle, cancel_rx.resubscribe()).await {
                        Ok(llama_scores) => {
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "info",
                                &format!("✓ Резервный llama-cli успешно обработал {} оставшихся реплик", remaining.len()),
                            );
                            all_scores.extend_from_slice(&llama_scores);
                        }
                        Err(llama_err) => {
                            if llama_err.to_string().contains("Отменено") {
                                return Err(llama_err);
                            }
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "warn",
                                &format!("Сбой резервного llama-cli ({}). Финальное переключение на встроенный NLP-движок.", llama_err),
                            );
                            all_scores.extend_from_slice(&compute_text_scores_advanced(remaining));
                        }
                    }
                    break;
                }
            }
        }
        return Ok(all_scores);
    }

    if config.engine == TextEngine::OpenAi {
        let base_url_display = if config.openai_base_url.trim().is_empty() {
            "https://api.openai.com/v1"
        } else {
            config.openai_base_url.trim()
        };
        let model_display = if config.model_name.trim().is_empty() {
            "gpt-4o-mini"
        } else {
            config.model_name.trim()
        };

        crate::logger::emit_log(
            app_handle,
            "LLM",
            "info",
            &format!(
                "Подключение к OpenAI-совместимому API: {} (модель: {})",
                base_url_display, model_display
            ),
        );

        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();

        let mut all_scores = Vec::with_capacity(fragments.len());
        let batch_size = 24;
        let total_batches = (fragments.len() + batch_size - 1) / batch_size;
        let mut total_failures = 0usize;

        for (batch_idx, chunk) in fragments.chunks(batch_size).enumerate() {
            if cancel_rx.try_recv().is_ok() {
                crate::logger::emit_log(app_handle, "LLM", "warn", "Анализ отменен пользователем.");
                return Err(AppError::Pipeline("Отменено пользователем".into()));
            }

            crate::logger::emit_log(
                app_handle,
                "LLM",
                "info",
                &format!(
                    "Пакет {}/{}: отправка {} реплик в OpenAI API ({})...",
                    batch_idx + 1,
                    total_batches,
                    chunk.len(),
                    model_display
                ),
            );

            let prompt = build_prompt(chunk);
            let mut result_opt = None;
            let mut hard_channel_unavailable = false;
            // Если ранее уже был сбой на этом API, не тратим время на повторные попытки для каждого пакета
            let max_attempts = if total_failures == 0 { 2 } else { 1 };

            for attempt in 1..=max_attempts {
                match run_openai(&client, config, &prompt, app_handle).await {
                    Ok(res) => {
                        if parse_json_array(&res, chunk.len()).is_some() {
                            result_opt = Some(res);
                            break;
                        } else if attempt < max_attempts {
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "warn",
                                &format!(
                                    "Пакет {}/{}: неполный ответ OpenAI API ({} симв.), повторный запрос через 1 сек...",
                                    batch_idx + 1,
                                    total_batches,
                                    res.len()
                                ),
                            );
                            tokio::time::sleep(Duration::from_millis(1000)).await;
                        } else {
                            result_opt = Some(res);
                        }
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str.contains("502")
                            || err_str.contains("503")
                            || err_str.contains("504")
                            || err_str.contains("429")
                            || err_str.contains("недоступен")
                        {
                            hard_channel_unavailable = true;
                            break;
                        }
                        if attempt < max_attempts {
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "warn",
                                &format!(
                                    "Пакет {}/{}: задержка/сбой сети ({}), повторный запрос через 1.5 сек...",
                                    batch_idx + 1,
                                    total_batches,
                                    e
                                ),
                            );
                            tokio::time::sleep(Duration::from_millis(1500)).await;
                        } else {
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "warn",
                                &format!(
                                    "Пакет {}/{}: запрос к OpenAI API не удался ({}).",
                                    batch_idx + 1,
                                    total_batches,
                                    e
                                ),
                            );
                        }
                    }
                }
            }

            let parsed_scores = result_opt.as_deref().and_then(|res| parse_json_array(res, chunk.len()));
            if let Some(ref batch_scores) = parsed_scores {
                crate::logger::emit_log(
                    app_handle,
                    "LLM",
                    "info",
                    &format!(
                        "✓ Пакет {}/{}: получено {} валидных оценок от OpenAI API",
                        batch_idx + 1,
                        total_batches,
                        batch_scores.len()
                    ),
                );
                all_scores.extend_from_slice(&normalize_batch_scores(batch_scores, chunk));
            } else {
                total_failures += 1;
                // При недоступности бесплатного канала (502/503/429) или при 2-м сбое за сессию
                // немедленно передаем ТЕКУЩИЙ пакет и все оставшиеся пакеты в быстрый локальный llama-cli (GPU)!
                if hard_channel_unavailable || total_failures >= 2 || total_batches == 1 {
                    crate::logger::emit_log(
                        app_handle,
                        "LLM",
                        "warn",
                        &format!(
                            "Предохранитель OpenAI API: внешний канал нестабилен или перегружен (пакет {}/{}). Мгновенный каскадный переход на локальный llama-cli (GPU) без потери пакетов...",
                            batch_idx + 1,
                            total_batches
                        ),
                    );
                    let remaining = &fragments[batch_idx * batch_size..];
                    match analyze_with_llama_cli(remaining, None, app_handle, cancel_rx.resubscribe()).await {
                        Ok(llama_scores) => {
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "info",
                                &format!(
                                    "✓ Резервный llama-cli успешно обработал {} реплик (начиная с пакета {}/{})",
                                    remaining.len(),
                                    batch_idx + 1,
                                    total_batches
                                ),
                            );
                            all_scores.extend_from_slice(&llama_scores);
                        }
                        Err(llama_err) => {
                            if llama_err.to_string().contains("Отменено") {
                                return Err(llama_err);
                            }
                            crate::logger::emit_log(
                                app_handle,
                                "LLM",
                                "warn",
                                &format!("Сбой резервного llama-cli ({}). Финальное переключение на встроенный NLP-движок.", llama_err),
                            );
                            all_scores.extend_from_slice(&compute_text_scores_advanced(remaining));
                        }
                    }
                    break;
                } else {
                    crate::logger::emit_log(
                        app_handle,
                        "LLM",
                        "warn",
                        &format!(
                            "Пакет {}/{}: сбой ответа OpenAI API, для пакета применен локальный семантический анализатор.",
                            batch_idx + 1,
                            total_batches
                        ),
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
        let clean_txt = frag.text.replace(['\n', '\r', '"'], " ");
        let start_tag = if is_sentence_start(fragments, i) { "[START]" } else { "[CONT]" };
        let end_tag = if is_sentence_end(fragments, i) { "[END]" } else { "[INCOMPLETE]" };
        lines.push(format!("{}. {} {} \"{}\"", i + 1, start_tag, end_tag, clean_txt.trim()));
    }
    let input_list = lines.join("\n");
    format!(
        "Ты строгий профессиональный режиссер монтажа вертикальных роликов (Shorts, Reels, TikTok).\n\
Оцени эмоциональный накал каждой из {} реплик от 0.0 до 1.0 строго по шкале (НЕ завышай оценки спокойным фразам!):\n\
1. 0.15-0.42: обрывки ([CONT]/[INCOMPLETE]), спокойные доклады, бытовые реплики, вежливые разговоры.\n\
2. 0.45-0.66: обычные сюжетные диалоги, обсуждение планов, стандартные вопросы без крика.\n\
3. 0.70-0.84: напряженные споры, угрозы, драматические повороты, жесткие боевые приказы.\n\
4. 0.88-0.96: ИСКЛЮЧИТЕЛЬНЫЙ ЭМОЦИОНАЛЬНЫЙ ПИК / КУЛЬМИНАЦИЯ (яростный крик, шок, отчаяние, смертельная схватка — не более 1-2 реплик на пакет!).\n\
Выведи ТОЛЬКО валидный JSON-массив ровно из {} чисел в диапазоне 0.0-1.0 без текста и комментариев.\n\
Пример формата: [0.42, 0.89]\n\n\
Реплики:\n{}\n\n\
JSON:[",
        fragments.len(),
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

fn build_openai_chat_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return "https://api.openai.com/v1/chat/completions".to_string();
    }
    if trimmed.ends_with("/chat/completions") {
        return trimmed.to_string();
    }
    if trimmed.ends_with("/v1")
        || trimmed.ends_with("/v1beta")
        || trimmed.ends_with("/v4")
        || trimmed.ends_with("/openai")
    {
        return format!("{}/chat/completions", trimmed);
    }
    format!("{}/v1/chat/completions", trimmed)
}

async fn run_openai(
    client: &Client,
    config: &TextAnalysisConfig,
    prompt: &str,
    app_handle: &AppHandle,
) -> AppResult<String> {
    let url = build_openai_chat_url(&config.openai_base_url);
    let model = if config.model_name.trim().is_empty() || config.model_name.ends_with(".gguf") {
        "gpt-4o-mini".to_string()
    } else {
        config.model_name.trim().to_string()
    };

    let req_body = OpenAiChatRequest {
        model: model.clone(),
        messages: vec![
            OpenAiMessage {
                role: "system".to_string(),
                content: "Ты профессиональный ИИ-режиссер монтажа вертикальных роликов (Shorts, Reels, TikTok). Возвращай ТОЛЬКО валидный JSON-массив чисел от 0.0 до 1.0 без пояснений.".to_string(),
            },
            OpenAiMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            },
        ],
        temperature: 0.1,
        stream: false,
    };

    crate::logger::emit_log(
        app_handle,
        "LLM",
        "debug",
        &format!("OpenAI API POST {} (модель: {}, длина промпта: {} симв.)", url, model, prompt.len()),
    );

    let mut req = client.post(&url).json(&req_body);
    let api_key = config.openai_api_key.trim();
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }

    let resp = req.send().await.map_err(|e| {
        let err_msg = format!("Ошибка сетевого запроса к OpenAI API ({}): {}", url, e);
        crate::logger::emit_log(app_handle, "LLM", "error", &err_msg);
        AppError::Pipeline(err_msg)
    })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        let snippet: String = err_body.chars().take(180).collect();
        let err_msg = format!("HTTP {} от OpenAI API: {}", status, snippet);
        crate::logger::emit_log(app_handle, "LLM", "error", &err_msg);
        return Err(AppError::Pipeline(err_msg));
    }

    let parsed: OpenAiChatResponse = resp.json().await.map_err(|e| {
        let err_msg = format!("Не удалось распарсить JSON-ответ от OpenAI API: {}", e);
        crate::logger::emit_log(app_handle, "LLM", "error", &err_msg);
        AppError::Pipeline(err_msg)
    })?;

    let content = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .unwrap_or_default();

    crate::logger::emit_log(
        app_handle,
        "LLM",
        "debug",
        &format!("OpenAI API ответ получен (длина: {} символов)", content.len()),
    );

    Ok(content)
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

    // Динамический лимит токенов пропорционально размеру пакета + стоп-токен `]`
    // для мгновенного завершения генерации сразу после закрытия JSON-массива (с запасом для 25 элементов)
    let max_tokens = (expected_count * 12 + 32).clamp(64, 384).to_string();

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
        "-r", "]",
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

    // Если используется reasoning-модель (DeepSeek-R1, QwQ и т.д.), отсекаем блок размышлений <think>...</think>
    let after_think = if let Some(think_end) = raw_trimmed.rfind("</think>") {
        raw_trimmed[think_end + "</think>".len()..].trim()
    } else {
        raw_trimmed
    };

    let cleaned = after_think
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
        Some(
            nums.into_iter()
                .take(expected_len)
                .map(|x| x.clamp(0.05, 0.96))
                .collect(),
        )
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

    // Определяем, является ли весь файл субтитров преимущественно русскоязычным (>= 40% реплик содержат кириллицу).
    // Это защищает от ложного срабатывания, если видео изначально на английском языке!
    let cyrillic_lines = fragments
        .iter()
        .filter(|frag| frag.text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)))
        .count();
    let is_cyrillic_track = !fragments.is_empty() && cyrillic_lines * 10 >= fragments.len() * 4;

    // 1. В русскоязычном видео строки, содержащие латинские слова («We will like to start!», «Сильвестка не азера, Your move!») — это вокал песни/опенинга
    if is_cyrillic_track {
        let has_cyrillic = text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
        let latin_letters = text.chars().filter(|c| c.is_ascii_alphabetic()).count();
        if !has_cyrillic && latin_letters >= 3 {
            return true;
        }
        // Смешанная кириллица + английские слова в одной реплике (например «Сильвестка не азера, Your move!»)
        let has_english_word = text
            .split(|c: char| !c.is_ascii_alphabetic())
            .any(|w| w.len() >= 3 && w.chars().any(|ch| matches!(ch.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y')));
        if has_english_word && latin_letters >= 4 {
            return true;
        }
    }

    // 2. Детектор изолированного вокального «острова» опенинга/эндинга (длительностью до 95 секунд),
    // который с обеих сторон ограничен длинными музыкальными паузами (>= 14с, где были вырезаны теги [Звучит музыка] и строки с ♪)
    let mut cluster_start = idx;
    while cluster_start > 0 && (fragments[cluster_start].start - fragments[cluster_start - 1].end) < 13.5 {
        cluster_start -= 1;
        if idx - cluster_start > 15 {
            break;
        }
    }
    let mut cluster_end = idx;
    while cluster_end + 1 < fragments.len() && (fragments[cluster_end + 1].start - fragments[cluster_end].end) < 13.5 {
        cluster_end += 1;
        if cluster_end - idx > 15 {
            break;
        }
    }
    let gap_before_cluster = if cluster_start > 0 {
        (fragments[cluster_start].start - fragments[cluster_start - 1].end).max(0.0)
    } else {
        0.0
    };
    let gap_after_cluster = if cluster_end + 1 < fragments.len() {
        (fragments[cluster_end + 1].start - fragments[cluster_end].end).max(0.0)
    } else {
        20.0
    };
    let cluster_span = (fragments[cluster_end].end - fragments[cluster_start].start).max(0.0);
    let cluster_count = cluster_end - cluster_start + 1;
    if gap_before_cluster >= 14.0
        && gap_after_cluster >= 14.0
        && cluster_span <= 95.0
        && cluster_count <= 14
        && (f.start <= 260.0 || f.start >= 1200.0)
    {
        return true;
    }

    // 3. Проверяем, не находится ли реплика внутри 30-секундного вокального блока опенинга/эндинга,
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
        let has_latin_nearby = is_cyrillic_track
            && (win_start..=win_end).any(|k| {
                let t = &fragments[k].text;
                t.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 3
            });

        if int_dur_neighbors >= 4 && (has_music_gap_nearby || has_latin_nearby) {
            return true;
        }
    }

    // 3. Детектор фонетического мусора японского/иностранного эндинга в последних 90 секундах 20+-минутной серии:
    // когда после удаления тегов (Звучит музыка) остаются одиночные короткие псевдофразы, окруженные длинными музыкальными паузами (>= 6.5с)
    let total_track_end = fragments.last().map(|last| last.end).unwrap_or(0.0);
    if total_track_end >= 1200.0 && f.start >= total_track_end - 95.0 {
        let gap_before = if idx > 0 {
            (f.start - fragments[idx - 1].end).max(0.0)
        } else {
            0.0
        };
        let gap_after = if idx + 1 < fragments.len() {
            (fragments[idx + 1].start - f.end).max(0.0)
        } else {
            10.0
        };
        let lower_t = text.to_lowercase();
        let comma_count = lower_t.matches(',').count();
        if gap_before >= 6.5 && gap_after >= 6.5 && (comma_count >= 3 || text.split_whitespace().count() <= 4) {
            return true;
        }
    }

    false
}

/// Продвинутый локальный семантический анализатор диалогов, эмоциональных всплесков и целостности фраз
/// со строгой калибровкой шкалы эмоций (0.20 .. 0.95):
/// - Спокойные доклады и бытовые разговоры без восклицаний: 0.25 – 0.52
/// - Обычные тактические обсуждения и вопросы: 0.54 – 0.68
/// - Эмоциональные споры, приказы и драматические повороты: 0.70 – 0.84
/// - Исключительная кульминация (крик, ярость, шок, `?!` / `!!` + маркеры экстрима): 0.86 – 0.95
pub fn compute_text_scores_advanced(fragments: &[SubtitleFragment]) -> Vec<f64> {
    if fragments.is_empty() {
        return Vec::new();
    }

    // 1. Словник ЭКСТРЕМАЛЬНОГО эмоционального накала и кульминации
    let extreme_climax_keywords = [
        "ха-ха", "ахах", "haha", "что?!", "не может быть", "невозможно", "немедленно",
        "закрой свой рот", "прекрати", "довольно", "замолчи", "бей", "беги", "боже",
        "шок", "ужас", "смерт", "крик", "огонь", "уничтож", "кров", "взрыв", "лжешь",
        "врёшь", "поклялся", "никогда!", "ненавижу", "убью", "умри", "умрем", "умрём",
        "дьявол", "монстр", "кайдзю", "берегись", "держись", "черт", "чёрт", "проклят",
        "безум", "сдохн", "разнес", "сотру", "в атаку", "штурм", "прорыв", "прорв",
        "нам конец", "конец!", "спасены", "не умрем", "не умрём", "погибн", "подкреплен",
        "главных сил", "существо икс", "костьми лягу", "настойчивость", "недооценивать",
        "пощады", "в аду", "в бой", "стреляй", "тревога", "не подведи", "прикрой",
        "испытания", "играючи", "impossible", "destroy",
    ];

    // 2. Словник умеренного сюжетного/тактического интереса (не должен сам по себе задирать оценку до 0.9!)
    let moderate_plot_keywords = [
        "вперёд", "вперед", "стоп", "быстрее", "постой", "подожди", "смотри", "победа", "атака",
        "удар", "сила", "войн", "приказ", "враг", "преступлен", "спас", "биться", "секрет",
        "тайна", "правда", "прости", "обещаю", "люблю", "защищ", "опасно", "ловушк",
        "план", "победил", "проиграл", "герой", "магия", "способност", "сердц", "душа",
        "судьб", "выбор", "шанс", "капитан", "командир", "генерал", "лейтенант", "оружи",
        "батальон", "штаб", "импери", "сонар", "турбин", "линкор", "авианосец", "магов",
    ];

    let hook_starters = [
        "слушай", "знаешь", "смотри", "погоди", "неужели", "почему", "зачем",
        "как же", "на самом деле", "представь", "эй", "послушай", "внимание",
        "осторожно", "клянусь", "довольно", "отлично", "господи",
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

            let raw_trim = f.text.trim();
            let txt = raw_trim.to_lowercase().replace('ё', "е");
            let word_count = txt.split_whitespace().count();

            if word_count == 0 {
                return 0.05;
            }

            // Сверхкороткие междометия без восклицания не должны сами по себе становиться ядром клипа
            if word_count <= 1 && !txt.contains('!') && !txt.contains('?') {
                return 0.15;
            }
            if word_count <= 2 && !txt.contains('!') && !txt.contains('?') {
                return 0.20;
            }

            // Спокойная базовая линия (без искусственного завышения)
            let mut score: f64 = 0.22;

            // 1. Эмоциональная пунктуация и КАПС — главные индикаторы реального голосового накала в субтитрах
            let exclam_count = raw_trim.chars().filter(|&c| c == '!').count();
            let has_exclam = exclam_count > 0;
            let has_double_punct =
                raw_trim.contains("!!") || raw_trim.contains("?!") || raw_trim.contains("!?");

            // Проверяем, содержит ли реплика слова, написанные КАПСОМ (крик на надрыве, например «НАМ КОНЕЦ!»)
            let has_all_caps_scream = raw_trim.split_whitespace().any(|w| {
                let alphas: Vec<char> = w.chars().filter(|c| c.is_alphabetic()).collect();
                alphas.len() >= 3 && alphas.iter().all(|c| c.is_uppercase())
            });

            if has_exclam {
                score += 0.15;
                if exclam_count >= 2 {
                    score += 0.10;
                }
            }
            if has_double_punct {
                score += 0.16;
            }
            if has_all_caps_scream && (has_exclam || raw_trim.contains('?')) {
                score += 0.22;
            }
            if txt.contains('?') && !has_double_punct {
                score += 0.04;
            }
            if txt.contains('…') || txt.contains("...") {
                score += 0.02;
            }

            // 2. Экстремальные слова кульминации vs обычные сюжетные термины
            let mut extreme_hits = 0usize;
            for kw in &extreme_climax_keywords {
                if txt.contains(kw) {
                    extreme_hits += 1;
                    if extreme_hits >= 3 {
                        break;
                    }
                }
            }

            let mut mod_hits = 0usize;
            for kw in &moderate_plot_keywords {
                if txt.contains(kw) {
                    mod_hits += 1;
                    if mod_hits >= 2 {
                        break;
                    }
                }
            }

            score += (extreme_hits as f64) * 0.16;
            score += (mod_hits as f64) * 0.04;

            // 3. Хук-маркеры и целостность границ фразы
            if is_sentence_start(fragments, idx) {
                score += 0.03;
                if hook_starters.iter().any(|h| txt.starts_with(h) || txt.contains(h)) {
                    score += 0.04;
                }
            } else {
                // Середина незаконченного предложения
                score -= 0.08;
            }

            if is_sentence_end(fragments, idx) {
                score += 0.03;
            }

            // 4. Темп и плотность живой речи
            let duration = (f.end - f.start).max(0.4);
            let speech_rate = (word_count as f64) / duration;
            if speech_rate >= 2.4 && speech_rate <= 5.5 {
                score += ((speech_rate - 2.2) * 0.022).clamp(0.01, 0.05);
            }

            // 5. Насыщенность реплики по числу слов
            if word_count >= 6 {
                score += ((word_count as f64 - 5.0) * 0.006).min(0.04);
            }

            // Спокойные повествовательные фразы без `!` и без экстремальных триггеров не могут превышать 0.55
            if !has_exclam && extreme_hits == 0 && !has_double_punct && !has_all_caps_scream {
                score = score.min(0.55);
            }

            score.clamp(0.12, 0.96)
        })
        .collect();

    // Шаг 2. Группировка реплик в полные предложения и передача импульса к НАЧАЛУ предложения
    // без искусственного завышения спокойных предложений
    let mut i = 0;
    while i < fragments.len() {
        let sent_start = i;
        let mut sent_end = i;
        while sent_end + 1 < fragments.len() && !is_sentence_end(fragments, sent_end) && (sent_end - sent_start) < 6 {
            sent_end += 1;
        }

        if sent_end > sent_start {
            let max_in_sentence = raw_scores[sent_start..=sent_end]
                .iter()
                .copied()
                .fold(0.0_f64, f64::max);

            // Передаем импульс к первой части предложения только если в предложении действительно есть эмоция
            let boost = if max_in_sentence >= 0.78 { 0.02 } else { 0.0 };
            raw_scores[sent_start] = (raw_scores[sent_start] * 0.40 + max_in_sentence * 0.60 + boost).clamp(0.16, 0.95);
        }

        i = sent_end + 1;
    }

    // Шаг 3. Кластерный импульс для сцен с серией восклицаний подряд (боевая кульминация / накал страстей):
    // если вокруг восклицательной реплики в радиусе ±6.5с есть еще >= 2 восклицательных реплик,
    // усиливаем эмоциональное ядро кластера, чтобы настоящие кульминации достигали 0.86–0.95
    let snapshot_scores = raw_scores.clone();
    for idx in 0..fragments.len() {
        if !fragments[idx].text.contains('!') || snapshot_scores[idx] < 0.48 {
            continue;
        }
        let mut exclam_neighbors = 0usize;
        let mut max_neighbor_score = 0.0_f64;
        let start_scan = idx.saturating_sub(3);
        let end_scan = (idx + 3).min(fragments.len().saturating_sub(1));
        for n_idx in start_scan..=end_scan {
            if n_idx == idx {
                continue;
            }
            let time_dist = if fragments[n_idx].start > fragments[idx].end {
                fragments[n_idx].start - fragments[idx].end
            } else if fragments[idx].start > fragments[n_idx].end {
                fragments[idx].start - fragments[n_idx].end
            } else {
                0.0
            };
            if time_dist <= 6.5 && fragments[n_idx].text.contains('!') && snapshot_scores[n_idx] >= 0.45 {
                exclam_neighbors += 1;
                if snapshot_scores[n_idx] > max_neighbor_score {
                    max_neighbor_score = snapshot_scores[n_idx];
                }
            }
        }
        if exclam_neighbors >= 2 {
            let cluster_boost = (exclam_neighbors.min(4) as f64) * 0.055
                + if max_neighbor_score >= 0.75 { 0.04 } else { 0.0 };
            raw_scores[idx] = (snapshot_scores[idx] + cluster_boost).clamp(0.16, 0.95);
        }
    }

    raw_scores
}

pub fn compute_text_scores_fallback(fragments: &[SubtitleFragment]) -> Vec<f64> {
    compute_text_scores_advanced(fragments)
}
