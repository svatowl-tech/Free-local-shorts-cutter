// src-tauri/src/audio_analyzer.rs
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandEvent;
use tokio::sync::broadcast::Receiver;
use crate::error::{AppError, AppResult};
use crate::text_analyzer::SubtitleFragment;

/// Профиль звуковой энергии видеодорожки с высоким разрешением (шаг 20 мс = 50 отсчётов/сек)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AudioEnergyProfile {
    pub step_sec: f64,
    pub duration_sec: f64,
    pub rms_db: Vec<f32>,
    pub peak_db: Vec<f32>,
    pub zcr: Vec<f32>,
    pub noise_floor_db: f32,
    pub quiet_valley_db: f32,
    pub speech_threshold_db: f32,
    pub loud_peak_db: f32,
    pub op_ed_zones: Vec<(f64, f64)>,
}

/// Статистика выравнивания границ клипов по тихим местам
#[derive(Clone, Debug, Default)]
pub struct BoundaryRefinementStats {
    pub total_clips: usize,
    pub adjusted_clips: usize,
    pub rescued_speech_starts: usize,
    pub rescued_speech_ends: usize,
    pub max_shift_sec: f64,
}

impl AudioEnergyProfile {
    /// Возвращает индекс 20-мс фрейма для заданной секунды
    #[inline]
    pub fn time_to_idx(&self, t_sec: f64) -> usize {
        if self.rms_db.is_empty() || t_sec <= 0.0 {
            return 0;
        }
        let idx = (t_sec / self.step_sec).round() as usize;
        idx.min(self.rms_db.len().saturating_sub(1))
    }

    /// Возвращает время в секундах для индекса фрейма
    #[inline]
    pub fn idx_to_time(&self, idx: usize) -> f64 {
        (idx as f64) * self.step_sec
    }

    /// Проверяет, попадает ли временной интервал `[start, end]` в зону опенинга или эндинга
    pub fn overlaps_op_ed(&self, start: f64, end: f64) -> bool {
        for &(z_start, z_end) in &self.op_ed_zones {
            let overlap_start = start.max(z_start);
            let overlap_end = end.min(z_end);
            if overlap_end > overlap_start {
                let overlap_len = overlap_end - overlap_start;
                let clip_len = (end - start).max(0.1);
                if overlap_len >= 2.5 || overlap_len / clip_len >= 0.30 {
                    return true;
                }
            }
        }
        false
    }

    /// Сглаженный уровень RMS dBFS в окрестности кадра `idx` (по 3 соседним отсчётам = 60 мс)
    #[inline]
    pub fn smoothed_rms_at(&self, idx: usize) -> f32 {
        if self.rms_db.is_empty() {
            return -90.0;
        }
        let i0 = idx.saturating_sub(1);
        let i1 = (idx + 1).min(self.rms_db.len() - 1);
        let mut sum = 0.0f32;
        let mut cnt = 0usize;
        for k in i0..=i1 {
            sum += self.rms_db[k];
            cnt += 1;
        }
        sum / (cnt.max(1) as f32)
    }
}

/// Возвращает путь к кэшированному аудио-профилю в папке проекта `<Видео>_Проект/<Видео>.audio_profile.json`
pub fn get_project_audio_profile_path(video_path: &Path) -> PathBuf {
    let parent = video_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    parent
        .join(format!("{}_Проект", stem))
        .join(format!("{}.audio_profile.json", stem))
}

/// Извлекает или загружает из кэша проекта 20-мс профиль звуковой энергии всей видеодорожки
pub async fn get_or_create_audio_profile(
    video_path: &Path,
    fragments: &[SubtitleFragment],
    app_handle: &AppHandle,
    mut cancel_rx: Receiver<()>,
) -> AppResult<AudioEnergyProfile> {
    let profile_path = get_project_audio_profile_path(video_path);

    if profile_path.exists() {
        if let Ok(content) = tokio::fs::read_to_string(&profile_path).await {
            if let Ok(mut cached) = serde_json::from_str::<AudioEnergyProfile>(&content) {
                if cached.rms_db.len() > 50 {
                    // Обновляем зоны OP/ED с учетом актуальных очищенных субтитров, если они переданы
                    if !fragments.is_empty() {
                        cached.op_ed_zones = detect_op_ed_zones(&cached, fragments);
                    }
                    crate::logger::emit_log(
                        app_handle,
                        "Audio",
                        "info",
                        &format!(
                            "✓ Загружен кэшированный профиль аудио-энергии ({} отсчётов с шагом 20 мс, порог тишины {:.1} dB, речь {:.1} dB, пик {:.1} dB, зон OP/ED: {})",
                            cached.rms_db.len(),
                            cached.quiet_valley_db,
                            cached.speech_threshold_db,
                            cached.loud_peak_db,
                            cached.op_ed_zones.len()
                        ),
                    );
                    return Ok(cached);
                }
            }
        }
    }

    crate::logger::emit_log(
        app_handle,
        "Audio",
        "info",
        "Построение высокоточной карты интенсивности звука (шаг 20 мс, RMS/Peak/ZCR)...",
    );

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    let temp_wav = std::env::temp_dir().join(format!(
        "vc_audio_env_{}_{}.wav",
        std::process::id(),
        ts
    ));

    let shell = app_handle.shell();
    let command = shell
        .sidecar("ffmpeg")
        .map_err(|e| AppError::Pipeline(format!("FFmpeg sidecar не найден: {}", e)))?
        .env("PATH", crate::logger::build_sidecar_env_path(app_handle))
        .args([
            "-nostdin",
            "-y",
            "-vn",
            "-i",
            &video_path.to_string_lossy(),
            "-acodec",
            "pcm_s16le",
            "-ar",
            "4000",
            "-ac",
            "1",
            &temp_wav.to_string_lossy(),
        ]);

    let (mut rx, child) = command
        .spawn()
        .map_err(|e| AppError::Pipeline(format!("Не удалось запустить FFmpeg для аудио-профиля: {}", e)))?;

    let extract_fut = async {
        while let Some(ev) = rx.recv().await {
            if let CommandEvent::Terminated(payload) = ev {
                if payload.code == Some(0) {
                    return Ok(());
                } else {
                    return Err(AppError::Pipeline(format!(
                        "FFmpeg завершился с кодом {:?} при построении аудио-профиля",
                        payload.code
                    )));
                }
            }
        }
        Err(AppError::Pipeline(
            "Поток FFmpeg прервался при построении аудио-профиля".to_string(),
        ))
    };

    let res = tokio::select! {
        r = extract_fut => r,
        _ = cancel_rx.recv() => {
            let _ = child.kill();
            let _ = tokio::fs::remove_file(&temp_wav).await;
            return Err(AppError::Pipeline("Анализ аудио отменен пользователем.".to_string()));
        }
    };

    if let Err(e) = res {
        let _ = tokio::fs::remove_file(&temp_wav).await;
        return Err(e);
    }

    let raw_bytes = tokio::fs::read(&temp_wav).await.map_err(AppError::Io)?;
    let _ = tokio::fs::remove_file(&temp_wav).await;

    let profile = build_profile_from_wav_bytes(&raw_bytes, 4000, 0.020, fragments);

    if let Some(parent) = profile_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Ok(json_str) = serde_json::to_string(&profile) {
        let _ = tokio::fs::write(&profile_path, json_str).await;
    }

    let op_ed_summary = if profile.op_ed_zones.is_empty() {
        "не обнаружены".to_string()
    } else {
        profile
            .op_ed_zones
            .iter()
            .map(|(s, e)| format!("[{:.1}с..{:.1}с]", s, e))
            .collect::<Vec<_>>()
            .join(", ")
    };

    crate::logger::emit_log(
        app_handle,
        "Audio",
        "info",
        &format!(
            "✓ Профиль аудио построен ({:.1} сек, {} окон по 20 мс): фон {:.1} dB, тихая пауза <= {:.1} dB, активная речь >= {:.1} dB, экшн-пик >= {:.1} dB (зоны OP/ED: {})",
            profile.duration_sec,
            profile.rms_db.len(),
            profile.noise_floor_db,
            profile.quiet_valley_db,
            profile.speech_threshold_db,
            profile.loud_peak_db,
            op_ed_summary
        ),
    );

    Ok(profile)
}

/// Строит профиль энергии из байтов WAV 16-bit PCM mono
fn build_profile_from_wav_bytes(
    wav_bytes: &[u8],
    sample_rate: usize,
    step_sec: f64,
    fragments: &[SubtitleFragment],
) -> AudioEnergyProfile {
    // Находим секцию "data" в заголовке WAV (стандартно 44 байта, но с LIST/INFO может быть до 256 байт)
    let mut data_offset = 44usize.min(wav_bytes.len());
    let search_limit = wav_bytes.len().min(512);
    for i in 12..search_limit.saturating_sub(8) {
        if &wav_bytes[i..i + 4] == b"data" {
            data_offset = i + 8;
            break;
        }
    }

    let pcm_slice = &wav_bytes[data_offset..];
    let samples_per_frame = ((sample_rate as f64) * step_sec).round().max(1.0) as usize;
    let bytes_per_frame = samples_per_frame * 2;
    let num_frames = pcm_slice.len() / bytes_per_frame;

    let mut rms_db = Vec::with_capacity(num_frames);
    let mut peak_db = Vec::with_capacity(num_frames);
    let mut zcr = Vec::with_capacity(num_frames);

    for frame_bytes in pcm_slice.chunks_exact(bytes_per_frame) {
        let mut sum_sq = 0.0f64;
        let mut max_abs = 0i32;
        let mut crossings = 0usize;
        let mut prev_sign = 0i8;

        for pair in frame_bytes.chunks_exact(2) {
            let sample = i16::from_le_bytes([pair[0], pair[1]]) as i32;
            let abs_s = sample.abs();
            if abs_s > max_abs {
                max_abs = abs_s;
            }
            let norm = (sample as f64) / 32768.0;
            sum_sq += norm * norm;

            let sign = if sample > 96 {
                1
            } else if sample < -96 {
                -1
            } else {
                0
            };
            if sign != 0 {
                if prev_sign != 0 && sign != prev_sign {
                    crossings += 1;
                }
                prev_sign = sign;
            }
        }

        let rms = (sum_sq / (samples_per_frame as f64)).sqrt();
        let peak = (max_abs as f64) / 32768.0;

        let r_db = (20.0 * rms.max(1e-5).log10() as f32).clamp(-90.0, 0.0);
        let p_db = (20.0 * peak.max(1e-5).log10() as f32).clamp(-90.0, 0.0);
        let z_rate = (crossings as f32) / (samples_per_frame as f32);

        rms_db.push(r_db);
        peak_db.push(p_db);
        zcr.push(z_rate);
    }

    // Вычисляем адаптивные пороги по перцентилям непустых фреймов (> -82 dB)
    let mut sorted_rms: Vec<f32> = rms_db.iter().copied().filter(|&v| v > -82.0).collect();
    sorted_rms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let (noise_floor_db, quiet_valley_db, speech_threshold_db, loud_peak_db) =
        if sorted_rms.len() >= 20 {
            let p18 = sorted_rms[sorted_rms.len() * 18 / 100];
            let p32 = sorted_rms[sorted_rms.len() * 32 / 100];
            let p48 = sorted_rms[sorted_rms.len() * 48 / 100];
            let p88 = sorted_rms[sorted_rms.len() * 88 / 100];

            let floor = p18.clamp(-58.0, -27.0);
            let valley = p32.clamp(floor + 2.5, -23.0);
            let speech = p48.clamp(valley + 3.0, -16.0);
            let loud = p88.clamp(speech + 4.5, -4.0);
            (floor, valley, speech, loud)
        } else {
            (-45.0, -36.0, -28.0, -14.0)
        };

    let duration_sec = (rms_db.len() as f64) * step_sec;

    let mut profile = AudioEnergyProfile {
        step_sec,
        duration_sec,
        rms_db,
        peak_db,
        zcr,
        noise_floor_db,
        quiet_valley_db,
        speech_threshold_db,
        loud_peak_db,
        op_ed_zones: Vec::new(),
    };

    profile.op_ed_zones = detect_op_ed_zones(&profile, fragments);
    profile
}

/// Автоматическое распознавание зон опенинга (OP) и эндинга (ED) в длинных видео (>= 8 мин):
/// Ищет 65–100-секундные участки непрерывного музыкального плато (высокая плотность громких фреймов без глубоких разговорных пауз
/// и низкая плотность обычных диалоговых субтитров) в первых 5.5 минутах и последних 4.5 минутах серии.
pub fn detect_op_ed_zones(
    profile: &AudioEnergyProfile,
    fragments: &[SubtitleFragment],
) -> Vec<(f64, f64)> {
    let mut zones = Vec::new();
    if profile.duration_sec < 480.0 || profile.rms_db.is_empty() {
        return zones;
    }

    let window_sec = 76.0;
    let win_frames = (window_sec / profile.step_sec).round() as usize;
    let step_frames = (2.0 / profile.step_sec).round().max(1.0) as usize;

    // 1. Поиск Опенинга в диапазоне [0.0с .. 330.0с]
    let op_max_end_sec = 330.0_f64.min(profile.duration_sec * 0.35);
    if let Some(op_zone) = find_music_plateau_in_range(
        profile,
        fragments,
        0.0,
        op_max_end_sec,
        win_frames,
        step_frames,
    ) {
        zones.push(op_zone);
    }

    // 2. Поиск Эндинга в диапазоне [duration - 260.0с .. duration]
    let ed_min_start_sec = (profile.duration_sec - 260.0).max(op_max_end_sec + 60.0);
    if let Some(ed_zone) = find_music_plateau_in_range(
        profile,
        fragments,
        ed_min_start_sec,
        profile.duration_sec,
        win_frames,
        step_frames,
    ) {
        zones.push(ed_zone);
    }

    zones
}

fn find_music_plateau_in_range(
    profile: &AudioEnergyProfile,
    fragments: &[SubtitleFragment],
    range_start_sec: f64,
    range_end_sec: f64,
    win_frames: usize,
    step_frames: usize,
) -> Option<(f64, f64)> {
    let start_idx = profile.time_to_idx(range_start_sec);
    let end_idx = profile.time_to_idx(range_end_sec);
    if end_idx <= start_idx + win_frames {
        return None;
    }

    let music_floor_db = (profile.quiet_valley_db + 1.5).min(profile.speech_threshold_db - 1.0);
    let mut best_score = 0.0f64;
    let mut best_bounds: Option<(f64, f64)> = None;

    let mut i = start_idx;
    while i + win_frames <= end_idx {
        let slice = &profile.rms_db[i..i + win_frames];
        let active_cnt = slice.iter().filter(|&&v| v >= music_floor_db).count();
        let active_ratio = (active_cnt as f64) / (win_frames as f64);

        // Считаем максимальную непрерывную тихую паузу внутри 76-секундного окна
        let mut max_quiet_run = 0usize;
        let mut curr_quiet_run = 0usize;
        for &v in slice {
            if v < profile.quiet_valley_db - 1.0 {
                curr_quiet_run += 1;
                if curr_quiet_run > max_quiet_run {
                    max_quiet_run = curr_quiet_run;
                }
            } else {
                curr_quiet_run = 0;
            }
        }
        let max_quiet_sec = (max_quiet_run as f64) * profile.step_sec;

        let w_start = profile.idx_to_time(i);
        let w_end = profile.idx_to_time(i + win_frames);

        // Проверяем количество реальных диалоговых реплик внутри окна
        let mut dialogue_cues = 0usize;
        let mut song_cues = 0usize;
        for (f_idx, f) in fragments.iter().enumerate() {
            if f.end > w_start + 2.0 && f.start < w_end - 2.0 {
                if crate::whisper::is_whisper_hallucination(&f.text)
                    || crate::text_analyzer::is_opening_or_song_fragment(fragments, f_idx)
                {
                    song_cues += 1;
                } else {
                    dialogue_cues += 1;
                }
            }
        }

        // Опенинг/эндинг характеризуется высокой непрерывной заполненностью звуком (>= 78%),
        // отсутствием длинных пауз тишины (<= 2.2с) и малым числом разговорных реплик (<= 5 за 76 секунд)
        let is_musical_plateau = active_ratio >= 0.78
            && max_quiet_sec <= 2.2
            && (dialogue_cues <= 5 || song_cues >= 2);

        if is_musical_plateau {
            let score = active_ratio * 1.5
                + (song_cues as f64) * 0.15
                - (dialogue_cues as f64) * 0.08
                - max_quiet_sec * 0.12;
            if score > best_score {
                best_score = score;
                best_bounds = Some((w_start, w_end));
            }
        }

        i += step_frames;
    }

    // Расширяем найденное ядро плато (76с) до стандартных 88–90с границ музыкальной темы (до ближайшей глубокой паузы)
    if let Some((mut z_start, mut z_end)) = best_bounds {
        let s_idx = profile.time_to_idx(z_start);
        let min_s_idx = profile.time_to_idx((z_start - 10.0).max(range_start_sec));
        for k in (min_s_idx..s_idx).rev() {
            if profile.smoothed_rms_at(k) < profile.quiet_valley_db {
                z_start = profile.idx_to_time(k);
                break;
            }
        }

        let e_idx = profile.time_to_idx(z_end);
        let max_e_idx = profile.time_to_idx((z_end + 12.0).min(range_end_sec));
        for k in e_idx..=max_e_idx {
            if profile.smoothed_rms_at(k) < profile.quiet_valley_db {
                z_end = profile.idx_to_time(k);
                break;
            }
        }

        return Some((z_start, z_end));
    }

    None
}

/// Вычисляет локальный порог тихой впадины (22-й перцентиль RMS в окне ±3.5с вокруг `center_sec`),
/// чтобы во время боевых сцен с фоновой музыкой / гулом двигателей (-21 dB) алгоритм не путал фоновую музыку с оборванной речью!
fn compute_local_valley_db(profile: &AudioEnergyProfile, center_sec: f64) -> f32 {
    let idx_s = profile.time_to_idx((center_sec - 3.5).max(0.0));
    let idx_e = profile.time_to_idx((center_sec + 3.5).min(profile.duration_sec));
    if idx_e <= idx_s + 6 {
        return profile.quiet_valley_db;
    }
    let mut local_vals: Vec<f32> = profile.rms_db[idx_s..=idx_e]
        .iter()
        .copied()
        .filter(|&v| v > -82.0)
        .collect();
    if local_vals.len() < 8 {
        return profile.quiet_valley_db;
    }
    local_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p22 = local_vals[local_vals.len() * 22 / 100];
    p22.max(profile.quiet_valley_db).min(profile.loud_peak_db - 3.5)
}

/// Выравнивает начало и конец клипа по тихим местам в широком окне до `±2.5 сек` (`max_shift_sec = 2.5`):
/// 1) Исключает обрыв начала голосовой фразы (если актёр начал говорить раньше таймкода Whisper — сдвигает старт назад в тишину перед атакой голоса).
/// 2) Исключает обрыв конца фразы (если в расчетной точке `end_sec` голос ещё звучит — доводит границу вперёд до реального спада звука в тихую паузу).
/// 3) Если же на хвосте `end_sec` случайно захвачен первый слог/вдох следующей чужой фразы после паузы — подтягивает `end_sec` назад в центр этой тихой паузы.
pub fn refine_clip_bounds_by_audio(
    profile: &AudioEnergyProfile,
    raw_start: f64,
    raw_end: f64,
    fragments: &[SubtitleFragment],
    max_shift_sec: f64,
    min_clip_dur: f64,
    max_clip_dur: f64,
) -> (f64, f64, bool, bool) {
    if profile.rms_db.len() < 25 || raw_end <= raw_start + 1.0 {
        return (raw_start, raw_end, false, false);
    }

    let max_shift = max_shift_sec.clamp(0.15, 2.5);

    // Определяем первую и последнюю реплики внутри клипа (если есть субтитры), чтобы знать безопасные границы соседних чужих фраз
    let first_cue_idx = fragments
        .iter()
        .position(|f| f.end > raw_start + 0.20 && f.start < raw_end - 0.20);
    let last_cue_idx = fragments
        .iter()
        .rposition(|f| f.end > raw_start + 0.20 && f.start < raw_end - 0.20);

    // --- 1. ВЫРАВНИВАНИЕ НАЧАЛА КЛИПА (`new_start`) ---
    let mut min_allowed_start = (raw_start - max_shift).max(0.0);
    let mut max_allowed_start = (raw_start + 0.45.min(max_shift)).min(raw_end - min_clip_dur.max(3.5));

    if let Some(f_idx) = first_cue_idx {
        let first_cue = &fragments[f_idx];
        // Не сдвигаем старт позже, чем за 0.08с до начала первой реплики клипа,
        // и не уводим раньше, чем за 0.65с до первой реплики (чтобы фоновый шум/BGM не утягивал старт на 2.3с в предыдущую сцену!)
        max_allowed_start = max_allowed_start.min((first_cue.start - 0.08).max(raw_start));
        min_allowed_start = min_allowed_start.max((first_cue.start - 0.65.min(max_shift)).max(0.0));

        if f_idx > 0 {
            let prev_cue = &fragments[f_idx - 1];
            let gap_before = (first_cue.start - prev_cue.end).max(0.0);
            if gap_before <= max_shift + 0.5 {
                min_allowed_start = min_allowed_start.max((prev_cue.end + 0.02).min(raw_start));
            }
        }
    }

    let raw_start_idx = profile.time_to_idx(raw_start);
    let start_rms = profile.smoothed_rms_at(raw_start_idx);
    let local_start_valley_db = compute_local_valley_db(profile, raw_start);
    let mut rescued_start = false;

    let new_start = if start_rms > local_start_valley_db + 2.2 && start_rms > profile.quiet_valley_db + 1.8 {
        // В точке `raw_start` УЖЕ звучит голос поверх локального фона (фраза началась раньше таймкода)!
        rescued_start = true;
        find_quiet_valley_before_onset(
            profile,
            raw_start,
            min_allowed_start,
            max_allowed_start,
            local_start_valley_db,
        )
    } else {
        // В точке `raw_start` относительно тихо — находим самую чистую точку тишины (минимальный RMS + Peak)
        let local_min_bound = (raw_start - 0.45.min(max_shift)).max(min_allowed_start);
        find_cleanest_quiet_point(profile, raw_start, local_min_bound, max_allowed_start)
    };

    // --- 2. ВЫРАВНИВАНИЕ КОНЦА КЛИПА (`new_end`) ---
    let mut min_allowed_end = (raw_end - 0.45.min(max_shift)).max(new_start + min_clip_dur.max(4.0));
    let mut max_allowed_end = (raw_end + max_shift)
        .min(profile.duration_sec)
        .min(new_start + max_clip_dur + 3.8);

    if let Some(l_idx) = last_cue_idx {
        let last_cue = &fragments[l_idx];
        // Разрешаем отступить назад максимум до `last_cue.end - 0.22`, а вперед — не далее `last_cue.end + 0.95`
        min_allowed_end = min_allowed_end.max(last_cue.start + 0.65).max(last_cue.end - 0.22);
        max_allowed_end = max_allowed_end.min((last_cue.end + 0.95.min(max_shift)).min(profile.duration_sec));

        if l_idx + 1 < fragments.len() {
            let next_cue = &fragments[l_idx + 1];
            let gap_after = (next_cue.start - last_cue.end).max(0.0);
            if gap_after >= 0.08 {
                // Строго не залезаем на начало следующей реплики!
                max_allowed_end = max_allowed_end.min((next_cue.start - 0.04).max(last_cue.end + 0.04));
            }
        }
    }

    let raw_end_idx = profile.time_to_idx(raw_end);
    let end_rms = profile.smoothed_rms_at(raw_end_idx);
    let local_end_valley_db = compute_local_valley_db(profile, raw_end);
    let mut rescued_end = false;

    let new_end = if end_rms > local_end_valley_db + 2.0 && end_rms > profile.quiet_valley_db + 1.5 {
        rescued_end = true;
        resolve_active_end_boundary(
            profile,
            raw_end,
            min_allowed_end,
            max_allowed_end,
            local_end_valley_db,
        )
    } else {
        let local_max_bound = (raw_end + 0.50.min(max_shift)).min(max_allowed_end);
        find_cleanest_quiet_point(profile, raw_end, min_allowed_end, local_max_bound)
    };

    let final_start = (new_start * 100.0).round() / 100.0;
    let final_end = (new_end * 100.0).round() / 100.0;

    if final_end <= final_start + 3.0 {
        return (raw_start, raw_end, false, false);
    }

    (final_start, final_end, rescued_start, rescued_end)
}

/// Если в точке `raw_start` уже звучит голос, сканирует назад до `min_start`,
/// чтобы найти тихую впадину перед началом этой фразы и поставить точку старта за ~100 мс до голосовой атаки.
fn find_quiet_valley_before_onset(
    profile: &AudioEnergyProfile,
    raw_start: f64,
    min_start: f64,
    max_start: f64,
    local_valley_db: f32,
) -> f64 {
    let idx_raw = profile.time_to_idx(raw_start);
    let idx_min = profile.time_to_idx(min_start);
    let idx_max = profile.time_to_idx(max_start);

    // 1. Сначала проверим, нет ли тихой паузы чуть правее `raw_start` (до `idx_max`),
    // если `raw_start` зацепил самый последний затухающий хвост предыдущего шума перед паузой!
    for k in idx_raw..=idx_max {
        if k + 2 < profile.rms_db.len()
            && profile.rms_db[k] <= local_valley_db + 0.5
            && profile.rms_db[k + 1] <= local_valley_db + 0.5
        {
            let valley_center = (k + 1).min(idx_max);
            return profile.idx_to_time(valley_center);
        }
    }

    // 2. Иначе шагаем назад от `idx_raw` к `idx_min`, пока длится непрерывная звуковая атака начавшейся раньше фразы
    let mut best_idx = idx_raw;
    let mut best_db = profile.smoothed_rms_at(idx_raw);

    for k in (idx_min..=idx_raw).rev() {
        let db = profile.smoothed_rms_at(k);
        if db < best_db {
            best_db = db;
            best_idx = k;
        }
        // Если нашли устойчивую тихую паузу (>= 3 фреймов = 60 мс подряд ниже локального порога тихой впадины)
        if k >= idx_min + 2
            && profile.rms_db[k] <= local_valley_db + 0.8
            && profile.rms_db[k - 1] <= local_valley_db + 0.8
            && profile.rms_db[k - 2] <= local_valley_db + 1.2
        {
            let target_idx = (k.saturating_sub(1)).max(idx_min);
            return profile.idx_to_time(target_idx);
        }
    }

    profile.idx_to_time(best_idx)
}

/// Когда в точке `raw_end` уровень звука высокий (голос ещё звучит):
/// - Проверяет, не начался ли прямо перед `raw_end` (после тихой впадины) первый слог СЛЕДУЮЩЕЙ фразы. Если да — режет по этой тихой впадине!
/// - Иначе (текущая фраза ещё договаривается за пределами таймкода субтитра) — идет вперёд до первой устойчивой тихой паузы после окончания фразы!
fn resolve_active_end_boundary(
    profile: &AudioEnergyProfile,
    raw_end: f64,
    min_end: f64,
    max_end: f64,
    local_valley_db: f32,
) -> f64 {
    let idx_raw = profile.time_to_idx(raw_end);
    let idx_min = profile.time_to_idx(min_end);
    let idx_max = profile.time_to_idx(max_end);

    // 1. Проверяем, была ли в последних 0.45с перед `raw_end` четкая тихая пауза (`>= 60 мс`), после которой начался новый всплеск речи
    let lookback_start = idx_raw.saturating_sub(22).max(idx_min);
    for k in (lookback_start..idx_raw.saturating_sub(2)).rev() {
        if profile.rms_db[k] <= local_valley_db + 0.6
            && profile.rms_db[k + 1] <= local_valley_db + 0.6
            && profile.rms_db[k.saturating_sub(1)] <= local_valley_db + 1.2
        {
            return profile.idx_to_time(k + 1);
        }
    }

    // 2. Текущая фраза/возглас продолжается в точке `raw_end` — идем вперед до `idx_max`,
    // пока фраза полностью не договорится и не уйдет в локальную тихую впадину!
    let mut quiet_run = 0usize;
    let mut lowest_idx = idx_raw;
    let mut lowest_db = profile.smoothed_rms_at(idx_raw);

    for k in idx_raw..=idx_max {
        let db = profile.smoothed_rms_at(k);
        if db < lowest_db {
            lowest_db = db;
            lowest_idx = k;
        }

        if profile.rms_db[k] <= local_valley_db + 1.0 {
            quiet_run += 1;
            if quiet_run >= 4 {
                let target_idx = (k.saturating_sub(1)).min(idx_max);
                return profile.idx_to_time(target_idx);
            }
        } else {
            quiet_run = 0;
        }
    }

    // 3. Если на фоне играет музыка и абсолютной тишины нет — выбираем самый тихий локальный минимум энергии в окне `[idx_min .. idx_max]`
    for k in idx_min..idx_raw {
        let db = profile.smoothed_rms_at(k);
        if db + 2.5 < lowest_db {
            lowest_db = db;
            lowest_idx = k;
        }
    }

    profile.idx_to_time(lowest_idx)
}

/// Находит самую тихую и свободную от пиков точку в заданном интервале `[min_sec .. max_sec]` вблизи `target_sec`
fn find_cleanest_quiet_point(
    profile: &AudioEnergyProfile,
    target_sec: f64,
    min_sec: f64,
    max_sec: f64,
) -> f64 {
    let idx_target = profile.time_to_idx(target_sec);
    let idx_min = profile.time_to_idx(min_sec);
    let idx_max = profile.time_to_idx(max_sec);
    if idx_max <= idx_min {
        return target_sec;
    }

    let mut best_idx = idx_target.clamp(idx_min, idx_max);
    let mut best_cost = f32::MAX;

    for k in idx_min..=idx_max {
        let rms = profile.smoothed_rms_at(k);
        let peak = profile.peak_db[k];
        // Небольшой штраф за удаление от исходной точки, чтобы при равной тишине не уводить границу далеко
        let dist_sec = ((k as isize - idx_target as isize).abs() as f32) * (profile.step_sec as f32);
        let cost = rms * 0.70 + peak * 0.30 + dist_sec * 3.2;
        if cost < best_cost {
            best_cost = cost;
            best_idx = k;
        }
    }

    profile.idx_to_time(best_idx)
}

/// Выравнивает список готовых интервалов клипов по карте аудио-интенсивности (в окне до ±2.5 сек),
/// гарантируя отсутствие нахлестов соседних клипов и возвращая подробную статистику коррекции.
pub fn refine_selected_intervals_with_audio(
    profile: &AudioEnergyProfile,
    intervals: &mut [(f64, f64, f64, String, String)],
    fragments: &[SubtitleFragment],
    min_dur: f64,
    max_dur: f64,
) -> BoundaryRefinementStats {
    let mut stats = BoundaryRefinementStats {
        total_clips: intervals.len(),
        ..Default::default()
    };

    for idx in 0..intervals.len() {
        let (orig_s, orig_e, _, _, _) = intervals[idx];
        let (mut new_s, mut new_e, r_start, r_end) = refine_clip_bounds_by_audio(
            profile,
            orig_s,
            orig_e,
            fragments,
            2.5,
            min_dur,
            max_dur,
        );

        // Предотвращаем нахлест на предыдущий уже выровненный клип
        if idx > 0 {
            let prev_end = intervals[idx - 1].1;
            if new_s < prev_end + 0.5 {
                new_s = orig_s.max(prev_end + 0.5);
            }
        }
        // Предотвращаем нахлест на следующий клип
        if idx + 1 < intervals.len() {
            let next_start = intervals[idx + 1].0;
            if new_e > next_start - 0.5 {
                new_e = orig_e.min(next_start - 0.5);
            }
        }

        if new_e > new_s + 3.5 {
            let shift_s = (new_s - orig_s).abs();
            let shift_e = (new_e - orig_e).abs();
            let max_s = shift_s.max(shift_e);
            if max_s >= 0.04 {
                stats.adjusted_clips += 1;
                if max_s > stats.max_shift_sec {
                    stats.max_shift_sec = max_s;
                }
            }
            if r_start {
                stats.rescued_speech_starts += 1;
            }
            if r_end {
                stats.rescued_speech_ends += 1;
            }
            intervals[idx].0 = new_s;
            intervals[idx].1 = new_e;
        }
    }

    stats
}

/// Находит самые громкие и динамичные экшн-сцены в серии (взрывы, сражения, магия, боевой саундтрек),
/// которые могли не попасть в текстовый отбор из-за отсутствия субтитров или малого числа слов.
/// Строго комбинирует пики звуковой интенсивности + плотность смены кадров (`scene_cuts`) и автоматически отсекает OP/ED!
#[allow(clippy::too_many_arguments)]
pub fn extract_high_intensity_action_windows(
    profile: &AudioEnergyProfile,
    scene_cuts: &[f64],
    fragments: &[SubtitleFragment],
    scenes: &[crate::analyzer::SceneCandidate],
    min_dur: f64,
    target_dur: f64,
    max_dur: f64,
    intro_skip: f64,
    outro_limit: f64,
) -> Vec<(f64, f64, f64, String, String)> {
    let mut action_windows = Vec::new();
    if profile.rms_db.len() < 100 || profile.duration_sec < min_dur {
        return action_windows;
    }

    let win_sec = target_dur.clamp(min_dur, max_dur);
    let win_frames = (win_sec / profile.step_sec).round().max(20.0) as usize;
    let step_frames = (2.5 / profile.step_sec).round().max(5.0) as usize;

    let start_idx = profile.time_to_idx(intro_skip);
    let end_idx = profile.time_to_idx(outro_limit.min(profile.duration_sec));
    if end_idx <= start_idx + win_frames {
        return action_windows;
    }

    // Кандидаты: (raw_start, raw_end, action_score, local_cuts, peak_db_90)
    let mut raw_candidates: Vec<(f64, f64, f64, usize)> = Vec::new();

    let mut i = start_idx;
    while i + win_frames <= end_idx {
        let w_start = profile.idx_to_time(i);
        let w_end = profile.idx_to_time(i + win_frames);

        // 1. Строго исключаем зоны Опенинга и Эндинга (OP/ED)
        if profile.overlaps_op_ed(w_start, w_end) {
            i += step_frames;
            continue;
        }

        // Дополнительная защита от неразмеченного эндинга в последних 95 секундах 20+-минутной серии
        if profile.duration_sec >= 1200.0 && w_end >= profile.duration_sec - 95.0 {
            i += step_frames;
            continue;
        }

        // 2. Проверяем плотность визуальных монтажных склеек (`scene_cuts`) внутри окна `[w_start - 1.5 .. w_end + 1.5]`
        let local_cuts = scene_cuts
            .iter()
            .filter(|&&t| t >= w_start - 1.5 && t <= w_end + 1.5)
            .count();

        // Для настоящего экшена (чтобы не брать статичный экран с громким гулом) требуется минимум 3 смены кадра
        if local_cuts < 3 {
            i += step_frames;
            continue;
        }

        // 3. Оцениваем энергетический профиль окна (громкость пиков + динамический перепад взрывов/ударов)
        let rms_slice = &profile.rms_db[i..i + win_frames];
        let peak_slice = &profile.peak_db[i..i + win_frames];

        let loud_frames = rms_slice
            .iter()
            .filter(|&&db| db >= profile.loud_peak_db - 2.0)
            .count();
        let loud_ratio = (loud_frames as f64) / (win_frames as f64);

        // Требуем, чтобы окно действительно содержало мощный звуковой пик (хотя бы 18% времени на пиковой мощности)
        if loud_ratio < 0.18 {
            i += step_frames;
            continue;
        }

        // Считаем динамическую взрывную атаку (средний скачок громкости между соседними 60-мс интервалами)
        let mut transient_jumps = 0.0f64;
        for k in (i + 3)..(i + win_frames) {
            let diff = (profile.rms_db[k] - profile.rms_db[k - 3]).abs() as f64;
            if diff >= 4.5 {
                transient_jumps += diff;
            }
        }
        let transient_norm = (transient_jumps / (win_frames as f64 * 2.2)).clamp(0.0, 1.0);

        let max_peak = peak_slice
            .iter()
            .copied()
            .fold(-90.0f32, f32::max);

        // Требуем реальный пик громкости вблизи максимума дорожки
        if max_peak < profile.loud_peak_db + 1.0 {
            i += step_frames;
            continue;
        }

        // Вычисляем итоговый рейтинг экшн-сцены (0.72 .. 0.94)
        let cut_bonus = ((local_cuts as f64 - 2.0) * 0.022).clamp(0.02, 0.16);
        let loud_bonus = (loud_ratio * 0.22).clamp(0.04, 0.16);
        let transient_bonus = (transient_norm * 0.12).clamp(0.01, 0.10);

        // Проверяем, есть ли подтверждение от Vision AI (`scenes`) в этом интервале
        let vision_bonus = scenes
            .iter()
            .filter(|s| s.timestamp_sec >= w_start - 2.0 && s.timestamp_sec <= w_end + 2.0)
            .map(|s| ((s.intensity - 0.72) * 0.18).clamp(0.0, 0.06))
            .fold(0.0f64, f64::max);

        let action_score = (0.64 + cut_bonus + loud_bonus + transient_bonus + vision_bonus).clamp(0.70, 0.94);

        raw_candidates.push((w_start, w_end, action_score, local_cuts));
        i += step_frames;
    }

    // Сортируем по убыванию силы экшн-пика и выбираем до 8 независимых экшн-кульминаций серии
    raw_candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let min_action_dist = (profile.duration_sec / 35.0).clamp(18.0, 40.0);
    let mut chosen_raw: Vec<(f64, f64, f64, usize)> = Vec::new();
    for cand in raw_candidates {
        let too_close = chosen_raw
            .iter()
            .any(|c| (c.0 - cand.0).abs() < min_action_dist);
        if !too_close {
            chosen_raw.push(cand);
        }
        if chosen_raw.len() >= 8 {
            break;
        }
    }

    for (raw_s, raw_e, score, cuts) in chosen_raw {
        // Выравниваем границы экшн-сцены по тихим местам в окне до ±2.5 сек!
        let (clean_s, clean_e, _, _) = refine_clip_bounds_by_audio(
            profile,
            raw_s,
            raw_e,
            fragments,
            2.5,
            min_dur,
            max_dur,
        );

        if clean_e - clean_s < min_dur * 0.85 || profile.overlaps_op_ed(clean_s, clean_e) {
            continue;
        }

        // Если внутри экшн-сцены есть реплики, берем их для превью в логе; иначе помечаем как «Экшн-кульминация (пик звука)»
        let cues_inside: Vec<&SubtitleFragment> = fragments
            .iter()
            .filter(|f| {
                f.end > clean_s + 0.3
                    && f.start < clean_e - 0.3
                    && !crate::whisper::is_whisper_hallucination(&f.text)
            })
            .collect();

        let (first_label, last_label) = if let (Some(first_f), Some(last_f)) = (cues_inside.first(), cues_inside.last()) {
            let f_str: String = first_f.text.chars().take(36).collect();
            let l_str: String = last_f.text.chars().take(36).collect();
            (format!("[Экшн] {}", f_str), l_str)
        } else {
            (
                format!("Экшн-пик звука ({} склеек)", cuts),
                "Боевая/динамичная сцена".to_string(),
            )
        };

        action_windows.push((clean_s, clean_e, score, first_label, last_label));
    }

    action_windows
}
