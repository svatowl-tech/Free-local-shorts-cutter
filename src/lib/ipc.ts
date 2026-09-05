import { invoke } from '@tauri-apps/api/core';
import { listen, type Event } from '@tauri-apps/api/event';

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

export interface RenderConfig {
  vertical_format: boolean;
  subtitle_style?: SubtitleStyle;
  hw_accel: boolean;
}

export interface PipelineConfig {
  asr_model_path?: string;
  vision_model_path?: string;
  text_engine: string;
  text_model_name: string;
  ollama_url: string;
  output_dir?: string;
  min_intensity_threshold: number;
  max_clip_duration_sec: number;
  vertical_format: boolean;
  subtitle_style?: SubtitleStyle;
  mode: 'auto' | 'manual' | 'confirm';
}

export interface PipelineProgressPayload {
  percent: number;
  stage: string;
}

export async function extractAudio(videoPath: string, outputAudioPath?: string): Promise<string> {
  try {
    return await invoke<string>('extract_audio', { videoPath, outputAudioPath });
  } catch (err) {
    throw new Error(`Ошибка извлечения аудио: ${err}`);
  }
}

export async function transcribeAudio(audioPath: string, modelPath?: string): Promise<TranscribeResult> {
  try {
    return await invoke<TranscribeResult>('transcribe_audio', { audioPath, modelPath });
  } catch (err) {
    throw new Error(`Ошибка Whisper ASR: ${err}`);
  }
}

export async function analyzeVideo(videoPath: string, modelPath?: string): Promise<SceneCandidate[]> {
  try {
    return await invoke<SceneCandidate[]>('analyze_video', { videoPath, modelPath });
  } catch (err) {
    throw new Error(`Ошибка видеоанализа Qwen3-VL: ${err}`);
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

export async function startPipeline(videoPath: string, config: PipelineConfig): Promise<string[]> {
  try {
    return await invoke<string[]>('start_pipeline', { videoPath, config });
  } catch (err) {
    throw new Error(`Ошибка pipeline: ${err}`);
  }
}

export interface FragmentDto {
  start: number;
  end: number;
}

export async function renderClipsFromFragments(videoPath: string, fragments: FragmentDto[], config: PipelineConfig): Promise<string[]> {
  try {
    return await invoke<string[]>('render_clips_from_fragments', { videoPath, fragments, config });
  } catch (err) {
    throw new Error(`Ошибка рендеринга клипов по фрагментам: ${err}`);
  }
}

export async function generateSubtitlesOnly(videoPath: string, config: PipelineConfig): Promise<string> {
  try {
    return await invoke<string>('generate_subtitles_only', { videoPath, config });
  } catch (err) {
    throw new Error(`Ошибка генерации субтитров: ${err}`);
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

// РАЗДЕЛ: ИНТЕЛЛЕКТУАЛЬНЫЕ МОДЕЛИ И ИХ ЗАГРУЗКА
export interface ModelInfo {
  id: string;
  name: string;
  filename: string;
  url: string;
  size_mb: number;
  description: string;
  exists: boolean;
  local_path: string;
}

export interface ModelDownloadProgressPayload {
  model_id: string;
  percent: number;
  downloaded_bytes: number;
  total_bytes: number;
  status: 'downloading' | 'completed' | 'failed';
}

export async function getModelsStatus(): Promise<ModelInfo[]> {
  try {
    return await invoke<ModelInfo[]>('get_models_status');
  } catch (err) {
    throw new Error(`Ошибка получения статуса моделей: ${err}`);
  }
}

export async function downloadModel(modelId: string): Promise<string> {
  try {
    return await invoke<string>('download_model', { modelId });
  } catch (err) {
    throw new Error(`Ошибка запуска скачивания: ${err}`);
  }
}

export async function onModelDownloadProgress(callback: (payload: ModelDownloadProgressPayload) => void) {
  return await listen<ModelDownloadProgressPayload>('model-download-progress', (event) => callback(event.payload));
}

