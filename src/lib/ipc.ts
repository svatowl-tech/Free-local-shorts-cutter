import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { isTauriRuntime, formatErrorMessage } from './utils/path';

export interface FfmpegProgressPayload {
  current_time: number;
  duration: number;
  percent: number;
}

export interface WhisperProgressPayload {
  percent: number;
  stage: string;
}

export interface AnalyzerProgressPayload {
  percent: number;
  stage: string;
  current_frame: number;
  total_frames: number;
}

export interface SceneCandidate {
  timestamp_sec: number;
  description: string;
  intensity: number;
  actions: string[];
}

export interface TranscribeResult {
  srt_path: string;
  srt_content: string;
}

export interface SubtitleStyle {
  font_name: string;
  font_size: number;
  color: string;
  placement: string;
}

export interface OpenAiProfile {
  id: string;
  name: string;
  base_url: string;
  api_key: string;
  model: string;
  available_models?: string[];
}

export interface SubtitleCharacterInfo {
  name: string;
  line_count: number;
  total_duration_sec?: number;
  sample_text?: string;
}

export interface ParsedSubtitleLine {
  start: number;
  end: number;
  text: string;
  character: string;
}

export interface CustomSubtitlesParseResult {
  file_path: string;
  total_lines: number;
  characters: SubtitleCharacterInfo[];
  lines?: ParsedSubtitleLine[];
}

export interface BumperItemConfig {
  enabled: boolean;
  mode: 'custom_video' | 'generate';
  custom_video_path?: string | null;
  custom_audio_path?: string | null;
  custom_audio_duration?: number | null;
  sync_duration_to_audio?: boolean;
  audio_volume?: number;
  audio_fade_in?: boolean;
  audio_fade_out?: boolean;
  title: string;
  subtitle?: string | null;
  duration_sec: number;
  bg_mode?: 'blur_frame' | 'custom_video' | 'custom_image' | 'gradient';
  bg_video_path?: string | null;
  bg_image_path?: string | null;
  bg_color_primary?: string;
  bg_color_secondary?: string;
  bg_blur?: number;
  bg_dim?: number;
  bg_scale?: number;
  bg_saturation?: number;
  bg_contrast?: number;
  bg_vignette?: boolean;
  bg_video_speed?: number;
  logo_image_path?: string | null;
  logo_scale: number;
  logo_x_percent: number;
  logo_y_percent: number;
  logo_opacity?: number;
  logo_radius?: number;
  logo_shadow?: boolean;
  title_color: string;
  title_size: number;
  subtitle_size?: number;
  subtitle_color?: string;
  outline_color?: string;
  outline_width?: number;
  plate_style?: 'glass' | 'solid' | 'none';
  plate_color?: string;
  plate_opacity?: number;
  title_x_percent: number;
  title_y_percent: number;
  animation?: 'fade_zoom' | 'slide_up' | 'pulse' | 'none';
  transition: 'crossfade' | 'fade_black' | 'cut';
  overlay_png_base64?: string | null;
}

export interface BumperCategoryConfig {
  preroll: BumperItemConfig;
  postroll: BumperItemConfig;
}

export interface BumperPreset {
  id: string;
  name: string;
  v916: BumperCategoryConfig;
  v169: BumperCategoryConfig;
  created_at: number;
}

export interface BumperGlobalConfig {
  enabled: boolean;
  active_preset_id: string;
  presets: BumperPreset[];
}

export interface PipelineConfig {
  asr_model_path?: string;
  vision_model_path?: string;
  text_engine: string;
  text_model_name: string;
  ollama_url: string;
  openai_base_url?: string;
  openai_api_key?: string;
  openai_model?: string;
  openai_profile_id?: string;
  output_dir?: string;
  min_intensity_threshold: number;
  max_clip_duration_sec: number;
  max_clips_count?: number;
  vertical_format: boolean;
  render_highlights?: boolean;
  render_talk_video?: boolean;
  custom_subtitles_path?: string | null;
  selected_characters?: string[];
  subtitle_style?: SubtitleStyle;
  bumper_config?: BumperGlobalConfig;
  mode: 'auto' | 'manual' | 'confirm';
}

export interface PipelineProgressPayload {
  percent: number;
  stage: string;
}

export async function extractAudio(videoPath: string, outputAudioPath?: string): Promise<string> {
  try {
    return await invoke<string>('extract_audio', { videoPath, outputAudioPath });
  } catch (err: unknown) {
    throw new Error(`Ошибка извлечения аудио: ${formatErrorMessage(err)}`);
  }
}

export function parseSubtitlesClientFallback(fileName: string, content: string): CustomSubtitlesParseResult {
  const cleanContent = content.replace(/^\uFEFF/, '');
  const ext = (fileName.split('.').pop() || '').toLowerCase();
  const isAss = ext === 'ass' || ext === 'ssa' || cleanContent.includes('[Events]') || cleanContent.includes('\nDialogue:');

  const parseTime = (t: string): number => {
    const norm = t.trim().replace(',', '.');
    const parts = norm.split(':').map((p) => parseFloat(p) || 0);
    if (parts.length === 3) return parts[0] * 3600 + parts[1] * 60 + parts[2];
    if (parts.length === 2) return parts[0] * 60 + parts[1];
    return 0;
  };

  const extractPrefix = (raw: string): { char: string; body: string } => {
    const trimmed = raw.trim();
    const vttMatch = trimmed.match(/^<v\s+([^>]+)>(.*)$/i);
    if (vttMatch) {
      return { char: vttMatch[1].trim(), body: vttMatch[2].replace(/<\/v>/gi, '').trim() };
    }
    const bracketMatch = trimmed.match(/^[\[(]([^\])]{1,28})[\])]\s*[:\-—]?\s*(.+)$/);
    if (bracketMatch) {
      return { char: bracketMatch[1].trim(), body: bracketMatch[2].trim() };
    }
    const colonIdx = trimmed.indexOf(':');
    if (colonIdx > 0 && colonIdx <= 28) {
      const name = trimmed.slice(0, colonIdx).replace(/^[-—\s]+/, '').trim();
      const body = trimmed.slice(colonIdx + 1).trim();
      const words = name.split(/\s+/).filter(Boolean).length;
      if (name && words >= 1 && words <= 3 && !/^\d+$/.test(name) && body) {
        return { char: name, body };
      }
    }
    return { char: 'Общий диктор', body: trimmed };
  };

  const parsedLines: ParsedSubtitleLine[] = [];

  if (isAss) {
    let formatCols: string[] = [];
    for (const rawLine of cleanContent.split(/\r?\n/)) {
      const trimmed = rawLine.trim();
      if (!trimmed || trimmed.startsWith(';')) continue;
      if (trimmed.startsWith('Format:')) {
        formatCols = trimmed
          .slice(7)
          .split(',')
          .map((c) => c.trim().toLowerCase());
      } else if (trimmed.startsWith('Dialogue:')) {
        const valsStr = trimmed.slice(9);
        const colCount = formatCols.length > 0 ? formatCols.length : 10;
        const parts: string[] = [];
        let cur = valsStr;
        for (let i = 0; i < colCount - 1; i++) {
          const idx = cur.indexOf(',');
          if (idx === -1) break;
          parts.push(cur.slice(0, idx));
          cur = cur.slice(idx + 1);
        }
        parts.push(cur);

        const getCol = (name: string, fallbackIdx: number): string => {
          const pos = formatCols.indexOf(name);
          return (pos >= 0 ? parts[pos] : parts[fallbackIdx]) || '';
        };

        const start = parseTime(getCol('start', 1));
        const end = parseTime(getCol('end', 2));
        const style = getCol('style', 3).trim();
        const actor = (getCol('name', 4) || getCol('actor', 4)).trim();
        const rawText = (getCol('text', parts.length - 1) || '').trim();
        const cleanText = rawText
          .replace(/\{[^}]*\}/g, '')
          .replace(/\\[Nn]/g, ' ')
          .replace(/\\h/g, ' ')
          .replace(/\s+/g, ' ')
          .trim();

        if (cleanText && end > start) {
          const styleLow = style.toLowerCase();
          if (['sign', 'signs', 'typeset', 'надписи', 'вывески', 'karaoke'].includes(styleLow)) continue;
          let charName = 'Общий диктор';
          let body = cleanText;
          if (actor) {
            charName = actor;
            body = extractPrefix(cleanText).body;
          } else if (
            style &&
            !['default', 'normal', 'main', 'alt', 'title', 'top', 'bottom', 'стандарт', 'основной', 'dialog', 'dialogue'].includes(styleLow)
          ) {
            charName = style;
            body = extractPrefix(cleanText).body;
          } else {
            const extRes = extractPrefix(cleanText);
            charName = extRes.char;
            body = extRes.body;
          }
          parsedLines.push({ start, end, text: body, character: charName });
        }
      }
    }
  } else {
    const blocks = cleanContent.split(/\r?\n\r?\n+/);
    for (const block of blocks) {
      const lines = block.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
      const timeLineIdx = lines.findIndex((l) => l.includes('-->'));
      if (timeLineIdx >= 0) {
        const [sStr, eStr] = lines[timeLineIdx].split('-->');
        const start = parseTime(sStr || '0:00:00');
        const end = parseTime((eStr || '0:00:00').split(/\s+/)[0]);
        const rawText = lines
          .slice(timeLineIdx + 1)
          .join(' ')
          .replace(/\{[^}]*\}/g, '')
          .replace(/<[^>]+>/g, '')
          .trim();
        if (rawText && end > start) {
          const { char, body } = extractPrefix(rawText);
          parsedLines.push({ start, end, text: body, character: char });
        }
      }
    }
  }

  const stats = new Map<string, { count: number; dur: number; sample: string }>();
  for (const line of parsedLines) {
    const d = Math.max(0.2, Math.min(25, line.end - line.start));
    const entry = stats.get(line.character) || { count: 0, dur: 0, sample: '' };
    entry.count += 1;
    entry.dur += d;
    if (!entry.sample && line.text.length >= 6) {
      entry.sample = line.text.slice(0, 70);
    }
    stats.set(line.character, entry);
  }

  const characters: SubtitleCharacterInfo[] = Array.from(stats.entries())
    .map(([name, st]) => ({
      name,
      line_count: st.count,
      total_duration_sec: Math.round(st.dur * 10) / 10,
      sample_text: st.sample
    }))
    .sort((a, b) => b.line_count - a.line_count);

  return {
    file_path: fileName,
    total_lines: parsedLines.length,
    characters,
    lines: parsedLines
  };
}

export async function parseCustomSubtitles(filePath: string): Promise<CustomSubtitlesParseResult> {
  if (!isTauriRuntime()) {
    throw new Error('В браузерном режиме используйте загрузку файла через диалог выбора субтитров');
  }
  try {
    return await invoke<CustomSubtitlesParseResult>('parse_custom_subtitles', { filePath });
  } catch (err: unknown) {
    throw new Error(`Ошибка парсинга субтитров: ${formatErrorMessage(err)}`);
  }
}

export async function parseCustomSubtitlesContent(fileName: string, content: string): Promise<CustomSubtitlesParseResult> {
  if (!isTauriRuntime()) {
    return parseSubtitlesClientFallback(fileName, content);
  }
  try {
    return await invoke<CustomSubtitlesParseResult>('parse_custom_subtitles_content', { fileName, content });
  } catch (err: unknown) {
    console.warn('Fallback to client subtitle parser:', formatErrorMessage(err));
    return parseSubtitlesClientFallback(fileName, content);
  }
}

export async function transcribeAudio(audioPath: string, modelPath?: string): Promise<TranscribeResult> {
  try {
    return await invoke<TranscribeResult>('transcribe_audio', { audioPath, modelPath });
  } catch (err: unknown) {
    throw new Error(`Ошибка Whisper ASR: ${formatErrorMessage(err)}`);
  }
}

export async function analyzeVideo(videoPath: string, modelPath?: string): Promise<SceneCandidate[]> {
  try {
    return await invoke<SceneCandidate[]>('analyze_video', { videoPath, modelPath });
  } catch (err: unknown) {
    throw new Error(`Ошибка видеоанализа Qwen3-VL: ${formatErrorMessage(err)}`);
  }
}

export async function onFfmpegProgress(callback: (payload: FfmpegProgressPayload) => void) {
  return await listen<FfmpegProgressPayload>('ffmpeg-progress', (event) => callback(event.payload));
}

export async function onWhisperProgress(callback: (payload: WhisperProgressPayload) => void) {
  return await listen<WhisperProgressPayload>('whisper-progress', (event) => callback(event.payload));
}

export async function onAnalyzerProgress(callback: (payload: AnalyzerProgressPayload) => void) {
  return await listen<AnalyzerProgressPayload>('analyzer-progress', (event) => callback(event.payload));
}

export interface PipelineResult {
  clips: string[];
  suggested_fragments: FragmentDto[];
}

export async function startPipeline(videoPath: string, config: PipelineConfig): Promise<PipelineResult> {
  try {
    return await invoke<PipelineResult>('start_pipeline', { videoPath, config });
  } catch (err: unknown) {
    throw new Error(`Ошибка pipeline: ${formatErrorMessage(err)}`);
  }
}

export interface FragmentDto {
  start: number;
  end: number;
}

export async function renderClipsFromFragments(
  videoPath: string,
  fragments: FragmentDto[],
  config: PipelineConfig
): Promise<string[]> {
  try {
    return await invoke<string[]>('render_clips_from_fragments', { videoPath, fragments, config });
  } catch (err: unknown) {
    throw new Error(`Ошибка рендеринга клипов по фрагментам: ${formatErrorMessage(err)}`);
  }
}

export async function generateSubtitlesOnly(videoPath: string, config: PipelineConfig): Promise<string> {
  try {
    return await invoke<string>('generate_subtitles_only', { videoPath, config });
  } catch (err: unknown) {
    throw new Error(`Ошибка генерации субтитров: ${formatErrorMessage(err)}`);
  }
}

export interface SuggestedFragmentsPayload {
  fragments: FragmentDto[];
}

export async function onSuggestedFragments(callback: (payload: SuggestedFragmentsPayload) => void) {
  return await listen<SuggestedFragmentsPayload>('suggested-fragments', (event) => callback(event.payload));
}

export async function onPipelineProgress(callback: (payload: PipelineProgressPayload) => void) {
  return await listen<PipelineProgressPayload>('pipeline-progress', (event) => callback(event.payload));
}

export async function onRenderProgress(callback: (payload: { percent: number; stage: string }) => void) {
  return await listen<{ percent: number; stage: string }>('render-progress', (event) => callback(event.payload));
}

export interface PipelineLogPayload {
  level: 'info' | 'warn' | 'error' | 'debug';
  stage: string;
  message: string;
  timestamp: string;
}

export async function onPipelineLog(callback: (payload: PipelineLogPayload) => void) {
  return await listen<PipelineLogPayload>('pipeline-log', (event) => callback(event.payload));
}

// РАЗДЕЛ: ИНТЕЛЛЕКТУАЛЬНЫЕ МОДЕЛИ И ИХ ЗАГРУЗКА
export interface ModelInfo {
  id: string;
  name: string;
  filename: string;
  url: string;
  size_mb: number;
  description: string;
  category: 'whisper' | 'llm' | 'vision' | string;
  params: string;
  exists: boolean;
  local_path: string;
  mmproj_filename?: string;
  mmproj_url?: string;
  mmproj_size_mb?: number;
}

export interface ModelDownloadProgressPayload {
  model_id: string;
  percent: number;
  downloaded_bytes: number;
  total_bytes: number;
  status: 'downloading' | 'completed' | 'failed' | 'cancelled';
}

export async function getModelsStatus(): Promise<ModelInfo[]> {
  try {
    return await invoke<ModelInfo[]>('get_models_status');
  } catch (err: unknown) {
    throw new Error(`Ошибка получения статуса моделей: ${formatErrorMessage(err)}`);
  }
}

export async function downloadModel(modelId: string): Promise<string> {
  try {
    return await invoke<string>('download_model', { modelId });
  } catch (err: unknown) {
    throw new Error(`Ошибка запуска скачивания: ${formatErrorMessage(err)}`);
  }
}

export async function deleteModel(modelId: string): Promise<string> {
  try {
    return await invoke<string>('delete_model', { modelId });
  } catch (err: unknown) {
    throw new Error(`Ошибка удаления модели: ${formatErrorMessage(err)}`);
  }
}

export async function cancelModelDownload(modelId: string): Promise<string> {
  try {
    return await invoke<string>('cancel_model_download', { modelId });
  } catch (err: unknown) {
    throw new Error(`Ошибка отмены загрузки: ${formatErrorMessage(err)}`);
  }
}

export async function onModelDownloadProgress(callback: (payload: ModelDownloadProgressPayload) => void) {
  return await listen<ModelDownloadProgressPayload>('model-download-progress', (event) => callback(event.payload));
}

export interface VideoMetadata {
  duration: number;
  codec?: string;
  size?: number;
}

export async function getVideoMetadata(videoPath: string): Promise<VideoMetadata> {
  return await invoke<VideoMetadata>('get_video_metadata', { videoPath });
}

export async function cancelPipeline(): Promise<void> {
  try {
    await invoke('cancel_pipeline');
  } catch (err: unknown) {
    console.warn('cancel_pipeline invoke error:', formatErrorMessage(err));
  }
}

export async function openFolder(folderPath: string): Promise<string> {
  return await invoke<string>('open_folder', { folderPath });
}

export async function getOpenAiProfiles(): Promise<OpenAiProfile[]> {
  if (!isTauriRuntime()) {
    return [];
  }
  try {
    return await invoke<OpenAiProfile[]>('get_openai_profiles');
  } catch (err: unknown) {
    console.warn('Не удалось получить профили OpenAI из Rust-бэкенда:', formatErrorMessage(err));
    return [];
  }
}

export async function saveOpenAiProfiles(profiles: OpenAiProfile[]): Promise<string> {
  if (!isTauriRuntime()) {
    return '';
  }
  try {
    return await invoke<string>('save_openai_profiles', { profiles });
  } catch (err: unknown) {
    throw new Error(`Ошибка сохранения профилей OpenAI: ${formatErrorMessage(err)}`);
  }
}

export async function fetchOpenAiModels(baseUrl: string, apiKey: string): Promise<string[]> {
  if (!isTauriRuntime()) {
    throw new Error('Запрос списка моделей OpenAI выполняется только через защищенный Rust-бэкенд (Tauri IPC)');
  }
  try {
    return await invoke<string[]>('fetch_openai_models', { baseUrl, apiKey });
  } catch (err: unknown) {
    throw new Error(formatErrorMessage(err));
  }
}

export async function renderBumperPreview(
  item: BumperItemConfig,
  isVertical: boolean,
  sourceVideoPath: string,
  presetId: string,
  bumperType: 'preroll' | 'postroll'
): Promise<string> {
  if (!isTauriRuntime()) {
    return 'demo_bumper_preview.mp4';
  }
  try {
    return await invoke<string>('render_bumper_preview', {
      item,
      isVertical,
      sourceVideoPath,
      presetId,
      bumperType,
    });
  } catch (err: unknown) {
    throw new Error(`Ошибка генерации заставки: ${formatErrorMessage(err)}`);
  }
}

export async function probeAudioFileDuration(audioPath: string): Promise<number> {
  if (!isTauriRuntime()) {
    return 2.5;
  }
  try {
    return await invoke<number>('probe_audio_file_duration', { audioPath });
  } catch (err: unknown) {
    console.warn('Не удалось определить длину аудио через FFmpeg:', formatErrorMessage(err));
    return 2.5;
  }
}

export async function exportBumperPresetBundle(preset: BumperPreset, destPath: string): Promise<string> {
  if (!isTauriRuntime()) {
    throw new Error('Используйте браузерную выгрузку .vcpb');
  }
  try {
    return await invoke<string>('export_bumper_preset_bundle', { preset, destPath });
  } catch (err: unknown) {
    throw new Error(`Ошибка экспорта архива пресета: ${formatErrorMessage(err)}`);
  }
}

export async function importBumperPresetBundle(archivePath: string): Promise<BumperPreset> {
  if (!isTauriRuntime()) {
    throw new Error('Используйте браузерный импорт .vcpb');
  }
  try {
    return await invoke<BumperPreset>('import_bumper_preset_bundle', { archivePath });
  } catch (err: unknown) {
    throw new Error(`Ошибка импорта архива пресета: ${formatErrorMessage(err)}`);
  }
}
