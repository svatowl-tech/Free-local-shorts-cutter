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
}

export interface CustomSubtitlesParseResult {
  file_path: string;
  total_lines: number;
  characters: SubtitleCharacterInfo[];
}

export interface BumperItemConfig {
  enabled: boolean;
  mode: 'custom_video' | 'generate';
  custom_video_path?: string | null;
  custom_audio_path?: string | null;
  title: string;
  subtitle?: string | null;
  duration_sec: number;
  bg_image_path?: string | null;
  logo_image_path?: string | null;
  logo_scale: number;
  logo_x_percent: number;
  logo_y_percent: number;
  title_color: string;
  title_size: number;
  title_x_percent: number;
  title_y_percent: number;
  transition: 'crossfade' | 'fade_black' | 'cut';
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

export async function parseCustomSubtitles(filePath: string): Promise<CustomSubtitlesParseResult> {
  try {
    return await invoke<CustomSubtitlesParseResult>('parse_custom_subtitles', { filePath });
  } catch (err: unknown) {
    throw new Error(`Ошибка парсинга субтитров: ${formatErrorMessage(err)}`);
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
  presetId: String,
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



