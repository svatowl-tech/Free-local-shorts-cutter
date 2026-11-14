// src-tauri/src/text_analyzer.rs

use crate::error::{AppError, AppResult};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tokio::sync::broadcast::Receiver;

fn is_model_marked_broken(path_str: &str) -> bool {
    crate::models::is_model_marked_broken(path_str)
}

fn mark_model_broken(path_str: &str) {
    crate::models::mark_model_broken(path_str);
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

    // Одиночные изолированные придаточные условия / уступки («Если ты сказал правду.», «Хотя это не точно.», «Пока мы не проверим.»)
    // без главного предложения — грамматически не закончены
    if (clean.starts_with("если ") || clean.starts_with("хотя ") || clean.starts_with("пока ") || clean.starts_with("раз уж "))
        && tokens.len() <= 7
    {
        return true;
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

    // Если текущая строка начинается со строчной (маленькой) буквы — это середина незаконченного предложения!
    // Предложение не может начинаться с маленькой буквы или оторванного хвоста слова/фразы
    if starts_with_lowercase_continuation(curr_trimmed) {
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

    // Никогда не начинаем клип со строчной буквы или продолжения предложения
    if starts_with_lowercase_continuation(&fragments[idx].text) {
        return true;
    }

    let clean = fragments[idx]
        .text
        .trim()
        .trim_start_matches(['-', '—', ' '])
        .trim_end_matches(['.', '!', '?', '…', '"', '»', ' '])
        .to_lowercase()
        .replace('ё', "е");

    // Одиночные инфинитивы или слова-вопросы без субъекта («поступить?», «поступить», «делать?», «сказать?»)
    if clean.chars().filter(|c| c.is_alphabetic()).count() <= 12
        && matches!(
            clean.as_str(),
            "поступить" | "поступить?" | "делать" | "делать?" | "сказать" | "сказать?" | "идти" | "идти?" | "быть" | "быть?"
        )
    {
        return true;
    }

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
        "вау",
        "срочно",
        "ой",
        "ай",
        "эй",
        "блин",
        "слышь",
        "слушай",
        "хм",
        "э-э",
        "о-о",
        "ёлки-палки",
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
/// (незакрытые вопросы `?`, короткие поддакивания, переспросы, реплики-подводки перед ответом или дубли/самоисправления актера озвучки)
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

    // 0.5. Самоисправления и мета-комментарии актера дубляжа / диктора при запоротом дубле
    // («не понравилось», «в тайминг не попал», «все хуйня», «перепишем», «давай еще раз», «плохо получилось»)
    if clean.contains("не понравилось")
        || clean.contains("в тайминг не")
        || clean.contains("не попал")
        || clean.contains("перепишем")
        || clean.contains("переписать")
        || clean.contains("перепишу")
        || clean.contains("давай еще раз")
        || clean.contains("давай еще")
        || clean.contains("попробую еще раз")
        || clean.contains("все хуйня")
        || clean.contains("все херня")
        || clean.contains("все фигня")
        || clean.contains("плохой дубль")
        || clean.contains("звучит натужно")
        || clean.ends_with("не то")
        || clean.ends_with("не тот вайп")
        || clean.ends_with("давай заново")
        || clean.ends_with("давай сначала")
    {
        return true;
    }

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
        "это срочно", "странное дело", "понимаете",
        "вы уверены", "ты уверен",
        "простите, вы хотите сказать", "простите",
        "не нужно", "ненужно", "я понял", "ясно", "понял",
        "сами ведь знаете", "вы же знаете", "ты же знаешь", "сама же знаешь",
        "да, само собой", "само собой",
        "а это еще что", "что это", "кто здесь",
        "ну-ка, ну-ка", "ну-ка ну-ка", "ну-ка", "ну ка", "ну-ка.", "ну-ка, ну-ка.",
        "покажи им", "покажи", "показывай", "ой, не стоит", "не стоит",
        "слышишь", "слушай", "глянь", "смотри",
    ];
    if weak_backchannels.contains(&clean.as_str()) {
        return true;
    }

    // 3. Катафорические фразы-подводки и двухшаговые зачины, которые анонсируют следующую реплику
    let setup_phrases = [
        "а зовут его",
        "зовут его",
        "вывод из этого такой",
        "вывод такой",
        "добавлю еще одну деталь",
        "добавлю еще",
        "более того",
        "кое-что уточню",
        "слушай внимательно",
        "слушайте внимательно",
        "тема сегодняшнего",
        "проще говоря",
        "сначала они ",
        "сначала мы ",
        "сначала я ",
        "а потом еще и ",
        "и пока я ",
        "а раз так",
        "меня об этом не предупреждали",
        "а тут я заметил",
        "давайте уже перейдем к сути",
        "давайте перейдем",
        "обратите внимание",
        "дело в том",
        "короче говоря",
        "в этом видео",
        "в этом ролике",
        "в сегодняшнем видео",
        "сегодня мы поговорим",
        "сегодня я расскажу",
        "некоторые окружающие вас предметы",
        "окружающие вас предметы",
        "можно брать или перемещать",
        "нажмите клавишу",
        "нажмите кнопку",
        "для перемещения",
        "для взаимодействия",
        "чтобы сохраниться",
        "цель задания",
        "миссия провалена",
        "задание обновлено",
        "подсказка:",
    ];
    if setup_phrases.iter().any(|p| clean.starts_with(p) || clean.ends_with(p)) {
        return true;
    }

    // 4. Приветствия, обращения при встрече и вводные вопросы к новой теме
    if clean.starts_with("прости за ожидан")
        || clean.starts_with("значит, ты и есть")
        || clean.starts_with("заходи, я представл")
        || clean.starts_with("всем привет")
        || clean.starts_with("привет всем")
        || clean.starts_with("добро пожаловать")
        || clean.starts_with("давайте начнем")
        || clean.contains("вы же понимаете, что")
        || clean.contains("как считаете, на какие уступки")
        || clean.contains("могу я быть с вами честен")
        || clean.contains("я бы тоже хотел услышать ваше мнение")
    {
        return true;
    }

    if word_count <= 3
        && !raw_trimmed.ends_with('!')
        && [
            "господин",
            "госпожа",
            "товарищ",
            "коллеги",
            "друзья",
            "ребята",
            "здравствуйте",
            "добрый день",
            "доброе утро",
            "добрый вечер",
            "привет",
            "хай",
        ]
        .iter()
        .any(|greeting| clean.starts_with(greeting))
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
                "бой" | "ай" | "ой" | "эй" | "стой" | "стоять" | "что" | "а" | "ох" | "ах" | "ух" | "хм" | "ну" | "погоди-ка" | "погоди"
            ))
    {
        return true;
    }

    // Чистый вопрос в конце клипа (без `?!`/`!?`, например «А это ещё что?», «Ли, так кого надо проверить?», «Что ты думаешь?»)
    let is_pure_question_cue = raw_trimmed.ends_with('?')
        && !raw_trimmed.ends_with("?!")
        && !raw_trimmed.ends_with("!?");

    // Обращенный вопрос собеседнику или открытый вопрос в диалоге:
    // («что ты думаешь?», «что вы думаете?», «как думаешь?», «что скажешь?», «ты уверен?», «почему?», «куда?», «где?»)
    let is_interlocutor_question = is_pure_question_cue && (
        clean.ends_with("что ты думаешь")
            || clean.ends_with("что вы думаете")
            || clean.ends_with("как ты думаешь")
            || clean.ends_with("как думаешь")
            || clean.ends_with("что скажешь")
            || clean.ends_with("что скажете")
            || clean.ends_with("ты уверен")
            || clean.ends_with("ты уверена")
            || clean.ends_with("вы уверены")
            || clean.ends_with("понял")
            || clean.ends_with("понятно")
            || clean.ends_with("согласен")
            || clean.ends_with("согласна")
            || clean.ends_with("слышишь")
            || clean.ends_with("видишь")
            || clean.ends_with("помнишь")
            || clean.ends_with("правда")
            || clean.ends_with("не так ли")
            || clean.contains("что ты думаешь")
            || clean.contains("что скажешь")
            || clean.contains("что вы думаете")
            || clean.contains("как вы считаете")
            || clean.contains("как ты считаешь")
    );

    // Если это прямой вопрос собеседнику или короткий вопрос <= 5 слов — он не может быть самостоятельным завершением
    if is_interlocutor_question || (is_pure_question_cue && word_count <= 5) {
        return true;
    }

    // 5. Анализ связки со СЛЕДУЮЩЕЙ репликой (`idx + 1`):
    //    - Вопрос (`?`), за которым в пределах 8.5с идет ответ собеседника или продолжение речи («Это там... Рейнский дьявол?», «Что вообще происходит?», «Что ты думаешь?») —
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
            // Любой вопрос (кроме риторического финала перед длинной паузой > 8.5с в конце видео) требует ответа!
            if is_pure_question_cue && gap_next <= 8.5 {
                return true;
            }
            // Эмоциональный вопрос с `?!` / `!?`, если ответ/продолжение звучит вскоре (`gap_next <= 2.2с`)
            if (raw_trimmed.ends_with("?!") || raw_trimmed.ends_with("!?")) && gap_next <= 2.2 {
                return true;
            }
            if word_count == 1 && gap_next < 2.5 {
                return true;
            }
            // Короткая 1–3 словная фраза с точкой («Знакомая атака.», «Странное дело.», «Сами ведь знаете.»), за которой в пределах 2.5с идет реакция
            if word_count <= 3 && !raw_trimmed.contains('!') && gap_next <= 2.5 {
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
                let has_question = chunk[i].text.contains('?');
                let effective_llm = if has_exclam && fallback[i] >= 0.75 {
                    val.clamp(0.12, 0.98).max(fallback[i] * 0.95)
                } else {
                    val.clamp(0.12, 0.98)
                };
                let raw_blend = effective_llm * 0.65 + fallback[i] * 0.35;
                let contrasted = if has_exclam {
                    (raw_blend * 1.10).clamp(0.35, 0.98)
                } else if has_question {
                    (raw_blend * 1.05).clamp(0.30, 0.92)
                } else {
                    raw_blend.clamp(0.20, 0.88)
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
        if !crate::models::is_gguf_compatible_with_llama_cli(primary_path) || is_model_marked_broken(&primary_str) {
            mark_model_broken(&primary_str);
            if let Some(alt_path) = crate::models::resolve_any_llm_model(app_handle, None) {
                let alt_str = alt_path.to_string_lossy().to_string();
                if !is_model_marked_broken(&alt_str) && alt_str != primary_str {
                    crate::logger::emit_log(
                        app_handle,
                        "LLM",
                        "warn",
                        &format!(
                            "Модель '{}' не поддерживается текущей сборкой llama-cli (или повреждена). Автоматическое переключение на совместимую локальную GGUF-модель '{}'",
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
        if let Some(alt_path) = crate::models::resolve_any_llm_model(app_handle, None) {
            let alt_str = alt_path.to_string_lossy().to_string();
            if !is_model_marked_broken(&alt_str) {
                model_path = alt_path;
                model_path_str = alt_str;
            } else {
                let err_msg = format!("Модель '{}' несовместима с llama-cli.", model_path.display());
                crate::logger::emit_log(app_handle, "LLM", "warn", &err_msg);
                return Err(AppError::Pipeline(err_msg));
            }
        } else {
            let err_msg = format!("Модель '{}' ранее показала ошибку загрузки в этой сессии.", model_path.display());
            crate::logger::emit_log(app_handle, "LLM", "warn", &err_msg);
            return Err(AppError::Pipeline(err_msg));
        }
    }

    crate::logger::emit_log(
        app_handle,
        "LLM",
        "info",
        &format!("Запуск локального llama-cli с моделью: '{}' (контекст: 4096, контроль целостности фраз)", model_path.display()),
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

        let (sys_prompt, user_prompt) = build_stage1_prompt_parts(chunk);
        let mut prompt = format_prompt_for_gguf_model(&model_path_str, &sys_prompt, &user_prompt, "JSON:[");
        let mut batch_res = run_llama_cli(
            app_handle,
            &model_path_str,
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
            // 1. Сначала пробуем выбранную пользователем модель на чистом CPU (исключаем сбой CUDA/FP16), если она не сломана архитектурно
            if !is_model_marked_broken(&model_path_str) {
                crate::logger::emit_log(
                    app_handle,
                    "LLM",
                    "warn",
                    &format!(
                        "Модель '{}' не выдала валидный JSON на GPU в первом пакете. Повтор на чистом CPU с той же моделью...",
                        model_path.display()
                    ),
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

            let still_corrupted_primary = match &batch_res {
                Ok(out) => parse_json_array(out, chunk.len()).is_none(),
                Err(_) => true,
            };

            // 2. Только если выбранная модель не смогла даже на CPU (или не загрузилась), переключаемся на резервную модель
            if still_corrupted_primary {
                mark_model_broken(&model_path_str);
                if let Some(alt_path) = crate::models::resolve_any_llm_model(app_handle, None) {
                    let alt_str = alt_path.to_string_lossy().to_string();
                    if !is_model_marked_broken(&alt_str) && alt_str != model_path_str {
                        crate::logger::emit_log(
                            app_handle,
                            "LLM",
                            "warn",
                            &format!(
                                "Модель '{}' выдала невалидный ответ. Автоматическое переключение на резервную GGUF-модель '{}'...",
                                model_path.display(),
                                alt_path.display()
                            ),
                        );
                        model_path = alt_path;
                        model_path_str = alt_str;
                        use_gpu = true;
                        prompt = format_prompt_for_gguf_model(&model_path_str, &sys_prompt, &user_prompt, "JSON:[");
                        batch_res = run_llama_cli(
                            app_handle,
                            &model_path_str,
                            &prompt,
                            chunk.len(),
                            true,
                            cancel_rx.resubscribe(),
                        )
                        .await;

                        let alt_gpu_bad = match &batch_res {
                            Ok(out) => parse_json_array(out, chunk.len()).is_none(),
                            Err(_) => true,
                        };
                        if alt_gpu_bad {
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
                }
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

/// Определяет, является ли реплика «водой», словом-паразитом, хезитацией или технической фразой без сюжетной ценности
pub fn is_filler_or_water_fragment(text: &str) -> bool {
    let lower = text
        .trim()
        .to_lowercase()
        .replace('ё', "е")
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string();
    if lower.is_empty() {
        return true;
    }

    let words: Vec<&str> = lower.split_whitespace().collect();
    if words.is_empty() {
        return true;
    }

    const PURE_FILLERS: &[&str] = &[
        "э", "ээ", "эээ", "м", "мм", "ммм", "а", "аа", "ну", "типа", "короче",
        "ну короче", "как бы", "так сказать", "в общем", "в общем то", "собственно",
        "значить", "значит так", "ладно", "окей", "ага", "угу", "понятно", "ясно",
        "так", "ну вот", "вот так", "такие дела", "раз раз", "проверка звука",
        "меня слышно", "слышно меня",
    ];
    if PURE_FILLERS.contains(&lower.as_str()) {
        return true;
    }

    if words.len() <= 5 {
        let filler_tokens = [
            "ээ", "эээ", "мм", "ммм", "ну", "короче", "типа", "как", "бы", "вот", "это", "самое",
        ];
        let f_cnt = words.iter().filter(|w| filler_tokens.contains(w)).count();
        if f_cnt * 2 >= words.len() && !text.contains('!') && !text.contains('?') {
            return true;
        }
    }

    false
}

/// Вычисляет текстовые метрики структуры шортса / хайлайта для окна `[start_idx..=end_idx]`:
/// Возвращает `(hook_text_score, punchline_text_score, filler_ratio)` в диапазоне `0.0 .. 1.0`.
pub fn compute_hook_and_punchline_text_metrics(
    fragments: &[SubtitleFragment],
    start_idx: usize,
    end_idx: usize,
    text_scores: &[f64],
) -> (f64, f64, f64) {
    if start_idx >= fragments.len() || end_idx >= fragments.len() || start_idx > end_idx {
        return (0.4, 0.4, 0.0);
    }

    let win_start_t = fragments[start_idx].start;
    let win_end_t = fragments[end_idx].end;
    let win_dur = (win_end_t - win_start_t).max(1.0);

    // 1. Оценка текстового Хука (первые 3.2 секунды от начала первой реплики)
    let mut hook_score = 0.35_f64;
    let hook_triggers = [
        "подожди", "погоди", "смотри", "глянь", "слушай", "знаешь", "серьезно",
        "не может быть", "ты чего", "вы чего", "какого", "в смысле", "представь",
        "осторожно", "берегись", "стой", "не смей", "сейчас", "только не", "неужели",
        "правда", "кто это", "что это", "зачем", "почему",
    ];

    for idx in start_idx..=end_idx {
        let f = &fragments[idx];
        if f.start > win_start_t + 3.2 && idx > start_idx {
            break;
        }
        let sc = text_scores.get(idx).copied().unwrap_or(0.45);
        let lower = f.text.to_lowercase().replace('ё', "е");
        let mut cue_hook = sc * 0.75;
        if f.text.contains('?') {
            cue_hook += 0.16;
        }
        if f.text.contains('!') {
            cue_hook += 0.14;
        }
        if hook_triggers.iter().any(|t| lower.contains(t)) {
            cue_hook += 0.14;
        }
        if is_filler_or_water_fragment(&f.text) {
            cue_hook *= 0.45;
        }
        if cue_hook > hook_score {
            hook_score = cue_hook;
        }
    }

    // 2. Оценка текстового Панчлайна / Кульминации (последние 32% длительности окна или последние 2 реплики)
    let punch_cutoff_t = win_end_t - (win_dur * 0.32).clamp(2.5, 5.5);
    let mut punch_score = 0.35_f64;
    let punch_triggers = [
        "ха-ха", "ахах", "вот именно", "получай", "конец", "ни за что", "никогда",
        "гениально", "офигеть", "обалдеть", "жесть", "капец", "блин", "черт",
        "победа", "прощай", "берегись", "сдохни", "вперед", "огонь", "ура",
    ];

    for idx in start_idx..=end_idx {
        let f = &fragments[idx];
        if f.end < punch_cutoff_t && idx + 1 < end_idx {
            continue;
        }
        let sc = text_scores.get(idx).copied().unwrap_or(0.45);
        let lower = f.text.to_lowercase().replace('ё', "е");
        let mut cue_punch = sc * 0.80;
        if f.text.contains('!') {
            cue_punch += 0.16;
        }
        if f.text.contains("?!") || f.text.contains("!?") {
            cue_punch += 0.18;
        }
        if punch_triggers.iter().any(|t| lower.contains(t)) {
            cue_punch += 0.12;
        }
        if idx == end_idx && is_sentence_end(fragments, idx) && !is_weak_clip_ending(fragments, idx) {
            cue_punch += 0.08;
        }
        if cue_punch > punch_score {
            punch_score = cue_punch;
        }
    }

    // 3. Доля «воды» и слов-паразитов внутри окна
    let total_cues = (end_idx - start_idx + 1).max(1);
    let filler_cues = (start_idx..=end_idx)
        .filter(|&i| is_filler_or_water_fragment(&fragments[i].text))
        .count();
    let filler_ratio = (filler_cues as f64) / (total_cues as f64);

    (
        hook_score.clamp(0.0, 1.0),
        punchline_score_clamp(punch_score),
        filler_ratio.clamp(0.0, 1.0),
    )
}

#[inline]
fn punchline_score_clamp(val: f64) -> f64 {
    val.clamp(0.0, 1.0)
}

/// Оборачивает системный и пользовательский промпты в нативный чат-шаблон конкретной GGUF-модели
/// (Phi-3 `<|system|>...<|end|>` или Qwen2.5/Vikhr ChatML `<|im_start|>...<|im_end|>`)
fn format_prompt_for_gguf_model(
    model_path: &str,
    system_msg: &str,
    user_msg: &str,
    assistant_prefill: &str,
) -> String {
    let lower = model_path.to_lowercase();
    if lower.contains("phi-3") || lower.contains("phi3") {
        format!(
            "<|system|>\n{}\n<|end|>\n<|user|>\n{}\n<|end|>\n<|assistant|>\n{}",
            system_msg.trim(),
            user_msg.trim(),
            assistant_prefill
        )
    } else {
        format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n{}",
            system_msg.trim(),
            user_msg.trim(),
            assistant_prefill
        )
    }
}

fn format_preview_text_68(txt: &str) -> String {
    let clean = txt.split_whitespace().collect::<Vec<&str>>().join(" ");
    let char_len = clean.chars().count();
    if char_len <= 68 {
        clean
    } else {
        let truncated: String = clean.chars().take(68).collect();
        if let Some(pos) = truncated.rfind(' ') {
            if pos >= 36 {
                return format!("{}...", &truncated[..pos]);
            }
        }
        format!("{}...", truncated.trim_end())
    }
}

fn build_stage1_prompt_parts(fragments: &[SubtitleFragment]) -> (String, String) {
    let mut lines = Vec::new();
    for (i, frag) in fragments.iter().enumerate() {
        let clean_txt = frag.text.replace(['\n', '\r', '"'], " ");
        let start_tag = if is_sentence_start(fragments, i) { "[START]" } else { "[CONT]" };
        let end_tag = if is_sentence_end(fragments, i) { "[END]" } else { "[INCOMPLETE]" };
        lines.push(format!("{}. {} {} \"{}\"", i + 1, start_tag, end_tag, clean_txt.trim()));
    }
    let input_list = lines.join("\n");
    let sys = "Ты строгий профессиональный режиссер монтажа вирусных роликов (Shorts, Reels, TikTok, Хайлайты 16:9). \
        Возвращай ТОЛЬКО валидный JSON-массив чисел в диапазоне от 0.0 до 1.0 без пояснений и лишнего текста.";
    let usr = format!(
        "Оцени потенциал каждой из {} русских реплик от 0.0 до 1.0 по шкале (штрафуй «воду» и высоко цени хуки, юмор и кульминации!):\n\
1. 0.12-0.38: «вода», слова-паразиты («эээ», «ну короче»), обрывки ([CONT]/[INCOMPLETE]), бытовые реплики, сухие доклады.\n\
2. 0.42-0.65: обычные сюжетные диалоги, стандартные вопросы и спокойные пояснения.\n\
3. 0.70-0.85: сильные интригующие хуки, острый разговорный юмор, сарказм, живой сленг, напряженные споры, угрозы и драматические повороты.\n\
4. 0.88-0.97: ЯРКИЙ ПАНЧЛАЙН / ИСКЛЮЧИТЕЛЬНАЯ КУЛЬМИНАЦИЯ (взрывная шутка, развязка, яростный крик, шок, эпичная схватка — 1-2 реплики на пакет!).\n\
Выведи ТОЛЬКО валидный JSON-массив ровно из {} чисел в диапазоне 0.0-1.0.\n\
Пример формата: [0.42, 0.89]\n\n\
Реплики:\n{}",
        fragments.len(),
        fragments.len(),
        input_list
    );
    (sys.to_string(), usr)
}

fn build_prompt(fragments: &[SubtitleFragment]) -> String {
    let (sys, usr) = build_stage1_prompt_parts(fragments);
    format!("{}\n\n{}\n\nJSON:[", sys, usr)
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

    // Динамический лимит токенов пропорционально размеру пакета + стоп-токены `]`, `<|im_end|>`, `<|end|>`
    let max_tokens = (expected_count * 12 + 48).clamp(80, 420).to_string();

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
        "-c", "4096",
        "-ngl", ngl_val,
        "-t", &threads,
        "-n", &max_tokens,
        "-r", "]",
        "-r", "<|im_end|>",
        "-r", "<|end|>",
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
    let model_path_owned = model_path.to_string();
    let run_future = async move {
        let mut stdout_accum = String::new();
        let mut stderr_accum = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    stdout_accum.push_str(&String::from_utf8_lossy(&bytes));
                }
                CommandEvent::Stderr(bytes) => {
                    let line_str = String::from_utf8_lossy(&bytes);
                    stderr_accum.push_str(&line_str);
                    crate::logger::emit_stream_lines(&app_handle_clone, "LLM", "debug", &bytes);
                }
                CommandEvent::Terminated(payload) => {
                    let lower_err = stderr_accum.to_lowercase();
                    if lower_err.contains("unknown model architecture")
                        || lower_err.contains("error loading model")
                        || lower_err.contains("failed to load model")
                        || lower_err.contains("unable to load model")
                    {
                        crate::models::mark_model_broken(&model_path_owned);
                    }
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

    let no_chat_tokens = after_think
        .replace("<|im_end|>", "")
        .replace("<|end|>", "")
        .replace("<|assistant|>", "")
        .replace("<|endoftext|>", "");

    let cleaned = no_chat_tokens
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

    // 3. Извлекаем числа напрямую из префикса ответа (включая нумерованные списки вроде `1. 0.45`, `1) 0.80`)
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

    let extract_score_token = |raw: &str| -> Option<f64> {
        let s = raw.trim().trim_matches(|c: char| matches!(c, '[' | ']' | '"' | '\''));
        if s.is_empty() {
            return None;
        }
        if let Ok(v) = s.parse::<f64>() {
            if (0.0..=1.05).contains(&v) {
                return Some(v);
            }
        }
        if let Some((_, tail)) = s.rsplit_once(|c: char| c.is_whitespace() || c == ':' || c == ')' || c == '-') {
            let tail_trim = tail.trim().trim_matches(|c: char| matches!(c, '[' | ']' | '"' | '\''));
            if let Ok(v) = tail_trim.parse::<f64>() {
                if (0.0..=1.05).contains(&v) {
                    return Some(v);
                }
            }
        }
        None
    };

    let nums: Vec<f64> = prefix_region
        .split(|c: char| c == ',' || c == '\n' || c == ';')
        .filter_map(extract_score_token)
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

#[allow(dead_code)]
pub fn compute_text_scores_fallback(fragments: &[SubtitleFragment]) -> Vec<f64> {
    compute_text_scores_advanced(fragments)
}

/// Тип дефекта, обнаруженного у собранного клипа при 2-й проверке
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClipDefectType {
    None,
    IncompleteThought,    // Незавершенная мысль / обрыв начала или финала
    BoringOrWeakDynamics, // Скукота / «вода» / вялый темп речи
    NonsenseOrGarbage,    // Бессвязный мусор / галлюцинация ASR / склейка чужих сцен
}

/// Структура уже собранного кандидата в клипы для 2-й финальной проверки через русскоязычную LLM (Vikhr / Qwen)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembledClipCandidate {
    pub start: f64,
    pub end: f64,
    pub initial_score: f64,
    pub first_phrase: String,
    pub last_phrase: String,
    pub full_dialogue: String,
    pub first_cue_idx: Option<usize>,
    pub last_cue_idx: Option<usize>,
    pub cue_count: usize,
    #[serde(default = "default_round_one")]
    pub round: usize,
    #[serde(default)]
    pub repair_note: Option<String>,
}

fn default_round_one() -> usize {
    1
}

/// Вердикт 2-й проверки готового фрагмента на связность, юмор, законченность мысли и отсев бреда/скукоты
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipVerificationVerdict {
    pub final_score: f64,
    pub coherence_score: f64,
    pub keep: bool,
    pub reason: String,
    pub defect_type: ClipDefectType,
}

/// Строит системный и пользовательский промпты для 2-й (финальной) проверки УЖЕ СОБРАННЫХ фрагментов
fn build_clip_verification_parts(candidates: &[AssembledClipCandidate]) -> (String, String) {
    let mut items = Vec::with_capacity(candidates.len());
    for (i, cand) in candidates.iter().enumerate() {
        let dur = (cand.end - cand.start).max(1.0);
        let clean_dialogue: String = cand
            .full_dialogue
            .replace(['\n', '\r', '"'], " ")
            .chars()
            .take(540)
            .collect();
        items.push(format!(
            "{}. [Длит: {:.1}с | Реплик: {}] \"{}\"",
            i + 1,
            dur,
            cand.cue_count,
            clean_dialogue.trim()
        ));
    }

    let list_block = items.join("\n");
    let sys = "Ты главный шеф-редактор вирусных шортсов и хайлайтов. Выполняешь строгую вторую проверку уже собранных клипов. \
        Возвращай ТОЛЬКО валидный JSON-массив чисел от 0.0 до 1.0 без текста и комментариев.";
    let usr = format!(
        "Выполни СТРОГУЮ ВТОРУЮ ПРОВЕРКУ {} УЖЕ СОБРАННЫХ видеофрагментов (ниже приведен полный диалог каждого готового ролика целиком).\n\
Оцени каждый готовый фрагмент от 0.0 до 1.0 по 4 жестким правилам:\n\
1. СВЯЗНОСТЬ И ЗАКОНЧЕННОСТЬ МЫСЛИ: реплики внутри клипа должны образовывать единую понятную мысль с логичным началом и четкой финальной точкой/панчлайном. Если склеены куски разных тем или финал оборван на полуслове — ставь 0.10-0.28!\n\
2. ОТСЕВ ПОЛНОГО БРЕДА: если текст бессвязный, похож на словесный мусор, галлюцинацию распознавания или набор несвязанных выкриков — ставь 0.05-0.20!\n\
3. ОТСЕВ СКУКОТЫ И «ВОДЫ»: если в отрывке унылая бытовуха, сухие технические фразы, затянутая «вода» без интриги и эмоций — ставь 0.15-0.34!\n\
4. ЮМОР, САРКАЗМ И КУЛЬМИНАЦИЯ: высоко оценивай (0.72-0.98) живой русский юмор, иронию, сленг, острую перепалку (вопрос -> хлесткий ответ) и яркую развязку в конце ролика.\n\
Выведи ТОЛЬКО валидный JSON-массив ровно из {} чисел от 0.0 до 1.0.\n\
Пример: [0.86, 0.18]\n\n\
Готовые фрагменты для 2-й проверки:\n{}",
        candidates.len(),
        candidates.len(),
        list_block
    );
    (sys.to_string(), usr)
}

fn build_clip_verification_prompt(candidates: &[AssembledClipCandidate]) -> String {
    let (sys, usr) = build_clip_verification_parts(candidates);
    format!("{}\n\n{}\n\nJSON:[", sys, usr)
}

/// Детерминированный аудит собранного диалога клипа на:
/// 1) Полный бред / разорванные фразы / склейку разных сцен
/// 2) Законченность мысли в начале и в конце
/// 3) Скукоту / «воду» vs живой русский юмор, сарказм и драматический панчлайн
fn audit_assembled_clip_dialogue(
    cand: &AssembledClipCandidate,
    fragments: &[SubtitleFragment],
) -> (f64, bool, bool, bool, String) {
    // Возвращает: (score, is_nonsense, is_boring, is_incomplete, reason)
    let (Some(first_idx), Some(last_idx)) = (cand.first_cue_idx, cand.last_cue_idx) else {
        let keep_action = cand.initial_score >= 0.62;
        return (
            cand.initial_score,
            false,
            !keep_action,
            false,
            if keep_action {
                "чистая динамичная экшн-сцена по пику звука".to_string()
            } else {
                "низкая динамика сцены без диалогов".to_string()
            },
        );
    };

    if first_idx >= fragments.len() || last_idx >= fragments.len() || first_idx > last_idx {
        return (0.20, true, false, false, "некорректные границы реплик".to_string());
    }

    let slice = &fragments[first_idx..=last_idx];
    let total_cues = slice.len().max(1);

    // 1. Проверка на галлюцинации, песни и «воду» внутри собранного клипа
    let mut hallucination_or_song_cues = 0usize;
    let mut filler_cues = 0usize;
    let mut total_words = 0usize;
    let mut broken_short_tokens = 0usize;
    let mut exclam_cues = 0usize;
    let mut question_cues = 0usize;

    for (rel_i, f) in slice.iter().enumerate() {
        let abs_idx = first_idx + rel_i;
        if crate::whisper::is_whisper_hallucination(&f.text)
            || is_opening_or_song_fragment(fragments, abs_idx)
        {
            hallucination_or_song_cues += 1;
        }
        if is_filler_or_water_fragment(&f.text) {
            filler_cues += 1;
        }
        if f.text.contains('!') {
            exclam_cues += 1;
        }
        if f.text.contains('?') {
            question_cues += 1;
        }
        for w in f.text.split_whitespace() {
            let clean_w: String = w.chars().filter(|c| c.is_alphabetic()).collect();
            if !clean_w.is_empty() {
                total_words += 1;
                if clean_w.chars().count() == 1
                    && !matches!(
                        clean_w.to_lowercase().as_str(),
                        "я" | "и" | "а" | "в" | "с" | "к" | "у" | "о"
                    )
                {
                    broken_short_tokens += 1;
                }
            }
        }
    }

    if hallucination_or_song_cues * 2 >= total_cues {
        return (
            0.12,
            true,
            false,
            false,
            "галлюцинация распознавания / музыкальная вставка".to_string(),
        );
    }

    if total_words > 6 && broken_short_tokens * 3 >= total_words {
        return (
            0.15,
            true,
            false,
            false,
            "бессвязный набор обрывочных слогов (бред ASR)".to_string(),
        );
    }

    // 2. Проверка на склейку двух несвязанных сцен внутри одного клипа
    for abs_idx in (first_idx + 1)..=last_idx {
        let gap = (fragments[abs_idx].start - fragments[abs_idx - 1].end).max(0.0);
        if is_new_scene_opener(&fragments[abs_idx].text) && gap >= 1.2 {
            return (
                0.22,
                true,
                false,
                false,
                "склейка двух разных несвязанных сцен внутри одного клипа".to_string(),
            );
        }
    }

    // 3. Проверка начала и конца клипа на законченность мысли
    let bad_start = !is_sentence_start(fragments, first_idx) || is_weak_clip_start(fragments, first_idx);
    let bad_end = !is_sentence_end(fragments, last_idx) || is_weak_clip_ending(fragments, last_idx);
    if bad_start && bad_end {
        return (
            0.20,
            false,
            false,
            true,
            "мысль оборвана и в начале, и в финале фрагмента".to_string(),
        );
    }
    if bad_end {
        return (
            0.28,
            false,
            false,
            true,
            "незаконченная мысль / обрыв фразы в финале клипа".to_string(),
        );
    }
    if bad_start {
        return (
            0.32,
            false,
            false,
            true,
            "мысль оборвана в начале (вход с полуслова)".to_string(),
        );
    }

    // 4. Проверка на повтор одних и тех же фраз (зацикливание / бред)
    if total_cues >= 3 {
        let mut unique_lines = HashSet::new();
        for f in slice {
            let norm = f
                .text
                .trim()
                .to_lowercase()
                .replace('ё', "е")
                .chars()
                .filter(|c| c.is_alphanumeric() || c.is_whitespace())
                .collect::<String>();
            if !norm.is_empty() {
                unique_lines.insert(norm);
            }
        }
        if unique_lines.len() * 2 <= total_cues {
            return (
                0.18,
                true,
                false,
                false,
                "повторяющиеся зацикленные реплики (бред)".to_string(),
            );
        }
    }

    // 5. Проверка на скукоту и «воду»
    let filler_ratio = (filler_cues as f64) / (total_cues as f64);
    let lower_full = cand.full_dialogue.to_lowercase().replace('ё', "е");

    let humor_and_vivid_markers = [
        "ха-ха", "ахах", "хе-хе", "шутк", "смеш", "прикол", "угар", "жесть", "капец",
        "офигеть", "обалдеть", "нифига", "блин", "черт", "серьезно", "издеваешься",
        "издеваетесь", "с ума", "псих", "безум", "гениально", "хитр", "ловушк",
        "сюрприз", "получай", "не может быть", "шутки плохи", "дьявол", "монстр",
        "проклятье", "крылышки", "наконец-то", "вот это", "ни за что", "никогда",
        "взрыв", "атака", "огонь", "спасай", "бежим", "сдохни", "умри", "победа",
    ];
    let boring_markers = [
        "проверка звука", "меня слышно", "перейдем к следующему", "в сегодняшнем видео",
        "нажмите кнопку", "по расписанию", "обычное дело", "ничего особенного",
        "доклад окончен", "такие дела", "ну как-то так", "в общем-то все",
    ];

    let humor_hits = humor_and_vivid_markers
        .iter()
        .filter(|&&m| lower_full.contains(m))
        .count();
    let boring_hits = boring_markers
        .iter()
        .filter(|&&m| lower_full.contains(m))
        .count();

    if filler_ratio >= 0.42 || (boring_hits > 0 && humor_hits == 0 && exclam_cues == 0) {
        return (
            0.26,
            false,
            true,
            false,
            "скукота и «вода» без развития мысли и эмоций".to_string(),
        );
    }

    let clip_dur = (cand.end - cand.start).max(1.0);
    let words_per_sec = (total_words as f64) / clip_dur;
    if words_per_sec < 0.65 && exclam_cues == 0 && humor_hits == 0 && cand.initial_score < 0.64 {
        return (
            0.30,
            false,
            true,
            false,
            "вялый темп речи и отсутствие эмоциональной кульминации (скукота)".to_string(),
        );
    }

    // 6. Расчет итогового детерминированного балла связности, юмора и законченности мысли
    let mut score = 0.52_f64;

    // Бонус за четкое начало и завершение мысли
    if !bad_start {
        score += 0.08;
    }
    if has_terminal_punctuation(&fragments[last_idx].text) && !has_incomplete_clause(&fragments[last_idx].text) {
        score += 0.08;
    }

    // Бонус за диалоговую структуру «Сетап / Вопрос -> Панчлайн / Ответ с !»
    let first_half_has_question = slice
        .iter()
        .take((total_cues / 2).max(1))
        .any(|f| f.text.contains('?'));
    let second_half_has_exclam = slice
        .iter()
        .skip(total_cues / 2)
        .any(|f| f.text.contains('!'));
    if first_half_has_question && second_half_has_exclam {
        score += 0.12;
    } else if exclam_cues > 0 {
        score += (exclam_cues.min(3) as f64) * 0.05;
    }

    // Бонус за юмор, иронию, сленг и яркие обороты
    if humor_hits > 0 {
        score += (humor_hits.min(3) as f64) * 0.065;
    }

    // Штраф за частичную воду
    score -= filler_ratio * 0.25;

    let clamped = score.clamp(0.15, 0.96);
    let is_boring = clamped < 0.38 && exclam_cues == 0 && humor_hits == 0 && question_cues == 0;
    let reason = if humor_hits > 0 && first_half_has_question && second_half_has_exclam {
        "связный диалог: интригующий зачин + юмор/панчлайн в финале".to_string()
    } else if humor_hits > 0 {
        "законченная мысль с живым юмором/эмоцией".to_string()
    } else if second_half_has_exclam {
        "цельная сцена с яркой эмоциональной кульминацией в конце".to_string()
    } else {
        "логически завершенный связный фрагмент".to_string()
    };

    (clamped, false, is_boring, false, reason)
}

/// Выполняет 2-ю финальную проверку уже собранных фрагментов через русскоязычную LLM (приоритет: модели Vikhr 2B/1.5B/1B)
/// в связке с детерминированным аудитором связности, юмора, законченности мысли и отсевом полного бреда/скукоты.
pub async fn verify_assembled_clips_coherence(
    candidates: &[AssembledClipCandidate],
    fragments: &[SubtitleFragment],
    config: &TextAnalysisConfig,
    app_handle: &AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<Vec<ClipVerificationVerdict>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    // 1. Сначала выполняем детерминированный аудит каждого собранного клипа
    let det_audits: Vec<(f64, bool, bool, bool, String)> = candidates
        .iter()
        .map(|c| audit_assembled_clip_dialogue(c, fragments))
        .collect();

    // 2. Запускаем 2-ю проверку через русскоязычную LLM (Vikhr GGUF через llama-cli, либо выбранный движок с авто-фоллбэком на Vikhr)
    let mut llm_scores_opt: Option<Vec<f64>> = None;

    // Проверяем наличие локальной русской модели Vikhr (или выбранной GGUF-модели) для 2-й проверки
    let russian_gguf_opt = crate::models::resolve_russian_verifier_llm_model(
        app_handle,
        Some(&config.model_name),
    );

    if config.engine != TextEngine::Builtin {
        let batch_size = 8usize;
        let total_batches = (candidates.len() + batch_size - 1) / batch_size;
        let mut collected_llm_scores: Vec<f64> = Vec::with_capacity(candidates.len());
        let mut llm_available = true;

        if let Some(ref gguf_path) = russian_gguf_opt {
            if config.engine == TextEngine::LlamaCli || config.engine == TextEngine::Auto {
                let model_file_label = gguf_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Vikhr-GGUF");
                let is_round_two = candidates.iter().any(|c| c.round > 1);
                let round_label = if is_round_two { "2-й круг ревизии" } else { "2-я проверка ИИ" };
                crate::logger::emit_log(
                    app_handle,
                    "LLM",
                    "info",
                    &format!(
                        "🇷🇺 [{}] Экспертиза {} фрагментов на связность, юмор и отсев бреда через русскую модель '{}'...",
                        round_label,
                        candidates.len(),
                        model_file_label
                    ),
                );
            }
        }

        let http_client = Client::builder()
            .timeout(Duration::from_secs(25))
            .build()
            .unwrap_or_default();

        for (b_idx, chunk) in candidates.chunks(batch_size).enumerate() {
            if cancel_rx.try_recv().is_ok() {
                return Err(AppError::Pipeline("Отменено пользователем".into()));
            }
            if !llm_available {
                break;
            }

            let (sys_p, usr_p) = build_clip_verification_parts(chunk);
            let prompt = build_clip_verification_prompt(chunk);
            let mut parsed_batch: Option<Vec<f64>> = None;

            if config.engine == TextEngine::OpenAi {
                if let Ok(resp) = run_openai(&http_client, config, &prompt, app_handle).await {
                    parsed_batch = parse_json_array(&resp, chunk.len());
                }
            } else if config.engine == TextEngine::Ollama {
                if let Ok(resp) = run_ollama(&http_client, config, &prompt, app_handle).await {
                    parsed_batch = parse_json_array(&resp, chunk.len());
                }
            }

            // Если выбран локальный GGUF (LlamaCli) ИЛИ внешний движок не ответил — используем нативный чат-шаблон выбранной GGUF-модели!
            if parsed_batch.is_none() {
                if let Some(ref gguf_path) = russian_gguf_opt {
                    let gguf_str = gguf_path.to_string_lossy().to_string();
                    let gguf_name = gguf_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("GGUF");
                    if !is_model_marked_broken(&gguf_str) {
                        crate::logger::emit_log(
                            app_handle,
                            "LLM",
                            "info",
                            &format!(
                                "🇷🇺 [2-я проверка ИИ] Пакет клипов {}/{}: оценка связности, юмора и законченности мысли ({} шт.)...",
                                b_idx + 1,
                                total_batches,
                                chunk.len()
                            ),
                        );
                        let gguf_prompt = format_prompt_for_gguf_model(&gguf_str, &sys_p, &usr_p, "JSON:[");
                        match run_llama_cli(
                            app_handle,
                            &gguf_str,
                            &gguf_prompt,
                            chunk.len(),
                            true,
                            cancel_rx.resubscribe(),
                        )
                        .await
                        {
                            Ok(out) => {
                                parsed_batch = parse_json_array(&out, chunk.len());
                                if let Some(ref scores) = parsed_batch {
                                    crate::logger::emit_log(
                                        app_handle,
                                        "LLM",
                                        "info",
                                        &format!(
                                            "✓ [2-я проверка ИИ] Пакет {}/{}: получено {} оценок от '{}' (GPU)",
                                            b_idx + 1,
                                            total_batches,
                                            scores.len(),
                                            gguf_name
                                        ),
                                    );
                                } else {
                                    crate::logger::emit_log(
                                        app_handle,
                                        "LLM",
                                        "warn",
                                        &format!(
                                            "⟳ [2-я проверка ИИ] Пакет {}/{}: модель '{}' не дала валидный JSON на GPU, повтор на CPU...",
                                            b_idx + 1,
                                            total_batches,
                                            gguf_name
                                        ),
                                    );
                                    if let Ok(cpu_out) = run_llama_cli(
                                        app_handle,
                                        &gguf_str,
                                        &gguf_prompt,
                                        chunk.len(),
                                        false,
                                        cancel_rx.resubscribe(),
                                    )
                                    .await
                                    {
                                        parsed_batch = parse_json_array(&cpu_out, chunk.len());
                                        if let Some(ref scores) = parsed_batch {
                                            crate::logger::emit_log(
                                                app_handle,
                                                "LLM",
                                                "info",
                                                &format!(
                                                    "✓ [2-я проверка ИИ] Пакет {}/{}: получено {} оценок от '{}' (CPU)",
                                                    b_idx + 1,
                                                    total_batches,
                                                    scores.len(),
                                                    gguf_name
                                                ),
                                            );
                                        }
                                    }
                                    if parsed_batch.is_none() {
                                        crate::logger::emit_log(
                                            app_handle,
                                            "LLM",
                                            "warn",
                                            &format!(
                                                "⚠ [2-я проверка ИИ] Пакет {}/{}: нестандартный вывод '{}', применен строгий детерминированный аудит связности.",
                                                b_idx + 1,
                                                total_batches,
                                                gguf_name
                                            ),
                                        );
                                    }
                                }
                            }
                            Err(_) => {
                                llm_available = false;
                            }
                        }
                    } else {
                        llm_available = false;
                    }
                } else {
                    llm_available = false;
                }
            }

            if let Some(mut scores) = parsed_batch {
                while scores.len() < chunk.len() {
                    let idx_in_chunk = scores.len();
                    let global_i = b_idx * batch_size + idx_in_chunk;
                    scores.push(det_audits[global_i].0);
                }
                collected_llm_scores.extend(scores.into_iter().take(chunk.len()));
            } else {
                for idx_in_chunk in 0..chunk.len() {
                    let global_i = b_idx * batch_size + idx_in_chunk;
                    collected_llm_scores.push(det_audits[global_i].0);
                }
            }
        }

        if collected_llm_scores.len() == candidates.len() {
            llm_scores_opt = Some(collected_llm_scores);
        }
    }

    // 3. Формируем финальные вердикты (объединяем оценку русской LLM и структурный аудит связности/бреда/скукоты)
    let mut verdicts = Vec::with_capacity(candidates.len());
    for (i, cand) in candidates.iter().enumerate() {
        let (det_score, is_nonsense, is_boring, is_incomplete, det_reason) = &det_audits[i];
        let llm_score = llm_scores_opt
            .as_ref()
            .and_then(|v| v.get(i).copied())
            .unwrap_or(*det_score);

        // Комбинированная оценка связности, юмора и законченности мысли
        let coherence_score = if llm_scores_opt.is_some() {
            (llm_score * 0.62 + det_score * 0.38).clamp(0.05, 0.98)
        } else {
            *det_score
        };

        // Жесткая отбраковка полного бреда, оборванных мыслей и откровенной скукоты
        let llm_flagged_garbage = llm_scores_opt.is_some() && llm_score < 0.34;
        let keep = !*is_nonsense && !*is_incomplete && !(*is_boring && coherence_score < 0.48) && !llm_flagged_garbage && coherence_score >= 0.38;

        let defect_type = if keep {
            ClipDefectType::None
        } else if *is_incomplete {
            ClipDefectType::IncompleteThought
        } else if *is_nonsense {
            ClipDefectType::NonsenseOrGarbage
        } else if *is_boring || llm_flagged_garbage {
            ClipDefectType::BoringOrWeakDynamics
        } else {
            ClipDefectType::IncompleteThought
        };

        let reason = if !keep {
            if *is_incomplete {
                det_reason.clone()
            } else if *is_nonsense {
                det_reason.clone()
            } else if llm_flagged_garbage {
                format!(
                    "отбракован ИИ-редактором как скучный/бессвязный фрагмент (оценка LLM: {:.2})",
                    llm_score
                )
            } else {
                det_reason.clone()
            }
        } else if llm_scores_opt.is_some() && llm_score >= 0.76 {
            format!("высокая оценка ИИ-редактора ({:.2}): {}", llm_score, det_reason)
        } else {
            det_reason.clone()
        };

        let final_score = if keep {
            (cand.initial_score * 0.48 + coherence_score * 0.52).clamp(0.15, 0.97)
        } else {
            (coherence_score * 0.5).clamp(0.05, 0.35)
        };

        verdicts.push(ClipVerificationVerdict {
            final_score,
            coherence_score,
            keep,
            reason,
            defect_type,
        });
    }

    Ok(verdicts)
}

/// Вспомогательный поиск альтернативного связного эмоционального пика рядом с проблемным клипом (±25с)
fn find_clean_peak_window_near(
    cand: &AssembledClipCandidate,
    fragments: &[SubtitleFragment],
    text_scores: &[f64],
    audio_profile: Option<&crate::audio_analyzer::AudioEnergyProfile>,
    min_dur: f64,
    max_dur: f64,
    intro_skip: f64,
    outro_limit: f64,
    video_duration: f64,
) -> Option<AssembledClipCandidate> {
    let search_min = (cand.start - 25.0).max(intro_skip).max(0.0);
    let search_max = (cand.end + 25.0).min(outro_limit).min(video_duration);

    let mut best_anchor: Option<(usize, f64)> = None;

    for (i, f) in fragments.iter().enumerate() {
        if f.start < search_min || f.end > search_max {
            continue;
        }
        if crate::whisper::is_whisper_hallucination(&f.text) || is_opening_or_song_fragment(fragments, i) {
            continue;
        }

        let mut peak_val = text_scores.get(i).copied().unwrap_or(0.35);
        if f.text.contains('!') {
            peak_val += 0.22;
        }
        if f.text.contains("?!") || f.text.contains("!?") {
            peak_val += 0.25;
        }
        if f.text.chars().filter(|c| c.is_alphabetic()).any(|c| c.is_uppercase()) && f.text.contains('!') {
            peak_val += 0.18;
        }

        if let Some(prof) = audio_profile {
            let audio_burst = prof.get_peak_energy_between(f.start, f.end);
            peak_val += audio_burst * 0.20;
        }

        let dist_from_old_center = ((f.start + f.end) / 2.0 - (cand.start + cand.end) / 2.0).abs();
        if dist_from_old_center < 3.0 {
            peak_val *= 0.65;
        }

        if peak_val > best_anchor.map(|(_, sc)| sc).unwrap_or(0.0) {
            best_anchor = Some((i, peak_val));
        }
    }

    if let Some((anchor_i, peak_sc)) = best_anchor {
        if peak_sc >= 0.40 {
            let mut start_i = anchor_i;
            while start_i > 0 && !is_sentence_start(fragments, start_i) && (anchor_i - start_i) < 4 {
                start_i -= 1;
            }
            let mut end_i = anchor_i;
            while end_i + 1 < fragments.len()
                && (fragments[end_i].end - fragments[start_i].start) < min_dur
            {
                end_i += 1;
            }
            while end_i + 1 < fragments.len()
                && (fragments[end_i + 1].end - fragments[start_i].start) <= max_dur + 4.0
                && (!is_sentence_end(fragments, end_i) || is_weak_clip_ending(fragments, end_i))
            {
                end_i += 1;
            }

            if !is_sentence_end(fragments, end_i) || is_weak_clip_ending(fragments, end_i) {
                for back_e in ((start_i + 1)..=end_i).rev() {
                    if is_sentence_end(fragments, back_e)
                        && !is_weak_clip_ending(fragments, back_e)
                        && (fragments[back_e].end - fragments[start_i].start) >= min_dur * 0.72
                    {
                        end_i = back_e;
                        break;
                    }
                }
            }

            if !is_sentence_end(fragments, end_i) || is_weak_clip_ending(fragments, end_i) {
                return None;
            }

            let new_start = (fragments[start_i].start - 0.25).max(intro_skip).max(0.0);
            let new_end = (fragments[end_i].end + 0.35).min(outro_limit).min(video_duration);

            let shift = (new_start - cand.start).abs() + (new_end - cand.end).abs();
            if shift >= 2.5 && (new_end - new_start) >= min_dur * 0.72 {
                let mut dialogue_parts: Vec<String> = Vec::new();
                for f in &fragments[start_i..=end_i] {
                    let trimmed = f.text.trim();
                    if !trimmed.is_empty() {
                        dialogue_parts.push(trimmed.to_string());
                    }
                }
                let first_p = format_preview_text_68(&fragments[start_i].text);
                let last_p = format_preview_text_68(&fragments[end_i].text);
                let full_dialogue = dialogue_parts.join(" — ");

                return Some(AssembledClipCandidate {
                    start: new_start,
                    end: new_end,
                    initial_score: (cand.initial_score * 0.5 + peak_sc * 0.5).clamp(0.40, 0.95),
                    first_phrase: first_p,
                    last_phrase: last_p,
                    full_dialogue,
                    first_cue_idx: Some(start_i),
                    last_cue_idx: Some(end_i),
                    cue_count: dialogue_parts.len(),
                    round: 2,
                    repair_note: Some(format!(
                        "Фокус сцены перенесен на яркий эмоциональный пик на {:.1}с",
                        fragments[anchor_i].start
                    )),
                });
            }
        }
    }

    None
}

/// Функция адресной доработки клипа, не прошедшего 1-й круг проверки (Self-Repair Review Loop)
/// Согласно требованиям:
/// 1) При незавершенной мысли / обрыве: расширяет границы до закрытия фразы и панчлайна (или подрезает к ближайшей чистой точке, не пропуская битый финал на 2-й круг)
/// 2) При скукоте или бреде: ищет яркий эмоциональный момент в том же тайм-диапазоне
/// 3) Выполняется строго 1 раз (round == 2), защищая от бесконечных циклов
pub fn repair_clip_candidate(
    cand: &AssembledClipCandidate,
    defect: &ClipDefectType,
    fragments: &[SubtitleFragment],
    text_scores: &[f64],
    audio_profile: Option<&crate::audio_analyzer::AudioEnergyProfile>,
    min_dur: f64,
    max_dur: f64,
    intro_skip: f64,
    outro_limit: f64,
    video_duration: f64,
) -> Option<AssembledClipCandidate> {
    if fragments.is_empty() {
        return None;
    }

    match defect {
        ClipDefectType::IncompleteThought => {
            let first_idx = cand.first_cue_idx.unwrap_or(0);
            let last_idx = cand.last_cue_idx.unwrap_or_else(|| fragments.len().saturating_sub(1));
            if first_idx >= fragments.len() || last_idx >= fragments.len() || first_idx > last_idx {
                return None;
            }

            // А. Проверяем начало: если фраза начинается на полуслове / со строчной / союзе продолжения, сдвигаем назад
            let mut repaired_first_idx = first_idx;
            if first_idx > 0 && (!is_sentence_start(fragments, first_idx) || is_weak_clip_start(fragments, first_idx)) {
                let start_bound = first_idx.saturating_sub(5);
                for back_i in (start_bound..first_idx).rev() {
                    if is_sentence_start(fragments, back_i) && !is_weak_clip_start(fragments, back_i) {
                        repaired_first_idx = back_i;
                        break;
                    }
                }
            }

            // Б. Проверяем финал: сканируем вперед до точки/восклицания/панчлайна в пределах эластичного лимита
            let mut repaired_last_idx = last_idx;
            let max_repaired_dur = (max_dur + 7.0).min(65.0);
            let forward_bound = (last_idx + 10).min(fragments.len().saturating_sub(1));
            for fwd_i in (last_idx + 1)..=forward_bound {
                let dur = fragments[fwd_i].end - fragments[repaired_first_idx].start;
                if dur > max_repaired_dur {
                    break;
                }
                if is_sentence_end(fragments, fwd_i) && !is_weak_clip_ending(fragments, fwd_i) {
                    repaired_last_idx = fwd_i;
                    if fragments[fwd_i].text.contains('!') || fragments[fwd_i].text.contains("?!") {
                        break;
                    }
                }
            }

            // Б2. Если финал всё ещё не закрыт (не хватило лимита длительности вперед):
            // 1) пробуем сдвинуть старт чуть вперед к следующему чистому началу предложения, чтобы освободить место под панчлайн;
            if !is_sentence_end(fragments, repaired_last_idx) || is_weak_clip_ending(fragments, repaired_last_idx) {
                let shift_limit = (repaired_first_idx + 4).min(last_idx);
                for cand_first in (repaired_first_idx + 1)..=shift_limit {
                    if !is_sentence_start(fragments, cand_first) || is_weak_clip_start(fragments, cand_first) {
                        continue;
                    }
                    let mut found_end: Option<usize> = None;
                    for fwd_i in last_idx..=forward_bound {
                        let dur = fragments[fwd_i].end - fragments[cand_first].start;
                        if dur > max_repaired_dur {
                            break;
                        }
                        if dur >= min_dur * 0.72
                            && is_sentence_end(fragments, fwd_i)
                            && !is_weak_clip_ending(fragments, fwd_i)
                        {
                            found_end = Some(fwd_i);
                            break;
                        }
                    }
                    if let Some(clean_end) = found_end {
                        repaired_first_idx = cand_first;
                        repaired_last_idx = clean_end;
                        break;
                    }
                }
            }

            // 2) Если вперёд закрыть не удалось — ищем чистый конец предложения внутри окна, отступая назад от `last_idx`
            if !is_sentence_end(fragments, repaired_last_idx) || is_weak_clip_ending(fragments, repaired_last_idx) {
                for back_e in ((repaired_first_idx + 1)..last_idx).rev() {
                    let dur = fragments[back_e].end - fragments[repaired_first_idx].start;
                    if dur < min_dur * 0.72 {
                        break;
                    }
                    if is_sentence_end(fragments, back_e) && !is_weak_clip_ending(fragments, back_e) {
                        repaired_last_idx = back_e;
                        break;
                    }
                }
            }

            let start_ok = is_sentence_start(fragments, repaired_first_idx) && !is_weak_clip_start(fragments, repaired_first_idx);
            let end_ok = is_sentence_end(fragments, repaired_last_idx) && !is_weak_clip_ending(fragments, repaired_last_idx);
            if !start_ok || !end_ok {
                // Если даже после коррекции границ мысль не замкнулась — переносим окно на соседний цельный пик в ±25с
                return find_clean_peak_window_near(
                    cand,
                    fragments,
                    text_scores,
                    audio_profile,
                    min_dur,
                    max_dur,
                    intro_skip,
                    outro_limit,
                    video_duration,
                );
            }

            let new_start = (fragments[repaired_first_idx].start - 0.25).max(intro_skip).max(0.0);
            let new_end = (fragments[repaired_last_idx].end + 0.35).min(outro_limit).min(video_duration);

            if new_end - new_start < min_dur * 0.72 {
                return None;
            }

            let mut dialogue_parts: Vec<String> = Vec::new();
            for f in &fragments[repaired_first_idx..=repaired_last_idx] {
                let trimmed = f.text.trim();
                if !trimmed.is_empty() {
                    dialogue_parts.push(trimmed.to_string());
                }
            }
            let first_p = format_preview_text_68(&fragments[repaired_first_idx].text);
            let last_p = format_preview_text_68(&fragments[repaired_last_idx].text);
            let full_dialogue = dialogue_parts.join(" — ");
            let delta_dur = (new_end - new_start) - (cand.end - cand.start);

            Some(AssembledClipCandidate {
                start: new_start,
                end: new_end,
                initial_score: cand.initial_score,
                first_phrase: first_p,
                last_phrase: last_p,
                full_dialogue,
                first_cue_idx: Some(repaired_first_idx),
                last_cue_idx: Some(repaired_last_idx),
                cue_count: dialogue_parts.len(),
                round: 2,
                repair_note: Some(format!(
                    "Границы уточнены [{:.1}с - {:.1}с] ({:+.1}с) до законченного панчлайна",
                    new_start, new_end, delta_dur
                )),
            })
        }
        ClipDefectType::BoringOrWeakDynamics | ClipDefectType::NonsenseOrGarbage => {
            find_clean_peak_window_near(
                cand,
                fragments,
                text_scores,
                audio_profile,
                min_dur,
                max_dur,
                intro_skip,
                outro_limit,
                video_duration,
            )
        }
        ClipDefectType::None => None,
    }
}

// =========================================================================================
// ФИНАЛЬНАЯ (3-Я) LLM-ЗАДАЧА: SMM-КОПИРАЙТЕР (НАЗВАНИЕ, ОПИСАНИЕ С ИРОНИЕЙ, ХЕШТЕГИ + .TXT)
// =========================================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmmClipMetadata {
    pub clip_path: String,
    pub txt_path: String,
    pub clip_type: String, // "shorts" | "highlights" | "talk_video"
    pub title: String,
    pub description: String,
    pub hashtags: String,
    pub full_copy_text: String,
    pub model_used: String,
    pub start_sec: f64,
    pub end_sec: f64,
}

#[derive(Debug, Clone)]
pub struct SmmGenerationInput {
    pub clip_path: String,
    pub clip_type: String, // "shorts" | "highlights" | "talk_video"
    pub clip_index: usize,
    pub total_clips: usize,
    pub video_title: String,
    pub start_sec: f64,
    pub end_sec: f64,
    pub inside_dialogue: String,
    pub surrounding_context: String,
    pub visual_actions: String,
    pub characters: Vec<String>,
}

/// Очищает техническое имя видеофайла от мусорных тегов релиза (`[Окане]`, `TV1`, `1080p`),
/// сохраняя чистое художественное название видео и номер серии (например: `[1 Серия]Маска вермильона, TV1[Окане]` -> `Маска вермильона (1 серия)`)
pub fn clean_video_title_for_smm(raw_stem: &str) -> String {
    let normalized = raw_stem.replace('_', " ");
    let mut episode_tag: Option<String> = None;
    let mut outside = String::new();
    let mut inside_bracket = String::new();
    let mut bracket_depth = 0usize;

    for ch in normalized.chars() {
        if ch == '[' || ch == '(' || ch == '【' {
            if bracket_depth == 0 {
                inside_bracket.clear();
            } else {
                inside_bracket.push(ch);
            }
            bracket_depth += 1;
        } else if ch == ']' || ch == ')' || ch == '】' {
            if bracket_depth > 0 {
                bracket_depth -= 1;
                if bracket_depth == 0 {
                    let block = inside_bracket.trim();
                    let lower_block = block.to_lowercase().replace('ё', "е");
                    let has_digit = lower_block.chars().any(|c| c.is_ascii_digit());
                    if has_digit
                        && (lower_block.contains("сери")
                            || lower_block.contains("эпизод")
                            || lower_block.contains("выпуск")
                            || lower_block.contains("часть")
                            || lower_block.starts_with("ep"))
                    {
                        let ep_clean = block
                            .split_whitespace()
                            .collect::<Vec<&str>>()
                            .join(" ")
                            .to_lowercase();
                        episode_tag = Some(ep_clean);
                    }
                    outside.push(' ');
                } else {
                    inside_bracket.push(ch);
                }
            }
        } else if bracket_depth > 0 {
            inside_bracket.push(ch);
        } else {
            outside.push(ch);
        }
    }

    // Удаляем технические маркеры (TV1, TV2, 1080p, WEB-DL, BDRip, x264 и т.д.)
    let mut words: Vec<String> = Vec::new();
    for raw_w in outside.split_whitespace() {
        let w_clean = raw_w
            .trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '[' | ']' | '(' | ')' | '-' | '.'))
            .trim();
        if w_clean.is_empty() {
            continue;
        }
        let lower_w = w_clean.to_lowercase();
        let is_tv_marker = (lower_w.starts_with("tv") && lower_w.len() <= 5 && lower_w[2..].chars().all(|c| c.is_ascii_digit() || c == '-'))
            || matches!(
                lower_w.as_str(),
                "1080p"
                    | "720p"
                    | "2160p"
                    | "4k"
                    | "web-dl"
                    | "webrip"
                    | "bdrip"
                    | "bluray"
                    | "x264"
                    | "x265"
                    | "hevc"
                    | "h264"
                    | "aac"
                    | "flac"
                    | "rus"
                    | "sub"
                    | "dub"
                    | "raw"
            );
        if !is_tv_marker {
            words.push(w_clean.to_string());
        }
    }

    let base_title = words.join(" ").trim().trim_matches([',', '-', ' ']).to_string();
    let final_base = if base_title.is_empty() {
        raw_stem
            .replace(['[', ']', '(', ')'], " ")
            .split_whitespace()
            .collect::<Vec<&str>>()
            .join(" ")
    } else {
        base_title
    };

    if let Some(ep) = episode_tag {
        if !final_base.to_lowercase().contains(&ep) {
            return format!("{} ({})", final_base, ep);
        }
    }

    final_base
}

/// Очищает реплику субтитров от префикса диктора/актера дубляжа (оставлено для вспомогательных проверок)
#[allow(dead_code)]
fn strip_speaker_prefix(line: &str) -> String {
    let mut s = line.trim();
    // Убираем ведущие дефисы диалога
    s = s.trim_start_matches(['-', '—', '•', ' ']);

    if let Some((prefix, rest)) = s.split_once(':') {
        let p_trim = prefix.trim();
        let word_cnt = p_trim.split_whitespace().count();
        // Если перед двоеточием 1–3 коротких слова (имя персонажа или ник актера озвучки до 26 символов без знаков препинания)
        let has_sentence_punct = p_trim.contains(['.', '!', '?', ',', ';']);
        if !p_trim.is_empty() && p_trim.chars().count() <= 26 && word_cnt <= 3 && !has_sentence_punct {
            let cleaned_rest = rest.trim().trim_start_matches(['-', '—', ' ']);
            if !cleaned_rest.is_empty() {
                return cleaned_rest.to_string();
            }
        }
    }

    s.to_string()
}

/// Нормализует строку диалога для SMM-копирайтера, сохраняя имя персонажа (`Персонаж: реплика`),
/// чтобы ИИ точно понимал, кто с кем спорит и что происходит в сцене
fn clean_smm_dialogue_line(line: &str) -> String {
    line.trim()
        .trim_start_matches(['-', '—', '•', ' '])
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
}

/// Переводит англоязычные описания кадров от Vision-модели (Moondream2 / LLaVA) в живой русский визуальный контекст
fn localize_visual_description_to_ru(raw_vis: &str) -> String {
    let trimmed = raw_vis.trim();
    if trimmed.is_empty() {
        return "выразительный крупный план и резкая смена эмоций в кадре".to_string();
    }

    // Если описание уже на русском языке — возвращаем как есть
    let has_cyrillic = trimmed.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
    if has_cyrillic {
        return trimmed.to_string();
    }

    let lower = trimmed.to_lowercase();
    let mut details: Vec<&str> = Vec::new();

    if lower.contains("mask") {
        details.push("таинственная маска крупным планом");
    }
    if lower.contains("glowing") || lower.contains("yellow eyes") || lower.contains("red eyes") {
        details.push("горящий взгляд");
    }
    if lower.contains("sword") || lower.contains("blade") || lower.contains("weapon") || lower.contains("knife") {
        details.push("холодное оружие наготове");
    }
    if lower.contains("fire") || lower.contains("flame") || lower.contains("explosion") || lower.contains("smoke") {
        details.push("всполохи огня и дыма");
    }
    if lower.contains("blood") || lower.contains("wound") {
        details.push("следы жесткой схватки");
    }
    if lower.contains("fight") || lower.contains("battle") || lower.contains("attack") || lower.contains("action") {
        details.push("динамичная боевая стойка");
    }
    if lower.contains("night") || lower.contains("dark") || lower.contains("shadow") {
        details.push("мрачная ночная атмосфера");
    }
    if lower.contains("roof") || lower.contains("city") || lower.contains("street") || lower.contains("building") {
        details.push("городские крыши и улицы");
    }
    if lower.contains("shrine") || lower.contains("temple") || lower.contains("room") || lower.contains("workshop") {
        details.push("атмосферный интерьер мастерской или святилища");
    }
    if lower.contains("scream") || lower.contains("angry") || lower.contains("shock") || lower.contains("intense") {
        details.push("предельный накал эмоций на лице героя");
    }
    if lower.contains("smile") || lower.contains("smirk") || lower.contains("laugh") {
        details.push("ироничная ухмылка персонажа");
    }

    if details.is_empty() {
        "напряженная мимика персонажей и выразительная смена ракурсов".to_string()
    } else {
        details.truncate(3);
        details.join(", ")
    }
}

/// Собирает контекст для генерации SMM-пакета конкретного ролика
pub fn build_smm_input_for_clip(
    clip_path: &str,
    clip_type: &str,
    clip_index: usize,
    total_clips: usize,
    video_title: &str,
    start_sec: f64,
    end_sec: f64,
    all_subtitles: &[SubtitleFragment],
    scenes: &[crate::analyzer::SceneCandidate],
    selected_characters: Option<&[String]>,
) -> SmmGenerationInput {
    let mut inside_lines = Vec::new();
    let mut before_lines = Vec::new();
    let mut after_lines = Vec::new();

    if clip_type == "shorts" {
        for f in all_subtitles {
            if crate::whisper::is_whisper_hallucination(&f.text) {
                continue;
            }
            let clean_line = clean_smm_dialogue_line(&f.text);
            if clean_line.is_empty() {
                continue;
            }
            if f.end > start_sec + 0.08 && f.start < end_sec - 0.08 {
                inside_lines.push(clean_line);
            } else if f.end <= start_sec + 0.08 && f.end >= (start_sec - 25.0).max(0.0) {
                before_lines.push(clean_line);
            } else if f.start >= end_sec - 0.08 && f.start <= end_sec + 18.0 {
                after_lines.push(clean_line);
            }
        }
    } else if clip_type == "highlights" {
        for f in all_subtitles
            .iter()
            .filter(|f| {
                !crate::whisper::is_whisper_hallucination(&f.text)
                    && (f.text.contains('!') || f.text.contains('?'))
            })
            .take(14)
        {
            let clean_line = clean_smm_dialogue_line(&f.text);
            if !clean_line.is_empty() {
                inside_lines.push(clean_line);
            }
        }
        if inside_lines.is_empty() {
            for f in all_subtitles.iter().take(10) {
                let clean_line = clean_smm_dialogue_line(&f.text);
                if !clean_line.is_empty() {
                    inside_lines.push(clean_line);
                }
            }
        }
        before_lines.push(format!(
            "Концентрированная нарезка лучших кульминаций и сюжетных поворотов выпуска «{}»",
            video_title
        ));
    } else {
        for f in all_subtitles
            .iter()
            .filter(|f| !crate::whisper::is_whisper_hallucination(&f.text))
            .take(12)
        {
            let clean_line = clean_smm_dialogue_line(&f.text);
            if !clean_line.is_empty() {
                inside_lines.push(clean_line);
            }
        }
        if all_subtitles.len() > 14 {
            for f in all_subtitles.iter().rev().take(5).rev() {
                let clean_line = clean_smm_dialogue_line(&f.text);
                if !clean_line.is_empty() {
                    after_lines.push(clean_line);
                }
            }
        }
        before_lines.push(format!(
            "Динамичный монтаж разговорного видео «{}» без пауз и воды",
            video_title
        ));
    }

    let inside_dialogue = if inside_lines.is_empty() {
        "Динамичная сцена с яркой визуальной подачей и эмоциями в кадре".to_string()
    } else {
        inside_lines.join(" — ")
    };

    let mut context_parts = Vec::new();
    if !before_lines.is_empty() {
        let b_str: String = before_lines.join(" ").chars().take(240).collect();
        context_parts.push(format!("Что было до этого: {}", b_str));
    }
    if !after_lines.is_empty() {
        let a_str: String = after_lines.join(" ").chars().take(200).collect();
        context_parts.push(format!("Что происходит дальше: {}", a_str));
    }
    let surrounding_context = if context_parts.is_empty() {
        format!("Ключевой эпизод из видео «{}»", video_title)
    } else {
        context_parts.join(" | ")
    };

    // Визуальные действия из анализатора сцен (с автоматической локализацией на русский язык)
    let mut vis_parts = Vec::new();
    for sc in scenes {
        if (clip_type != "shorts" || (sc.timestamp_sec >= start_sec - 4.0 && sc.timestamp_sec <= end_sec + 4.0))
            && !sc.description.trim().is_empty()
        {
            let ru_vis = localize_visual_description_to_ru(&sc.description);
            if !vis_parts.contains(&ru_vis) {
                vis_parts.push(ru_vis);
            }
            if vis_parts.len() >= 2 {
                break;
            }
        }
    }
    let visual_actions = if vis_parts.is_empty() {
        "выразительный крупный план, живая мимика и резкая смена динамики кадра".to_string()
    } else {
        vis_parts.join("; ")
    };

    let mut characters = Vec::new();
    if let Some(sel) = selected_characters {
        for c in sel {
            let trimmed_c = c.trim();
            if !trimmed_c.is_empty() && trimmed_c != "Все персонажи" && trimmed_c.to_lowercase() != "all" {
                characters.push(trimmed_c.to_string());
            }
        }
    }

    SmmGenerationInput {
        clip_path: clip_path.to_string(),
        clip_type: clip_type.to_string(),
        clip_index,
        total_clips,
        video_title: video_title.to_string(),
        start_sec,
        end_sec,
        inside_dialogue,
        surrounding_context,
        visual_actions,
        characters,
    }
}

fn build_smm_copywriter_parts(input: &SmmGenerationInput) -> (String, String) {
    let format_name = match input.clip_type.as_str() {
        "highlights" => "Сборник лучших моментов (Хайлайты 16:9)",
        "talk_video" => "Разговорное видео / Демка без пауз (16:9)",
        _ => "Вертикальный ролик (Shorts / Reels / TikTok)",
    };

    let dialogue_clipped: String = input.inside_dialogue.chars().take(540).collect();
    let context_clipped: String = input.surrounding_context.chars().take(340).collect();
    let visual_clipped: String = input.visual_actions.chars().take(200).collect();

    let sys = "Ты опытный русскоязычный SMM-редактор и автор сценариев для Shorts, Reels и YouTube. \
        Пишешь строго на грамотном живом русском языке без выдуманных слов-гибридов и без дословного копирования субтитров. \
        Всегда выдаешь ровно 3 строки с префиксами НАЗВАНИЕ:, ОПИСАНИЕ:, ХЕШТЕГИ: и завершаешь ответ тегом </SMM>.";

    let usr = format!(
        "Напиши пост для публикации готового ролика #{}.\n\n\
ВХОДНЫЕ ДАННЫЕ:\n\
- Источник (тайтл и серия): «{}»\n\
- Формат: {}\n\
- Что происходит и о чём спорят герои внутри клипа:\n{}\n\
- Что происходит в кадре: {}\n\
- Контекст вокруг сцены: {}\n\n\
ПРАВИЛА:\n\
1) НАЗВАНИЕ: короткий цепляющий заголовок (4–9 слов, до 70 символов) О СУТИ КОНФЛИКТА ИЛИ ШУТКИ В ЭТОЙ СЦЕНЕ. \
НЕ переписывай название тайтла «{}» и НЕ придумывай от него искажённых прилагательных; НЕ копируй реплики субтитров дословно.\n\
2) ОПИСАНИЕ: 2–3 живых предложения на хорошем русском языке: органично вплети название «{}», кратко объясни, что происходит в моменте и почему это важно по контексту, а в конце добавь одну точную ироничную шутку про ситуацию в сцене.\n\
3) ХЕШТЕГИ: 8–10 релевантных русских тегов через пробел с символом #.\n\n\
Выведи ТОЛЬКО 3 заполненных поля без квадратных скобок и без шаблонных фраз:\n\
НАЗВАНИЕ: ...\n\
ОПИСАНИЕ: ...\n\
ХЕШТЕГИ: ...\n\
</SMM>",
        input.clip_index + 1,
        input.video_title,
        format_name,
        dialogue_clipped,
        visual_clipped,
        context_clipped,
        input.video_title,
        input.video_title
    );

    (sys.to_string(), usr)
}

fn build_smm_copywriter_prompt(input: &SmmGenerationInput) -> String {
    let (sys, usr) = build_smm_copywriter_parts(input);
    format!("{}\n\n{}\n\nНАЗВАНИЕ:", sys, usr)
}

async fn run_openai_smm(
    client: &Client,
    config: &TextAnalysisConfig,
    prompt: &str,
    _app_handle: &AppHandle,
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
                content: "Ты креативный русскоязычный SMM-копирайтер для Shorts, Reels и YouTube. Никогда не копируй субтитры дословно — пиши авторские кликабельные заголовки и остроумные описания с ироничной шуткой в конце. Строго соблюдай поля НАЗВАНИЕ:, ОПИСАНИЕ: и ХЕШТЕГИ:.".to_string(),
            },
            OpenAiMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            },
        ],
        temperature: 0.60,
        stream: false,
    };

    let mut req = client.post(&url).json(&req_body);
    let api_key = config.openai_api_key.trim();
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }

    let resp = req.send().await.map_err(|e| {
        AppError::Pipeline(format!("Ошибка запроса к OpenAI API (SMM): {}", e))
    })?;

    if !resp.status().is_success() {
        return Err(AppError::Pipeline(format!("HTTP {} от OpenAI API (SMM)", resp.status())));
    }

    let parsed: OpenAiChatResponse = resp.json().await.map_err(|e| {
        AppError::Pipeline(format!("Ошибка JSON от OpenAI API (SMM): {}", e))
    })?;

    let content = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .unwrap_or_default();

    Ok(content)
}

async fn run_llama_cli_smm(
    app_handle: &AppHandle,
    model_path: &str,
    prompt: &str,
    use_gpu: bool,
    mut cancel_rx: Receiver<()>,
) -> AppResult<String> {
    use tauri_plugin_shell::process::CommandEvent;

    let sidecar_command = app_handle
        .shell()
        .sidecar("llama-cli")
        .map_err(|e| AppError::Pipeline(format!("llama-cli не найден: {}", e)))?;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let prompt_file = std::env::temp_dir().join(format!("vc_smm_prompt_{}_{}.txt", std::process::id(), ts));
    tokio::fs::write(&prompt_file, prompt.as_bytes()).await.map_err(AppError::Io)?;

    let ngl_val = if use_gpu { "99" } else { "0" };
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().clamp(2, 8))
        .unwrap_or(4)
        .to_string();

    let mut cmd = sidecar_command.env("PATH", crate::logger::build_sidecar_env_path(app_handle));
    if !use_gpu {
        cmd = cmd
            .env("CUDA_VISIBLE_DEVICES", "")
            .env("GGML_CUDA_NO_PINNED", "1");
    }

    let cmd = cmd.args([
        "-m",
        model_path,
        "-f",
        &prompt_file.to_string_lossy(),
        "-c",
        "4096",
        "-ngl",
        ngl_val,
        "-t",
        &threads,
        "-n",
        "384",
        "-r",
        "</SMM>",
        "-r",
        "<|im_end|>",
        "-r",
        "<|end|>",
        "--temp",
        "0.42",
        "--top-p",
        "0.90",
        "--repeat-penalty",
        "1.18",
        "--no-display-prompt",
    ]);

    let (mut rx, child) = match cmd.spawn() {
        Ok(pair) => pair,
        Err(e) => {
            let _ = tokio::fs::remove_file(&prompt_file).await;
            return Err(AppError::Pipeline(format!("Не удалось запустить llama-cli для SMM: {}", e)));
        }
    };

    let model_path_owned = model_path.to_string();
    let run_future = async move {
        let mut stdout_accum = String::new();
        let mut stderr_accum = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    stdout_accum.push_str(&String::from_utf8_lossy(&bytes));
                }
                CommandEvent::Stderr(bytes) => {
                    stderr_accum.push_str(&String::from_utf8_lossy(&bytes));
                }
                CommandEvent::Terminated(payload) => {
                    let lower_err = stderr_accum.to_lowercase();
                    if lower_err.contains("unknown model architecture")
                        || lower_err.contains("error loading model")
                        || lower_err.contains("failed to load model")
                        || lower_err.contains("unable to load model")
                    {
                        crate::models::mark_model_broken(&model_path_owned);
                    }
                    if payload.code == Some(0) || !stdout_accum.trim().is_empty() {
                        return Ok(stdout_accum);
                    } else {
                        return Err(AppError::Pipeline(format!(
                            "llama-cli (SMM) завершился с кодом {:?}",
                            payload.code
                        )));
                    }
                }
                _ => {}
            }
        }
        if !stdout_accum.trim().is_empty() {
            Ok(stdout_accum)
        } else {
            Err(AppError::Pipeline("Пустой ответ от llama-cli (SMM)".to_string()))
        }
    };

    let res = tokio::select! {
        r = tokio::time::timeout(Duration::from_secs(45), run_future) => {
            match r {
                Ok(inner) => inner,
                Err(_) => {
                    let _ = child.kill();
                    Err(AppError::Pipeline("Таймаут генерации SMM (45с)".to_string()))
                }
            }
        },
        _ = cancel_rx.recv() => {
            let _ = child.kill();
            Err(AppError::Pipeline("Отменено пользователем".to_string()))
        }
    };

    let _ = tokio::fs::remove_file(&prompt_file).await;
    res
}

/// Извлекает ключевой смысловой объект/тему из текста диалога, чтобы заголовки и описания
/// были предметными, но без дословного копирования фраз субтитров
fn extract_scene_topic_label(lower_dlg: &str) -> &'static str {
    if lower_dlg.contains("доктор") || lower_dlg.contains("обезбол") || lower_dlg.contains("боль") || lower_dlg.contains("больно") || lower_dlg.contains("ран") {
        "послебоевой перевязки и упрямого характера"
    } else if lower_dlg.contains("поеду с тобой") || lower_dlg.contains("поеду с ") || lower_dlg.contains("бывайте") || lower_dlg.contains("проща") || lower_dlg.contains("отблагодар") {
        "непростого расставания и упрямого выбора пути"
    } else if lower_dlg.contains("плач") || lower_dlg.contains("слез") || lower_dlg.contains("потеря") {
        "тихой боли и искренней поддержки в трудную минуту"
    } else if lower_dlg.contains("маск") {
        "древних масок и скрытой цены силы"
    } else if lower_dlg.contains("святилищ") || lower_dlg.contains("храм") || lower_dlg.contains("алтар") {
        "тайны закрытого святилища"
    } else if lower_dlg.contains("уничтож") || lower_dlg.contains("разруш") || lower_dlg.contains("наставник") {
        "бунта против наследия наставника"
    } else if lower_dlg.contains("сдохн") || lower_dlg.contains("умр") || lower_dlg.contains("выиграть время") || lower_dlg.contains("подальше") {
        "рискованного расчёта и отчаянной попытки выстоять"
    } else if lower_dlg.contains("монстр") || lower_dlg.contains("чудовищ") || lower_dlg.contains("демон") || lower_dlg.contains("проклят") {
        "пробуждения опасного проклятия"
    } else if lower_dlg.contains("оружи") || lower_dlg.contains("меч") || lower_dlg.contains("клин") || lower_dlg.contains("удар") || lower_dlg.contains("бой") || lower_dlg.contains("атак") {
        "решающей схватки на пределе возможностей"
    } else if lower_dlg.contains("секрет") || lower_dlg.contains("тайн") || lower_dlg.contains("правд") || lower_dlg.contains("скры") {
        "раскрытия неудобной правды"
    } else if lower_dlg.contains("план") || lower_dlg.contains("ловушк") || lower_dlg.contains("хитр") {
        "многоходовки, которая пошла не по плану"
    } else if lower_dlg.contains("деньг") || lower_dlg.contains("цен") || lower_dlg.contains("плат") || lower_dlg.contains("сделк") {
        "сделки с очень высокими ставками"
    } else if lower_dlg.contains("друг") || lower_dlg.contains("преда") || lower_dlg.contains("вер") {
        "проверки союзников на прочность"
    } else {
        "неожиданного сюжетного поворота"
    }
}

/// Креативный редакторский SMM-генератор (без сырых цитат из субтитров!):
/// синтезирует кликабельное название, живое описание (название видео + что происходит внутри + контекст + ироничная шутка) и хештеги
pub fn build_deterministic_smm_copy(input: &SmmGenerationInput) -> (String, String, String) {
    let lower_dlg = input.inside_dialogue.to_lowercase().replace('ё', "е");
    let lower_ctx = input.surrounding_context.to_lowercase().replace('ё', "е");
    let has_exclam = input.inside_dialogue.contains('!');
    let has_question = input.inside_dialogue.contains('?');

    let seed = (input.clip_index * 7
        + (input.start_sec.round() as usize) * 3
        + input.inside_dialogue.chars().count())
        .max(1);

    let topic_label = extract_scene_topic_label(&lower_dlg);

    // Определяем доминирующий драматический/комедийный архетип сцены
    let is_doctor_or_pain = lower_dlg.contains("доктор")
        || lower_dlg.contains("обезбол")
        || lower_dlg.contains("невыносимо больн")
        || lower_dlg.contains("больно")
        || lower_dlg.contains("жутко")
        || lower_dlg.contains("аж смешно");
    let is_farewell_or_join = lower_dlg.contains("поеду с тобой")
        || lower_dlg.contains("поеду с ")
        || lower_dlg.contains("бывайте")
        || lower_dlg.contains("пройти мимо")
        || lower_dlg.contains("отблагодар")
        || lower_dlg.contains("вот и все");
    let is_revenge_or_destruction = lower_dlg.contains("уничтож")
        || lower_dlg.contains("разруш")
        || lower_dlg.contains("своими руками")
        || lower_dlg.contains("сожг")
        || lower_dlg.contains("слома");
    let is_cunning_wait_or_trap = lower_dlg.contains("подожд")
        || lower_dlg.contains("выиграть время")
        || lower_dlg.contains("ловушк")
        || lower_dlg.contains("план")
        || lower_dlg.contains("хитр");
    let is_mystery_or_artifact = lower_dlg.contains("маск")
        || lower_dlg.contains("святилищ")
        || lower_dlg.contains("необычн")
        || lower_dlg.contains("древн")
        || lower_dlg.contains("бога войн")
        || lower_dlg.contains("проклят");
    let is_combat_or_danger = lower_dlg.contains("атак")
        || lower_dlg.contains("бой")
        || lower_dlg.contains("огонь")
        || lower_dlg.contains("взрыв")
        || lower_dlg.contains("бежим")
        || lower_dlg.contains("монстр")
        || lower_dlg.contains("берегис");

    // 1. Креативный заголовок ролика (БЕЗ дословных цитат из сабов и БЕЗ длинных списков актеров)
    let char_badge = if input.characters.len() == 1 {
        format!(" ({})", input.characters[0])
    } else if input.characters.len() == 2 {
        format!(" ({} и {})", input.characters[0], input.characters[1])
    } else {
        String::new()
    };

    let title = match input.clip_type.as_str() {
        "highlights" => {
            let hl_titles = [
                format!("🔥 Концентрат эпика и сарказма: лучшие моменты «{}»{}", input.video_title, char_badge),
                format!("⚡ Вся суть «{}» без воды: главные кульминации и панчлайны{}", input.video_title, char_badge),
                format!("🎬 Топ-сцены «{}»: когда сюжет выкрутили на максимум{}", input.video_title, char_badge),
            ];
            hl_titles[seed % hl_titles.len()].clone()
        }
        "talk_video" => {
            if !char_badge.is_empty() {
                format!("🎙️ Харизма у микрофона{}: лучшая выжимка из «{}»", char_badge, input.video_title)
            } else {
                format!("🎙️ «{}» на максималках: чистый драйв без пауз и воды", input.video_title)
            }
        }
        _ => {
            let hook_headline = if is_doctor_or_pain {
                let variants = [
                    "Медицина бессильна, когда на анестезии решили сэкономить",
                    "Сквозь боль и чёрный юмор: цена победы оказалась кусачей",
                    "Когда вместо сочувствия от врача досталась суровая реальность",
                ];
                variants[seed % variants.len()]
            } else if is_farewell_or_join {
                let variants = [
                    "Упрямство сильнее прощаний: попутчика никто не спрашивал",
                    "Когда «нет» не принимается и в путь идут вопреки всему",
                    "Трогательный ультиматум вместо тихого прощания",
                ];
                variants[seed % variants.len()]
            } else if is_revenge_or_destruction {
                let variants = [
                    "Радикальный подход к чужим шедеврам: когда терпение лопнуло",
                    "Личный план по сносу всего вокруг: конфликт вышел из берегов",
                    "Когда решил закрыть гештальт максимально разрушительным способом",
                ];
                variants[seed % variants.len()]
            } else if is_cunning_wait_or_trap {
                let variants = [
                    "Стратегия отступления с двойным дном: время играло на него",
                    "Холодный расчёт под прикрытием: зачем он тянул паузу до последнего",
                    "План на грани фола, но сработал точнее швейцарских часов",
                ];
                variants[seed % variants.len()]
            } else if is_mystery_or_artifact {
                let variants = [
                    "Цена чужой маски: почему этот подарок пугает больше врагов",
                    "Очнулся не там и с опасным трофеем на лице",
                    "Когда реликвия выбирает носителя раньше, чем он успел подумать",
                ];
                variants[seed % variants.len()]
            } else if is_combat_or_danger {
                let variants = [
                    "Секунда до бури: когда переговоры официально зашли в тупик",
                    "Никаких прелюдий — сразу в эпицентр замеса",
                    "Момент, когда инстинкт самосохранения взял выходной",
                ];
                variants[seed % variants.len()]
            } else if has_question && has_exclam {
                let variants = [
                    "Неудобный вопрос и ответ, после которого повисла тишина",
                    "Словесная дуэль на повышенных тонах: кто кого переиграет?",
                    "Когда одна реплика переворачивает расстановку сил в сцене",
                ];
                variants[seed % variants.len()]
            } else if has_exclam {
                let variants = [
                    "Эмоции через край: момент, когда сдерживаться уже невозможно",
                    "Когда спокойный разговор за секунду превратился в ультиматум",
                    "Точка кипения: сцена, которая меняет правила игры",
                ];
                variants[seed % variants.len()]
            } else {
                let variants = [
                    "Тишина после бури: диалог, в котором каждое слово весит тонну",
                    "Тонкий расчёт и двойное дно: о чём на самом деле эта сцена",
                    "Когда в короткой паузе читается больше, чем в громких криках",
                ];
                variants[seed % variants.len()]
            };

            format!("{} | {}", hook_headline, input.video_title)
        }
    };

    // 2. Ироничная шутка по теме конкретной ситуации
    let ironic_joke = match input.clip_type.as_str() {
        "highlights" => {
            let jokes = [
                "Собрали всю концентрацию эпика в одном ролике — осторожно, при просмотре возможен передоз харизмой и сарказмом.",
                "Идеальный формат для тех, кто хочет получить весь эмоциональный спектр выпуска быстрее, чем остынет чай.",
            ];
            jokes[seed % jokes.len()]
        }
        "talk_video" => {
            let jokes = [
                "Вырезали все паузы и вздохи — теперь плотность мыслей в секунду выше, чем на защите диплома за пять минут до дедлайна.",
                "Чистый концентрат живой речи без воды: хронометраж похудел, а харизма осталась в полном объёме.",
            ];
            jokes[seed % jokes.len()]
        }
        _ => {
            if is_doctor_or_pain {
                let jokes = [
                    "Бесплатная медицина в аниме-реалиях: бинты по талонам, сочувствие за отдельную доплату.",
                    "Уровень сервиса в лазарете: «терпи, герой, обезболивающее в бюджет этой серии не завезли».",
                ];
                jokes[seed % jokes.len()]
            } else if is_farewell_or_join {
                let jokes = [
                    "Тот редкий случай, когда от одинокого волка не спасает даже скорость отъезда: попутчик уже занял место.",
                    "План «уйти по-английски» официально провалился на этапе искренней благодарности.",
                ];
                jokes[seed % jokes.len()]
            } else if is_revenge_or_destruction {
                let jokes = [
                    "Творческий кризис — это когда вместо создания нового шедевра хочется с огоньком аннулировать все предыдущие.",
                    "Уровень благодарности учителю: пять звёзд за науку и ноль шансов на уцелевший инвентарь.",
                ];
                jokes[seed % jokes.len()]
            } else if is_cunning_wait_or_trap {
                let jokes = [
                    "Мастер-класс по тайм-менеджменту: иногда главное — не бежать первым в бой, а вовремя пересчитать ходы.",
                    "Терпение и холодный расчёт творят чудеса, особенно пока остальные уверены, что ты просто исчез.",
                ];
                jokes[seed % jokes.len()]
            } else if is_mystery_or_artifact {
                let jokes = [
                    "Главное правило любого артефакта: перед примеркой уточните, прилагается ли к нему инструкция по снятию.",
                    "Эксклюзивная бижутерия с характером — носится легко, зато расплачиваться приходится нервами.",
                ];
                jokes[seed % jokes.len()]
            } else if is_combat_or_danger {
                let jokes = [
                    "План был надежный, как швейцарские часы, но реальность снова забыла прочитать сценарий.",
                    "Тот самый случай, когда инструкцию по технике безопасности решили прочитать уже в разгар спецэффектов.",
                ];
                jokes[seed % jokes.len()]
            } else {
                let jokes = [
                    "Переговоры вышли на такой уровень искренности, что даже фоновая музыка нервно притихла.",
                    "Искусство тонкой дипломатии: сказать пару фраз так, чтобы у всех вокруг начался внеплановый мозговой штурм.",
                ];
                jokes[seed % jokes.len()]
            }
        }
    };

    // 3. Авторское описание (Название видео + Что внутри + Контекст + Ироничная шутка — БЕЗ сырых цитат из субтитров!)
    let start_m = (input.start_sec / 60.0) as u32;
    let start_s = (input.start_sec % 60.0) as u32;
    let end_m = (input.end_sec / 60.0) as u32;
    let end_s = (input.end_sec % 60.0) as u32;

    let inside_action_narrative = if is_doctor_or_pain {
        format!(
            "Внутри сцены — напряжённый разговор вокруг {}, где герой пытается держаться с сарказмом, несмотря на жесткие последствия схватки",
            topic_label
        )
    } else if is_farewell_or_join {
        format!(
            "Внутри момента — эмоциональный спор вокруг {}, где герои спорят о том, кто и зачем пойдёт дальше по одному маршруту",
            topic_label
        )
    } else if is_revenge_or_destruction {
        format!(
            "Внутри сцены разворачивается жесткий конфликт вокруг {}: герой открыто идет на обострение и готов собственноручно разрушить прежние порядки",
            topic_label
        )
    } else if is_cunning_wait_or_trap {
        format!(
            "Внутри момента — холодный тактический расчёт и выдержка вокруг {}, объясняющая скрытые мотивы героя",
            topic_label
        )
    } else if is_mystery_or_artifact {
        format!(
            "В центре внимания — атмосфера загадочности вокруг {}: тревожный диалог открывает опасную сторону необычной маски",
            topic_label
        )
    } else if is_combat_or_danger {
        format!(
            "Внутри ролика — резкий скачок динамики вокруг {}, где словесное напряжение мгновенно уступает место прямому столкновению",
            topic_label
        )
    } else if has_question && has_exclam {
        format!(
            "Внутри фрагмента — острая словесная перепалка вокруг {}, где интригующий вопрос мгновенно перерастает в эмоциональную развязку",
            topic_label
        )
    } else {
        format!(
            "Внутри сцены раскрывается ключевой эпизод вокруг {}, задающий тон дальнейшему развитию событий",
            topic_label
        )
    };

    let context_narrative = if lower_ctx.contains("что было до этого") && lower_ctx.contains("что происходит дальше") {
        format!(
            "Контекст эпизода: сцена связывает предшествующие события и дальнейший поворот сюжета (в кадре — {})",
            input.visual_actions
        )
    } else {
        format!(
            "Контекст происходящего: эпизод на стыке важных сюжетных решений (в кадре — {})",
            input.visual_actions
        )
    };

    let description = match input.clip_type.as_str() {
        "highlights" => format!(
            "Динамичный сборник самых ярких сцен из выпуска «{}»{}. {}. {}. {}",
            input.video_title,
            char_badge,
            inside_action_narrative,
            context_narrative,
            ironic_joke
        ),
        "talk_video" => format!(
            "Концентрированный разговорный монтаж видео «{}»{} без заминок и пауз. {}. {}. {}",
            input.video_title,
            char_badge,
            inside_action_narrative,
            context_narrative,
            ironic_joke
        ),
        _ => format!(
            "Эпизод [{:02}:{:02}–{:02}:{:02}] из выпуска «{}»{}. {}. {}. {}",
            start_m,
            start_s,
            end_m,
            end_s,
            input.video_title,
            char_badge,
            inside_action_narrative,
            context_narrative,
            ironic_joke
        ),
    };

    // 4. Хештеги (чистые слова из названия видео + тематика + формат)
    let mut tags: Vec<String> = Vec::new();
    for word in input
        .video_title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| {
            let l = w.to_lowercase();
            w.chars().count() >= 3 && !matches!(l.as_str(), "серия" | "эпизод" | "выпуск" | "часть")
        })
        .take(4)
    {
        let capitalized: String = word
            .chars()
            .enumerate()
            .map(|(i, c)| if i == 0 { c.to_uppercase().next().unwrap_or(c) } else { c })
            .collect();
        let tag = format!("#{}", capitalized);
        if !tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
            tags.push(tag);
        }
    }

    if input.characters.len() <= 2 {
        for ch in &input.characters {
            let clean_ch: String = ch.chars().filter(|c| c.is_alphanumeric()).collect();
            if clean_ch.chars().count() >= 2 {
                let tag = format!("#{}", clean_ch);
                if !tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
                    tags.push(tag);
                }
            }
        }
    }

    let base_tags = match input.clip_type.as_str() {
        "highlights" => vec![
            "#хайлайты", "#лучшиемоменты", "#нарезка", "#топмоменты", "#юмор", "#эпик", "#аниме", "#рек",
        ],
        "talk_video" => vec![
            "#разговорноевидео", "#озвучка", "#демка", "#дубляж", "#безводы", "#диалоги", "#рек",
        ],
        _ => vec![
            "#shorts", "#шортс", "#reels", "#рилс", "#рек", "#лучшиемоменты", "#юмор", "#ирония", "#нарезка",
        ],
    };

    for bt in base_tags {
        if !tags.iter().any(|t| t.eq_ignore_ascii_case(bt)) {
            tags.push(bt.to_string());
        }
    }

    let hashtags = tags.join(" ");
    (title, description, hashtags)
}

/// Проверяет, не скопировала ли малая LLM в заголовок или описание дословную фразу из субтитров
fn is_verbatim_subtitle_copy(candidate_text: &str, inside_dialogue: &str) -> bool {
    let norm_cand: String = candidate_text
        .to_lowercase()
        .replace('ё', "е")
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();

    for cue in inside_dialogue.split(" — ") {
        let stripped = strip_speaker_prefix(cue);
        let norm_cue: String = stripped
            .to_lowercase()
            .replace('ё', "е")
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect();
        let trimmed_cue = norm_cue.trim();
        if trimmed_cue.chars().count() >= 16 && norm_cand.contains(trimmed_cue) {
            return true;
        }
    }
    false
}

fn has_leaked_smm_template(text: &str) -> bool {
    let lower = text.to_lowercase().replace('ё', "е");
    const FORBIDDEN_LEAKS: &[&str] = &[
        "креативный заголовок",
        "своими словами",
        "авторское описание",
        "ироничная шутка в финале",
        "#хештег1",
        "#хештег2",
        "#хештег3",
        "выходные данные",
        "входные данные",
        "требования к результату",
        "...",
    ];
    if FORBIDDEN_LEAKS.iter().any(|m| {
        if *m == "..." {
            lower.trim() == "..."
        } else {
            lower.contains(m)
        }
    }) {
        return true;
    }
    // Отбраковываем CJK/экзотические символы галлюцинаций токенизатора
    text.chars().any(|c| {
        ('\u{4E00}'..='\u{9FFF}').contains(&c)
            || ('\u{0600}'..='\u{06FF}').contains(&c)
            || ('\u{0900}'..='\u{097F}').contains(&c)
    })
}

/// Отсекает выдуманные малой LLM несуществующие слова-гибриды вроде «Вермильоновый эскалатор» или «Маскеры вермильоновские»
fn has_hallucinated_title_stem(candidate: &str, video_title: &str, inside_dialogue: &str) -> bool {
    let norm_src_title = video_title.to_lowercase().replace('ё', "е");
    let norm_dlg = inside_dialogue.to_lowercase().replace('ё', "е");

    let mut title_stems: Vec<String> = Vec::new();
    for raw_w in norm_src_title.split(|c: char| !c.is_alphabetic()) {
        let chs: Vec<char> = raw_w.chars().collect();
        if chs.len() >= 5 && !matches!(raw_w, "серия" | "эпизод" | "выпуск" | "часть") {
            let prefix: String = chs.iter().take(5).collect();
            title_stems.push(prefix);
        }
    }
    if title_stems.is_empty() {
        return false;
    }

    for raw_cw in candidate.to_lowercase().replace('ё', "е").split(|c: char| !c.is_alphabetic()) {
        let c_chars: Vec<char> = raw_cw.chars().collect();
        if c_chars.len() < 6 {
            continue;
        }
        let c_prefix: String = c_chars.iter().take(5).collect();
        if title_stems.iter().any(|stem| stem == &c_prefix) {
            // Если это слово из названия тайтла искажено в несуществующую форму, которой нет ни в названии видео, ни в диалоге — бракуем!
            let appears_in_title = norm_src_title
                .split(|c: char| !c.is_alphabetic())
                .any(|tw| tw == raw_cw);
            let appears_in_dlg = norm_dlg
                .split(|c: char| !c.is_alphabetic())
                .any(|dw| dw == raw_cw);
            if !appears_in_title && !appears_in_dlg {
                return true;
            }
        }
    }
    false
}

fn cyrillic_word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| {
            w.chars()
                .filter(|c| ('\u{0400}'..='\u{04FF}').contains(c))
                .count()
                >= 2
        })
        .count()
}

/// Парсит ответ LLM для SMM-задачи, защищает от шаблонных утечек, словотворчества-галлюцинаций и прямых цитат из сабов
fn parse_smm_llm_output(
    raw_output: &str,
    input: &SmmGenerationInput,
) -> (String, String, String) {
    let (fallback_title, fallback_desc, fallback_tags) = build_deterministic_smm_copy(input);

    let no_tokens = raw_output
        .replace("<|im_end|>", "")
        .replace("<|end|>", "")
        .replace("<|assistant|>", "")
        .replace("<|endoftext|>", "");

    let trimmed = no_tokens
        .trim()
        .trim_end_matches("</SMM>")
        .trim();

    if trimmed.is_empty() || trimmed.contains("GGGGGGGG") || trimmed.contains('\u{FFFD}') {
        return (fallback_title, fallback_desc, fallback_tags);
    }

    let clean_text = if let Some(pos) = trimmed.rfind("</think>") {
        trimmed[pos + "</think>".len()..].trim()
    } else {
        trimmed
    };

    let mut parsed_title = String::new();
    let mut parsed_desc = String::new();
    let mut parsed_tags = String::new();

    let mut current_section = "title";

    for raw_line in clean_text.lines() {
        let line = raw_line
            .trim()
            .trim_matches(['*', '`'])
            .trim();
        if line.is_empty() || line == "</SMM>" {
            continue;
        }

        let upper = line.to_uppercase();
        if upper.starts_with("НАЗВАНИЕ:") || upper.starts_with("ЗАГОЛОВОК:") || upper.starts_with("TITLE:") {
            current_section = "title";
            if let Some((_, val)) = line.split_once(':') {
                let v = val.trim().trim_matches(['[', ']', '"', '«', '»']);
                if !v.is_empty() {
                    parsed_title = v.to_string();
                }
            }
        } else if upper.starts_with("ОПИСАНИЕ:") || upper.starts_with("ТЕКСТ:") || upper.starts_with("DESCRIPTION:") {
            current_section = "desc";
            if let Some((_, val)) = line.split_once(':') {
                let v = val.trim().trim_matches(['[', ']']);
                if !v.is_empty() {
                    parsed_desc = v.to_string();
                }
            }
        } else if upper.starts_with("ХЕШТЕГИ:") || upper.starts_with("ХЭШТЕГИ:") || upper.starts_with("ТЕГИ:") || upper.starts_with("HASHTAGS:") {
            current_section = "tags";
            if let Some((_, val)) = line.split_once(':') {
                let v = val.trim().trim_matches(['[', ']']);
                if !v.is_empty() {
                    parsed_tags = v.to_string();
                }
            }
        } else if line.starts_with('#') && line.contains(" #") {
            parsed_tags = line.to_string();
        } else {
            match current_section {
                "title" => {
                    if parsed_title.is_empty() {
                        parsed_title = line.trim_matches(['[', ']', '"', '«', '»']).to_string();
                        current_section = "desc";
                    }
                }
                "desc" => {
                    if !parsed_desc.is_empty() {
                        parsed_desc.push(' ');
                    }
                    parsed_desc.push_str(line.trim_matches(['[', ']']));
                }
                "tags" => {
                    if !parsed_tags.is_empty() {
                        parsed_tags.push(' ');
                    }
                    parsed_tags.push_str(line.trim_matches(['[', ']']));
                }
                _ => {}
            }
        }
    }

    let title_valid = parsed_title.chars().count() >= 10
        && parsed_title.chars().count() <= 130
        && cyrillic_word_count(&parsed_title) >= 2
        && !has_leaked_smm_template(&parsed_title)
        && !has_hallucinated_title_stem(&parsed_title, &input.video_title, &input.inside_dialogue)
        && !is_verbatim_subtitle_copy(&parsed_title, &input.inside_dialogue);

    let final_title = if title_valid {
        if !parsed_title.to_lowercase().contains(&input.video_title.to_lowercase())
            && parsed_title.chars().count() <= 80
        {
            format!("{} | {}", parsed_title, input.video_title)
        } else {
            parsed_title
        }
    } else {
        fallback_title
    };

    let desc_valid = parsed_desc.chars().count() >= 45
        && cyrillic_word_count(&parsed_desc) >= 6
        && !has_leaked_smm_template(&parsed_desc)
        && !has_hallucinated_title_stem(&parsed_desc, &input.video_title, &input.inside_dialogue)
        && !is_verbatim_subtitle_copy(&parsed_desc, &input.inside_dialogue);

    let final_desc = if desc_valid {
        parsed_desc
    } else {
        fallback_desc
    };

    let tags_valid = parsed_tags.contains('#')
        && parsed_tags.chars().count() >= 8
        && !has_leaked_smm_template(&parsed_tags);

    let final_tags = if tags_valid {
        let mut clean_tags: Vec<String> = Vec::new();
        for w in parsed_tags.split_whitespace() {
            let clean = w.trim_matches([',', ';', '.']);
            if clean.chars().count() < 2 {
                continue;
            }
            let normalized = if clean.starts_with('#') {
                clean.to_string()
            } else {
                format!("#{}", clean)
            };
            if !clean_tags.iter().any(|t| t.eq_ignore_ascii_case(&normalized)) {
                clean_tags.push(normalized);
            }
        }
        if clean_tags.len() >= 3 {
            clean_tags.join(" ")
        } else {
            fallback_tags
        }
    } else {
        fallback_tags
    };

    (final_title, final_desc, final_tags)
}

/// Форматирует готовый текст для `.txt` файла и для копирования в буфер обмена
pub fn format_smm_txt_file_content(
    meta: &SmmClipMetadata,
    video_title: &str,
) -> String {
    let start_m = (meta.start_sec / 60.0) as u32;
    let start_s = (meta.start_sec % 60.0) as u32;
    let end_m = (meta.end_sec / 60.0) as u32;
    let end_s = (meta.end_sec % 60.0) as u32;

    format!(
        "================================================================\r\n\
🎬 ГОТОВЫЙ ПОСТ ДЛЯ ВЫКЛАДКИ (СКОПИРУЙТЕ ЦЕЛИКОМ):\r\n\
================================================================\r\n\
{}\r\n\
\r\n\
{}\r\n\
\r\n\
{}\r\n\
\r\n\
----------------------------------------------------------------\r\n\
📌 РАЗДЕЛЫ ПО ОТДЕЛЬНОСТИ:\r\n\
----------------------------------------------------------------\r\n\
[НАЗВАНИЕ РОЛИКА]:\r\n\
{}\r\n\
\r\n\
[ОПИСАНИЕ РОЛИКА (С КОНТЕКСТОМ И ИРОНИЕЙ)]:\r\n\
{}\r\n\
\r\n\
[ХЕШТЕГИ]:\r\n\
{}\r\n\
\r\n\
----------------------------------------------------------------\r\n\
Исходное видео: {}\r\n\
Файл ролика: {}\r\n\
Таймкод: {:02}:{:02} - {:02}:{:02}\r\n\
Модель SMM: {}\r\n\
================================================================\r\n",
        meta.title,
        meta.description,
        meta.hashtags,
        meta.title,
        meta.description,
        meta.hashtags,
        video_title,
        meta.clip_path,
        start_m,
        start_s,
        end_m,
        end_s,
        meta.model_used
    )
}

/// Генерирует SMM-метаданные для одного клипа/ролика через выбранную модель (и сохраняет `.txt` файл рядом с видео)
pub async fn generate_smm_for_single_clip(
    input: &SmmGenerationInput,
    smm_config: &TextAnalysisConfig,
    app_handle: &AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<SmmClipMetadata> {
    if cancel_rx.try_recv().is_ok() {
        return Err(AppError::Pipeline("Отменено пользователем".into()));
    }

    let (smm_sys, smm_usr) = build_smm_copywriter_parts(input);
    let plain_prompt = build_smm_copywriter_prompt(input);
    let mut llm_raw_opt: Option<String> = None;
    let mut model_label = match smm_config.engine {
        TextEngine::Builtin => "Встроенный SMM-автор (RU)".to_string(),
        _ => smm_config.model_name.clone(),
    };

    if smm_config.engine != TextEngine::Builtin {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(25))
            .build()
            .unwrap_or_default();

        if smm_config.engine == TextEngine::OpenAi {
            if let Ok(resp) = run_openai_smm(&http_client, smm_config, &plain_prompt, app_handle).await {
                llm_raw_opt = Some(resp);
                model_label = format!("OpenAI ({})", smm_config.model_name);
            }
        } else if smm_config.engine == TextEngine::Ollama {
            if let Ok(resp) = run_ollama(&http_client, smm_config, &plain_prompt, app_handle).await {
                llm_raw_opt = Some(resp);
                model_label = format!("Ollama ({})", smm_config.model_name);
            }
        }

        if llm_raw_opt.is_none() {
            let mut candidate_paths: Vec<std::path::PathBuf> = Vec::new();
            if let Some(p1) = crate::models::resolve_russian_verifier_llm_model(
                app_handle,
                Some(&smm_config.model_name),
            ) {
                candidate_paths.push(p1);
            }
            if let Some(p2) = crate::models::resolve_any_llm_model(app_handle, None) {
                if !candidate_paths.iter().any(|p| p == &p2) {
                    candidate_paths.push(p2);
                }
            }

            for gguf_path in candidate_paths {
                let gguf_str = gguf_path.to_string_lossy().to_string();
                if is_model_marked_broken(&gguf_str) {
                    continue;
                }
                let fname = gguf_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Local-GGUF")
                    .to_string();

                let gguf_prompt = format_prompt_for_gguf_model(&gguf_str, &smm_sys, &smm_usr, "НАЗВАНИЕ:");

                if let Ok(out) = run_llama_cli_smm(
                    app_handle,
                    &gguf_str,
                    &gguf_prompt,
                    true,
                    cancel_rx.resubscribe(),
                )
                .await
                {
                    llm_raw_opt = Some(out);
                    model_label = fname;
                    break;
                } else if !is_model_marked_broken(&gguf_str) {
                    if let Ok(cpu_out) = run_llama_cli_smm(
                        app_handle,
                        &gguf_str,
                        &gguf_prompt,
                        false,
                        cancel_rx.resubscribe(),
                    )
                    .await
                    {
                        llm_raw_opt = Some(cpu_out);
                        model_label = format!("{} (CPU)", fname);
                        break;
                    }
                }
            }
        }
    }

    let (title, description, hashtags) = if let Some(ref raw) = llm_raw_opt {
        parse_smm_llm_output(raw, input)
    } else {
        build_deterministic_smm_copy(input)
    };

    let full_copy_text = format!("{}\n\n{}\n\n{}", title, description, hashtags);

    // Определяем путь к парному .txt файлу в той же папке, где лежит видеофайл
    let clip_path_buf = std::path::PathBuf::from(&input.clip_path);
    let txt_path_buf = clip_path_buf.with_extension("txt");
    let txt_path_str = txt_path_buf.to_string_lossy().to_string();

    let meta = SmmClipMetadata {
        clip_path: input.clip_path.clone(),
        txt_path: txt_path_str,
        clip_type: input.clip_type.clone(),
        title,
        description,
        hashtags,
        full_copy_text,
        model_used: model_label,
        start_sec: input.start_sec,
        end_sec: input.end_sec,
    };

    let txt_content = format_smm_txt_file_content(&meta, &input.video_title);
    if let Some(parent) = txt_path_buf.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(&txt_path_buf, txt_content.as_bytes()).await;

    Ok(meta)
}

/// Пакетная генерация SMM-описаний для всех экспортированных роликов (Шортсы, Хайлайты, Разговорное видео)
/// с автоматическим сохранением индивидуальных `.txt` файлов и сводных файлов по папкам
pub async fn generate_and_save_smm_bundle(
    video_path: &std::path::Path,
    clips_root: &std::path::Path,
    exported_items: &[(std::path::PathBuf, f64, f64, String)],
    all_subtitles: &[SubtitleFragment],
    scenes: &[crate::analyzer::SceneCandidate],
    selected_characters: Option<&[String]>,
    smm_config: &TextAnalysisConfig,
    app_handle: &AppHandle,
    mut cancel_rx: Receiver<()>,
) -> Vec<SmmClipMetadata> {
    if exported_items.is_empty() {
        return Vec::new();
    }

    let raw_stem = video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Видео");
    let clean_title = clean_video_title_for_smm(raw_stem);

    crate::logger::emit_log(
        app_handle,
        "LLM",
        "info",
        &format!(
            "✍️ [Финальный SMM-копирайтер] Генерация названий, ироничных описаний и хештегов для {} готовых роликов (модель: '{}')...",
            exported_items.len(),
            smm_config.model_name
        ),
    );

    let mut results: Vec<SmmClipMetadata> = Vec::with_capacity(exported_items.len());
    let total = exported_items.len();

    for (idx, (clip_path, start_s, end_s, clip_type)) in exported_items.iter().enumerate() {
        if cancel_rx.try_recv().is_ok() {
            break;
        }
        let clip_str = clip_path.to_string_lossy().to_string();
        let input = build_smm_input_for_clip(
            &clip_str,
            clip_type,
            idx,
            total,
            &clean_title,
            *start_s,
            *end_s,
            all_subtitles,
            scenes,
            selected_characters,
        );

        match generate_smm_for_single_clip(&input, smm_config, app_handle, cancel_rx.resubscribe()).await {
            Ok(meta) => {
                crate::logger::emit_log(
                    app_handle,
                    "LLM",
                    "info",
                    &format!(
                        "  ✓ [SMM #{}/{}] «{}» → сохранен текстовый файл '{}'",
                        idx + 1,
                        total,
                        meta.title,
                        meta.txt_path
                    ),
                );
                results.push(meta);
            }
            Err(e) => {
                crate::logger::emit_log(
                    app_handle,
                    "LLM",
                    "warn",
                    &format!("Предупреждение SMM для ролика #{}: {}", idx + 1, e),
                );
            }
        }
    }

    // Дополнительно сохраняем сводные текстовые файлы в папки Шортсы/, Хайлайты/, Ток-видео/ и в корень Готовые клипы/
    if !results.is_empty() {
        let mut all_summary = format!(
            "====================================================================\r\n\
📦 СВОДНЫЙ SMM-ПАКЕТ ДЛЯ ВЫКЛАДКИ ВСЕХ РОЛИКОВ\r\n\
Исходное видео: {}\r\n\
Всего роликов: {}\r\n\
====================================================================\r\n\r\n",
            clean_title,
            results.len()
        );

        let mut shorts_summary = format!(
            "====================================================================\r\n\
📱 SMM-ОПИСАНИЯ ДЛЯ ВСЕХ ШОРТСОВ ({})\r\n\
====================================================================\r\n\r\n",
            clean_title
        );
        let mut has_shorts = false;

        for (i, m) in results.iter().enumerate() {
            let block = format!(
                "--------------------------------------------------------------------\r\n\
РОЛИК #{:02} ({}) — Файл: {}\r\n\
--------------------------------------------------------------------\r\n\
НАЗВАНИЕ:\r\n{}\r\n\r\n\
ОПИСАНИЕ:\r\n{}\r\n\r\n\
ХЕШТЕГИ:\r\n{}\r\n\r\n",
                i + 1,
                m.clip_type.to_uppercase(),
                m.clip_path,
                m.title,
                m.description,
                m.hashtags
            );
            all_summary.push_str(&block);
            if m.clip_type == "shorts" {
                has_shorts = true;
                shorts_summary.push_str(&block);
            }
        }

        let _ = tokio::fs::create_dir_all(clips_root).await;
        let _ = tokio::fs::write(
            clips_root.join("SMM_Все_описания_и_хештеги.txt"),
            all_summary.as_bytes(),
        )
        .await;

        if has_shorts {
            let shorts_dir = clips_root.join("Шортсы");
            let _ = tokio::fs::create_dir_all(&shorts_dir).await;
            let _ = tokio::fs::write(
                shorts_dir.join("SMM_Описания_всех_шортсов.txt"),
                shorts_summary.as_bytes(),
            )
            .await;
        }

        // Сохраняем JSON-кэш SMM-метаданных для быстрой подгрузки в UI
        if let Ok(json_str) = serde_json::to_string_pretty(&results) {
            let _ = tokio::fs::write(clips_root.join("smm_metadata.json"), json_str.as_bytes()).await;
        }
    }

    results
}



