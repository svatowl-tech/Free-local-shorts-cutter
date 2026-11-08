// src-tauri/src/subtitles_parser.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleCharacterInfo {
    pub name: String,
    pub line_count: usize,
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
        .trim()
        .to_string()
}

/// Выделяет имя персонажа из текста реплики в формате SRT/VTT (`Окане: ...`, `[Окане] ...`, `(Окане) ...`)
fn extract_srt_character_prefix(text: &str) -> (String, String) {
    let trimmed = text.trim();

    // 1. Формат `[Имя] Текст` или `(Имя) Текст`
    if (trimmed.starts_with('[') && trimmed.contains(']')) || (trimmed.starts_with('(') && trimmed.contains(')')) {
        let closing_char = if trimmed.starts_with('[') { ']' } else { ')' };
        if let Some(close_idx) = trimmed.find(closing_char) {
            let name = trimmed[1..close_idx].trim().to_string();
            let body = trimmed[close_idx + 1..].trim().to_string();
            if !name.is_empty() && name.chars().count() <= 25 && !body.is_empty() {
                return (name, body);
            }
        }
    }

    // 2. Формат `Имя: Текст` или `Имя — Текст`
    if let Some((name_part, body_part)) = trimmed.split_once(':') {
        let name = name_part.trim();
        let body = body_part.trim();
        let words_cnt = name.split_whitespace().count();
        if !name.is_empty() && words_cnt >= 1 && words_cnt <= 3 && name.chars().count() <= 25 && !body.is_empty() {
            return (name.to_string(), body.to_string());
        }
    } else if let Some((name_part, body_part)) = trimmed.split_once(" — ") {
        let name = name_part.trim();
        let body = body_part.trim();
        let words_cnt = name.split_whitespace().count();
        if !name.is_empty() && words_cnt >= 1 && words_cnt <= 3 && name.chars().count() <= 25 && !body.is_empty() {
            return (name.to_string(), body.to_string());
        }
    }

    ("Общий диктор".to_string(), trimmed.to_string())
}

/// Умный парсер файлов субтитров (ASS, SSA, SRT, VTT) с вычленением персонажей и актеров
pub fn parse_custom_subtitles_file(file_path: &Path) -> Result<CustomSubtitlesParseResult, String> {
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| format!("Не удалось прочитать файл субтитров: {}", e))?;

    let file_str = file_path.to_string_lossy().to_string();
    let ext = file_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mut parsed_lines: Vec<ParsedSubtitleLine> = Vec::new();

    if ext == "ass" || ext == "ssa" {
        // Парсинг ASS / SSA формата
        let mut in_events = false;
        let mut format_cols: Vec<String> = Vec::new();

        for line in content.lines() {
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

            if in_events {
                if trimmed.starts_with("Format:") {
                    if let Some((_, cols_str)) = trimmed.split_once(':') {
                        format_cols = cols_str
                            .split(',')
                            .map(|s| s.trim().to_lowercase())
                            .collect();
                    }
                } else if trimmed.starts_with("Dialogue:") || trimmed.starts_with("Comment:") {
                    if let Some((_, vals_str)) = trimmed.split_once(':') {
                        let parts: Vec<&str> = if !format_cols.is_empty() {
                            vals_str.splitn(format_cols.len(), ',').collect()
                        } else {
                            vals_str.split(',').collect()
                        };

                        let get_col = |name: &str| -> Option<&str> {
                            format_cols
                                .iter()
                                .position(|c| c == name)
                                .and_then(|idx| parts.get(idx).copied())
                        };

                        let start_str = get_col("start").or_else(|| parts.get(1).copied()).unwrap_or("0:00:00.00");
                        let end_str = get_col("end").or_else(|| parts.get(2).copied()).unwrap_or("0:00:00.00");
                        let style_str = get_col("style").or_else(|| parts.get(3).copied()).unwrap_or("Default").trim();
                        let actor_str = get_col("name")
                            .or_else(|| get_col("actor"))
                            .or_else(|| parts.get(4).copied())
                            .unwrap_or("")
                            .trim();
                        let raw_text = get_col("text").or_else(|| parts.last().copied()).unwrap_or("").trim();

                        let start = parse_ass_time(start_str);
                        let end = parse_ass_time(end_str);
                        let clean_text = clean_ass_text(raw_text);

                        if !clean_text.is_empty() && end > start {
                            // Определение персонажа: сначала проверяем по полю Actor/Name, затем по Style
                            let char_name = if !actor_str.is_empty() {
                                actor_str.to_string()
                            } else if !style_str.is_empty()
                                && !matches!(
                                    style_str.to_lowercase().as_str(),
                                    "default" | "normal" | "main" | "alt" | "sign" | "title" | "top" | "bottom" | "стандарт"
                                )
                            {
                                style_str.to_string()
                            } else {
                                let (extracted, _) = extract_srt_character_prefix(&clean_text);
                                extracted
                            };

                            parsed_lines.push(ParsedSubtitleLine {
                                start,
                                end,
                                text: clean_text,
                                character: char_name,
                            });
                        }
                    }
                }
            }
        }
    } else {
        // Парсинг SRT / VTT формата
        let fragments = crate::text_analyzer::parse_srt(&content);
        for frag in fragments {
            let (char_name, clean_txt) = extract_srt_character_prefix(&frag.text);
            parsed_lines.push(ParsedSubtitleLine {
                start: frag.start,
                end: frag.end,
                text: clean_txt,
                character: char_name,
            });
        }
    }

    // Подсчет статистики реплик по персонажам
    let mut char_counts: HashMap<String, usize> = HashMap::new();
    for line in &parsed_lines {
        *char_counts.entry(line.character.clone()).or_insert(0) += 1;
    }

    let mut characters: Vec<SubtitleCharacterInfo> = char_counts
        .into_iter()
        .map(|(name, count)| SubtitleCharacterInfo { name, line_count: count })
        .collect();

    // Сортировка персонажей по убыванию количества реплик
    characters.sort_by(|a, b| b.line_count.cmp(&a.line_count));

    let total_lines = parsed_lines.len();

    Ok(CustomSubtitlesParseResult {
        file_path: file_str,
        total_lines,
        characters,
        lines: parsed_lines,
    })
}
