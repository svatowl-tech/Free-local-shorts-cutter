// src-tauri/src/subtitles_parser.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleCharacterInfo {
    pub name: String,
    pub line_count: usize,
    #[serde(default)]
    pub total_duration_sec: f64,
    #[serde(default)]
    pub sample_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedSubtitleLine {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub character: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomSubtitlesParseResult {
    pub file_path: String,
    pub total_lines: usize,
    pub characters: Vec<SubtitleCharacterInfo>,
    pub lines: Vec<ParsedSubtitleLine>,
}

/// Парсит время в формате ASS/SSA: `H:MM:SS.cs` (например `0:01:20.10` -> `80.10`)
fn parse_ass_time(time_str: &str) -> f64 {
    let normalized = time_str.trim().replace(',', ".");
    let parts: Vec<&str> = normalized.split(':').collect();
    if parts.len() == 3 {
        let h: f64 = parts[0].parse().unwrap_or(0.0);
        let m: f64 = parts[1].parse().unwrap_or(0.0);
        let s: f64 = parts[2].parse().unwrap_or(0.0);
        h * 3600.0 + m * 60.0 + s
    } else if parts.len() == 2 {
        let m: f64 = parts[0].parse().unwrap_or(0.0);
        let s: f64 = parts[1].parse().unwrap_or(0.0);
        m * 60.0 + s
    } else {
        0.0
    }
}

/// Удаляет ASS-теги форматирования вида `{\an8}`, `{\b1}`, `{\pos(100,200)}` и переносы `\N`
fn clean_ass_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_tag = false;

    for ch in text.chars() {
        if ch == '{' {
            in_tag = true;
        } else if ch == '}' {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }

    result
        .replace("\\N", " ")
        .replace("\\n", " ")
        .replace("\\h", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Выделяет имя персонажа из текста реплики в формате SRT/VTT (`Окане: ...`, `[Окане] ...`, `(Окане) ...`, `<v Окане>...`)
pub fn extract_srt_character_prefix(text: &str) -> (String, String) {
    let trimmed = text.trim();

    // 0. Формат WebVTT `<v Имя>Текст`
    if let Some(rest) = trimmed.strip_prefix("<v ") {
        if let Some(close_idx) = rest.find('>') {
            let name = rest[..close_idx].trim().to_string();
            let body = rest[close_idx + 1..].replace("</v>", "").trim().to_string();
            if !name.is_empty() && name.chars().count() <= 30 && !body.is_empty() {
                return (name, body);
            }
        }
    }

    // 1. Формат `[Имя] Текст` или `(Имя) Текст`
    if (trimmed.starts_with('[') && trimmed.contains(']'))
        || (trimmed.starts_with('(') && trimmed.contains(')'))
    {
        let closing_char = if trimmed.starts_with('[') { ']' } else { ')' };
        if let Some(close_idx) = trimmed.find(closing_char) {
            let name = trimmed[1..close_idx].trim().to_string();
            let body = trimmed[close_idx + 1..].trim_start_matches([':', '-', '—', ' ']).trim().to_string();
            if !name.is_empty() && name.chars().count() <= 28 && !body.is_empty() {
                return (name, body);
            }
        }
    }

    // 2. Формат `Имя: Текст` или `Имя — Текст`
    if let Some((name_part, body_part)) = trimmed.split_once(':') {
        let name = name_part.trim().trim_start_matches(['-', '—', ' ']).trim();
        let body = body_part.trim();
        let words_cnt = name.split_whitespace().count();
        if !name.is_empty()
            && (1..=3).contains(&words_cnt)
            && name.chars().count() <= 28
            && !name.chars().all(|c| c.is_ascii_digit())
            && !body.is_empty()
        {
            return (name.to_string(), body.to_string());
        }
    } else if let Some((name_part, body_part)) = trimmed.split_once(" — ") {
        let name = name_part.trim();
        let body = body_part.trim();
        let words_cnt = name.split_whitespace().count();
        if !name.is_empty()
            && (1..=3).contains(&words_cnt)
            && name.chars().count() <= 28
            && !body.is_empty()
        {
            return (name.to_string(), body.to_string());
        }
    }

    ("Общий диктор".to_string(), trimmed.to_string())
}

/// Парсинг субтитров в формате `.sub` (MicroDVD `{start_frame}{end_frame}Text` или SubViewer)
fn parse_sub_format(content: &str) -> Vec<ParsedSubtitleLine> {
    let mut lines = Vec::new();
    let fps = 25.0;

    for raw in content.lines() {
        let trimmed = raw.trim();
        if trimmed.starts_with('{') {
            let parts: Vec<&str> = trimmed.splitn(3, '}').collect();
            if parts.len() == 3 {
                let s_frame_str = parts[0].trim_start_matches('{').trim();
                let e_frame_str = parts[1].trim_start_matches('{').trim();
                if let (Ok(sf), Ok(ef)) = (s_frame_str.parse::<f64>(), e_frame_str.parse::<f64>()) {
                    let start = sf / fps;
                    let end = (ef / fps).max(start + 0.4);
                    let clean_txt = parts[2].replace('|', " ").trim().to_string();
                    if !clean_txt.is_empty() {
                        let (char_name, body) = extract_srt_character_prefix(&clean_txt);
                        lines.push(ParsedSubtitleLine {
                            start,
                            end,
                            text: body,
                            character: char_name,
                        });
                    }
                }
            }
        }
    }
    lines
}

/// Парсинг содержимого субтитров (ASS, SSA, SRT, VTT, SUB) из строки
pub fn parse_custom_subtitles_from_str(
    file_path_label: &str,
    ext_lower: &str,
    content: &str,
) -> Result<CustomSubtitlesParseResult, String> {
    let clean_content = content.trim_start_matches('\u{feff}');
    let mut parsed_lines: Vec<ParsedSubtitleLine> = Vec::new();

    let is_ass = ext_lower == "ass"
        || ext_lower == "ssa"
        || clean_content.contains("[Events]")
        || clean_content.contains("\nDialogue:");

    if is_ass {
        let mut in_events = false;
        let mut format_cols: Vec<String> = Vec::new();

        for line in clean_content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with(';') {
                continue;
            }

            let lower = trimmed.to_lowercase();
            if lower.starts_with("[events]") {
                in_events = true;
                continue;
            }

            if lower.starts_with('[') && lower.ends_with(']') && !lower.starts_with("[events]") {
                in_events = false;
                continue;
            }

            if in_events || trimmed.starts_with("Dialogue:") {
                if trimmed.starts_with("Format:") {
                    if let Some((_, cols_str)) = trimmed.split_once(':') {
                        format_cols = cols_str
                            .split(',')
                            .map(|s| s.trim().to_lowercase())
                            .collect();
                    }
                } else if trimmed.starts_with("Dialogue:") {
                    if let Some((_, vals_str)) = trimmed.split_once(':') {
                        let col_count = if !format_cols.is_empty() {
                            format_cols.len()
                        } else {
                            10
                        };
                        let parts: Vec<&str> = vals_str.splitn(col_count, ',').collect();

                        let get_col = |name: &str| -> Option<&str> {
                            format_cols
                                .iter()
                                .position(|c| c == name)
                                .and_then(|idx| parts.get(idx).copied())
                        };

                        let start_str = get_col("start")
                            .or_else(|| parts.get(1).copied())
                            .unwrap_or("0:00:00.00");
                        let end_str = get_col("end")
                            .or_else(|| parts.get(2).copied())
                            .unwrap_or("0:00:00.00");
                        let style_str = get_col("style")
                            .or_else(|| parts.get(3).copied())
                            .unwrap_or("Default")
                            .trim();
                        let actor_str = get_col("name")
                            .or_else(|| get_col("actor"))
                            .or_else(|| parts.get(4).copied())
                            .unwrap_or("")
                            .trim();
                        let raw_text = get_col("text")
                            .or_else(|| parts.last().copied())
                            .unwrap_or("")
                            .trim();

                        let start = parse_ass_time(start_str);
                        let end = parse_ass_time(end_str);
                        let clean_text = clean_ass_text(raw_text);

                        if !clean_text.is_empty() && end > start {
                            // Игнорируем чисто оформительские дорожки вывесок/надписей, если они помечены как Sign/Typeset
                            let style_low = style_str.to_lowercase();
                            let actor_low = actor_str.to_lowercase();
                            if matches!(
                                style_low.as_str(),
                                "sign" | "signs" | "typeset" | "надписи" | "вывески" | "karaoke" | "op" | "ed"
                            ) || matches!(actor_low.as_str(), "sign" | "signs" | "надпись" | "вывеска")
                            {
                                continue;
                            }

                            let (char_name, final_body) = if !actor_str.is_empty() {
                                let (_, stripped) = extract_srt_character_prefix(&clean_text);
                                (actor_str.to_string(), stripped)
                            } else if !style_str.is_empty()
                                && !matches!(
                                    style_low.as_str(),
                                    "default"
                                        | "normal"
                                        | "main"
                                        | "alt"
                                        | "title"
                                        | "top"
                                        | "bottom"
                                        | "стандарт"
                                        | "основной"
                                        | "dialog"
                                        | "dialogue"
                                )
                            {
                                let (_, stripped) = extract_srt_character_prefix(&clean_text);
                                (style_str.to_string(), stripped)
                            } else {
                                extract_srt_character_prefix(&clean_text)
                            };

                            parsed_lines.push(ParsedSubtitleLine {
                                start,
                                end,
                                text: final_body,
                                character: char_name,
                            });
                        }
                    }
                }
            }
        }
    } else if ext_lower == "sub" && clean_content.trim_start().starts_with('{') {
        parsed_lines = parse_sub_format(clean_content);
    } else {
        // Парсинг SRT / VTT формата
        let fragments = crate::text_analyzer::parse_srt(clean_content);
        for frag in fragments {
            let clean_raw = clean_ass_text(&frag.text);
            if clean_raw.is_empty() {
                continue;
            }
            let (char_name, clean_txt) = extract_srt_character_prefix(&clean_raw);
            parsed_lines.push(ParsedSubtitleLine {
                start: frag.start,
                end: frag.end,
                text: clean_txt,
                character: char_name,
            });
        }
    }

    // Сортируем реплики по времени начала
    parsed_lines.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));

    // Подсчет статистики по каждому персонажу (кол-во реплик, суммарный хронометраж, пример фразы)
    let mut char_stats: HashMap<String, (usize, f64, String)> = HashMap::new();
    for line in &parsed_lines {
        let dur = (line.end - line.start).clamp(0.2, 25.0);
        let entry = char_stats
            .entry(line.character.clone())
            .or_insert_with(|| (0, 0.0, String::new()));
        entry.0 += 1;
        entry.1 += dur;
        if entry.2.is_empty() && line.text.chars().count() >= 6 {
            entry.2 = line.text.chars().take(70).collect();
        }
    }

    let mut characters: Vec<SubtitleCharacterInfo> = char_stats
        .into_iter()
        .map(|(name, (line_count, total_duration_sec, sample_text))| SubtitleCharacterInfo {
            name,
            line_count,
            total_duration_sec: (total_duration_sec * 10.0).round() / 10.0,
            sample_text,
        })
        .collect();

    // Сортировка персонажей по убыванию количества реплик
    characters.sort_by(|a, b| {
        b.line_count
            .cmp(&a.line_count)
            .then_with(|| b.total_duration_sec.partial_cmp(&a.total_duration_sec).unwrap_or(std::cmp::Ordering::Equal))
    });

    let total_lines = parsed_lines.len();

    Ok(CustomSubtitlesParseResult {
        file_path: file_path_label.to_string(),
        total_lines,
        characters,
        lines: parsed_lines,
    })
}

/// Умный парсер файлов субтитров (ASS, SSA, SRT, VTT, SUB) с вычленением персонажей и актеров
pub fn parse_custom_subtitles_file(file_path: &Path) -> Result<CustomSubtitlesParseResult, String> {
    let bytes = std::fs::read(file_path)
        .map_err(|e| format!("Не удалось прочитать файл субтитров: {}", e))?;
    let content = String::from_utf8_lossy(&bytes);

    let file_str = file_path.to_string_lossy().to_string();
    let ext = file_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    parse_custom_subtitles_from_str(&file_str, &ext, &content)
}
