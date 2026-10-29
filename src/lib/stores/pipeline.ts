// src/lib/stores/pipeline.ts
import { writable, get } from 'svelte/store';
import { 
  startPipeline, 
  renderClipsFromFragments,
  onFfmpegProgress, 
  onWhisperProgress, 
  onAnalyzerProgress, 
  onPipelineProgress,
  onRenderProgress,
  onSuggestedFragments,
  onPipelineLog,
  cancelPipeline as ipcCancelPipeline,
  openFolder as ipcOpenFolder,
  type PipelineLogPayload,
  type PipelineConfig,
  type SubtitleStyle
} from '../ipc';
import { fragmentsStore, type Fragment } from './fragments';

export interface SelectedVideo {
  name: string;
  path: string;
  size: number;
  duration?: number;
  objectUrl?: string;
}

export interface StageDetails {
  ffmpeg?: { currentTime: number; duration: number; percent: number };
  whisper?: { percent: number; stage: string };
  analyzer?: { percent: number; stage: string; currentFrame: number; totalFrames: number };
  renderer?: { percent: number; stage: string };
}

export interface LogEntry {
  id: string;
  timestamp: string;
  stage: string;
  level: 'info' | 'warn' | 'error' | 'debug';
  message: string;
}

export interface PipelineState {
  video: SelectedVideo | null;
  config: PipelineConfig;
  status: 'idle' | 'running' | 'completed' | 'failed';
  currentStage: string;
  percentage: number;
  stageDetails: StageDetails;
  clips: string[];
  error: string | null;
  logs: string[];
  detailedLogs: LogEntry[];
}

const DEFAULT_CONFIG: PipelineConfig = {
  min_intensity_threshold: 0.6,
  max_clip_duration_sec: 15,
  vertical_format: true,
  asr_model_path: 'models/ggml-base.bin',
  vision_model_path: 'models/qwen-vl-2b.gguf',
  text_engine: 'llama',
  text_model_name: 'qwen-1.5b-chat.gguf',
  ollama_url: 'http://localhost:11434',
  output_dir: 'output_cuts/',
  subtitle_style: {
    font_name: 'Arial',
    font_size: 18,
    color: '#FFFFFF',
    placement: 'bottom'
  },
  mode: 'confirm'
};

// Безопасная обертка регистрации слушателя Tauri с таймаутом (не блокирует конвейер)
async function safeListen<T>(
  registerFn: () => Promise<() => void>,
  label: string
): Promise<() => void> {
  try {
    const timeoutPromise = new Promise<() => void>((resolve) => {
      setTimeout(() => {
        console.warn(`[IPC] Таймаут подписки на событие ${label}, продолжаем запуск...`);
        resolve(() => {});
      }, 2000);
    });
    return await Promise.race([registerFn(), timeoutPromise]);
  } catch (e) {
    console.warn(`[IPC] Не удалось подписаться на событие ${label}:`, e);
    return () => {};
  }
}

const createPipelineStore = () => {
  const store = writable<PipelineState>({
    video: null,
    config: { ...DEFAULT_CONFIG },
    status: 'idle',
    currentStage: '',
    percentage: 0,
    stageDetails: {},
    clips: [],
    error: null,
    logs: [],
    detailedLogs: []
  });
  const { subscribe, set, update } = store;

  // Вспомогательная функция глубокого логирования с выводом в системную консоль браузера
  const appendLog = (params: { message: string; stage?: string; level?: 'info' | 'warn' | 'error' | 'debug'; timestamp?: string }) => {
    const ts = params.timestamp || new Date().toLocaleTimeString();
    const stage = params.stage || 'Frontend';
    const level = params.level || 'info';
    const message = params.message;

    // Вывод в системную консоль браузера (DevTools)
    const prefix = `[${stage}][${ts}]`;
    if (level === 'error') {
      console.error(`%c${prefix}`, 'color: #ef4444; font-weight: bold;', message);
    } else if (level === 'warn') {
      console.warn(`%c${prefix}`, 'color: #f59e0b; font-weight: bold;', message);
    } else if (level === 'debug') {
      console.debug(`%c${prefix}`, 'color: #94a3b8;', message);
    } else {
      console.log(`%c${prefix}`, 'color: #3b82f6; font-weight: bold;', message);
    }

    const formattedLine = `[${ts}] [${stage}] ${message}`;
    const entry: LogEntry = {
      id: Math.random().toString(36).substring(2, 9),
      timestamp: ts,
      stage,
      level,
      message
    };

    update(s => ({
      ...s,
      logs: [...s.logs.slice(-1500), formattedLine],
      detailedLogs: [...s.detailedLogs.slice(-1500), entry]
    }));
  };

  const addLog = (text: string, stage = 'App', level: 'info' | 'warn' | 'error' | 'debug' = 'info') => {
    appendLog({ message: text, stage, level });
  };

  // Функция автоматической демо-симуляции процесса при запуске в браузере (не Tauri)
  const runBrowserSimulation = async (videoName: string, state: PipelineState) => {
    update(s => ({
      ...s,
      status: 'running',
      currentStage: 'Инициализация пайплайна',
      percentage: 2,
      error: null,
      clips: [],
      stageDetails: {}
    }));
    addLog(`Запущен демонстрационный симулятор пайплайна для: ${videoName}`);

    if (state.config.mode !== 'manual') {
      // Шаг 1: Извлечение аудио через FFmpeg
      await new Promise(resolve => setTimeout(resolve, 1500));
      update(s => ({
        ...s,
        currentStage: 'Извлечение аудио дорожки (FFmpeg)',
        percentage: 10,
        stageDetails: { ...s.stageDetails, ffmpeg: { currentTime: 0, duration: 180, percent: 0 } }
      }));
      addLog('Запуск sidecar-процесса FFmpeg для экспорта аудио в PCM WAV...');

      for (let p = 10; p <= 35; p += 5) {
        await new Promise(resolve => setTimeout(resolve, 300));
        const curTime = Math.floor((p - 10) * 7.2);
        update(s => ({
          ...s,
          percentage: p,
          stageDetails: { ...s.stageDetails, ffmpeg: { currentTime: curTime, duration: 180, percent: ((p - 10) / 25) * 100 } }
        }));
        addLog(`FFmpeg: Обработано ${curTime}с из 180с (${Math.round(((p - 10) / 25) * 100)}%)`);
      }

      addLog('Аудио дорожка успешно извлечена во временный файл: temp_audio.wav');

      // Шаг 2: Whisper ASR
      await new Promise(resolve => setTimeout(resolve, 500));
      update(s => ({
        ...s,
        currentStage: 'Распознавание речи и транскрипция (Whisper ASR)',
        percentage: 40,
        stageDetails: { ...s.stageDetails, whisper: { percent: 0, stage: 'Загрузка нейросети' } }
      }));
      addLog('Запуск whisper.cpp-модели. Загрузка параметров весов модели...');

      await new Promise(resolve => setTimeout(resolve, 500));
      update(s => ({
        ...s,
        stageDetails: { ...s.stageDetails, whisper: { percent: 15, stage: 'Декодирование речи' } }
      }));
      addLog('Whisper: Модель загружена. Начало распознавания аудиопотока...');

      for (let pct = 30; pct <= 100; pct += 30) {
        await new Promise(resolve => setTimeout(resolve, 300));
        const adjustedPct = Math.min(pct, 100);
        update(s => ({
          ...s,
          percentage: 40 + Math.floor((adjustedPct / 100) * 15),
          stageDetails: { ...s.stageDetails, whisper: { percent: adjustedPct, stage: 'Декодирование речи' } }
        }));
      }
      addLog(`Whisper ASR успешно завершено. Сгенерирован srt файл субтитров.`);

      // Шаг 3: Qwen3-VL Мультимодальный анализ
      await new Promise(resolve => setTimeout(resolve, 500));
      update(s => ({
        ...s,
        currentStage: 'Интеллектуальный анализ кадров (Qwen3-VL AI)',
        percentage: 60,
        stageDetails: { ...s.stageDetails, analyzer: { percent: 0, stage: 'Поиск интересных моментов', currentFrame: 0, totalFrames: 5400 } }
      }));
      addLog('Запуск инференса Qwen3-VL 2B. Анализ семантической важности сцен...');

      for (let frame = 1000; frame <= 5400; frame += 2000) {
        await new Promise(resolve => setTimeout(resolve, 400));
        const pct = Math.round((frame / 5400) * 100);
        update(s => ({
          ...s,
          percentage: 60 + Math.floor((pct / 100) * 20),
          stageDetails: { ...s.stageDetails, analyzer: { percent: pct, stage: 'Детекция движения и лиц', currentFrame: frame, totalFrames: 5400 } }
        }));
      }
      addLog('Qwen3-VL завершил анализ. Обнаружено несколько сцен для нарезки.');
      
      // Добавляем качественные фрагменты в fragmentsStore
      fragmentsStore.set([
        { id: Math.random().toString(36).substring(2, 9), start: 8, end: 22, title: 'Клип #1 (Интро и диалог)' },
        { id: Math.random().toString(36).substring(2, 9), start: 30, end: 45, title: 'Клип #2 (Кульминация сцены)' },
        { id: Math.random().toString(36).substring(2, 9), start: 55, end: 70, title: 'Клип #3 (Яркий панчлайн)' }
      ]);
      addLog('✓ 3 хайлайта успешно сформированы и перенесены на таймлайн.');

      if (state.config.mode === 'confirm') {
        update(s => ({
          ...s,
          status: 'idle',
          currentStage: 'Ожидание подтверждения',
          percentage: 100
        }));
        addLog('Режим подтверждения: ожидание решения пользователя. Можно редактировать шкалу и нажать "Экспортировать клипы".');
        return;
      }
    }

    // Шаг 4: Рендеринг клипов BMF
    await new Promise(resolve => setTimeout(resolve, 1000));
    update(s => ({
      ...s,
      currentStage: 'Экспорт и рендеринг клипов (BMF Engine)',
      percentage: 85,
      stageDetails: { ...s.stageDetails, renderer: { percent: 0, stage: 'Инициализация кодеков' } }
    }));
    addLog('Запуск Babit Multimedia Framework (BMF) с аппаратным ускорением...');

    for (let clipNo = 1; clipNo <= 3; clipNo++) {
      await new Promise(resolve => setTimeout(resolve, 800));
      const clipPct = Math.round((clipNo / 3) * 100);
      update(s => ({
        ...s,
        percentage: 85 + Math.floor((clipPct / 100) * 14),
        stageDetails: { ...s.stageDetails, renderer: { percent: clipPct, stage: `Генерация клипа ${clipNo}/3` } }
      }));
      addLog(`FFmpeg: Успешно экспортирован клип ${clipNo} из 3`);
    }

    // Завершено
    await new Promise(resolve => setTimeout(resolve, 800));
    const generatedClips = [
      `${state.config.output_dir || 'output_cuts/'}clip_001_highlights.mp4`,
      `${state.config.output_dir || 'output_cuts/'}clip_002_dynamic_action.mp4`,
      `${state.config.output_dir || 'output_cuts/'}clip_003_narrative_peak.mp4`
    ];

    update(s => ({
      ...s,
      status: 'completed',
      currentStage: 'Обработка успешно завершена!',
      percentage: 100,
      clips: generatedClips
    }));
    addLog(`Пайплайн успешно отработал! Сгенерировано клипов: ${generatedClips.length}`);
  };

  return {
    subscribe,
    setVideo: (video: SelectedVideo | null) => {
      update(s => {
        const newState = { ...s, video, clips: [], error: null, percentage: 0, status: 'idle' as const, currentStage: '' };
        return newState;
      });
      if (video) {
        const sizeStr = video.size > 0 
          ? ` (${(video.size / (1024 * 1024)).toFixed(1)} МБ)` 
          : '';
        addLog(`Выбран исходный видеофайл: ${video.name}${sizeStr}`);
      } else {
        addLog('Выбор видеофайла сброшен');
      }
    },
    updateVideoMetadata: (meta: { size?: number; duration?: number; path?: string; objectUrl?: string }) => {
      update(s => {
        if (!s.video) return s;
        return {
          ...s,
          video: {
            ...s.video,
            size: meta.size && meta.size > 0 ? meta.size : s.video.size,
            duration: meta.duration && meta.duration > 0 ? meta.duration : s.video.duration,
            path: meta.path || s.video.path,
            objectUrl: meta.objectUrl !== undefined ? meta.objectUrl : s.video.objectUrl
          }
        };
      });
    },
    updateConfig: (fields: Partial<PipelineConfig>) => {
      update(s => {
        const config = { ...s.config, ...fields };
        return { ...s, config };
      });
      addLog(`Обновлены настройки пайплайна: ${JSON.stringify(fields)}`);
    },
    updateError: (error: string | null) => {
      update(s => ({ ...s, error }));
    },
    cancelPipeline: async () => {
      try {
        await ipcCancelPipeline();
      } catch (err) {
        console.warn('Ошибка вызова cancel_pipeline:', err);
      }
      update(s => ({
        ...s,
        status: 'idle',
        currentStage: 'Операция отменена пользователем',
        percentage: 0,
        stageDetails: {}
      }));
      addLog('Операция отменена пользователем.', 'Pipeline', 'warn');
    },
    openFolder: async (folderPath: string) => {
      try {
        await ipcOpenFolder(folderPath);
      } catch (err) {
        console.error('Ошибка открытия папки:', err);
      }
    },
    reset: () => {
      set({
        video: null,
        config: { ...DEFAULT_CONFIG },
        status: 'idle',
        currentStage: '',
        percentage: 0,
        stageDetails: {},
        clips: [],
        error: null,
        logs: [],
        detailedLogs: []
      });
      addLog('Состояние сброшено к начальным настройкам');
    },
    exportManualClips: async (videoPath: string, fragments: Fragment[], config: PipelineConfig): Promise<string[]> => {
      const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;

      update(s => ({
        ...s,
        status: 'running',
        currentStage: 'Подготовка экспорта Reels (9:16 + Субтитры)...',
        percentage: 5,
        error: null,
        clips: []
      }));
      addLog(`Запуск экспорта ${fragments.length} клипов в формате 9:16 с синхронизированными субтитрами...`);

      if (!isTauri) {
        // Этап 1: Извлечение аудио
        update(s => ({
          ...s,
          currentStage: 'Извлечение аудиодорожки (FFmpeg)...',
          percentage: 20,
          stageDetails: { ...s.stageDetails, ffmpeg: { currentTime: 60, duration: 180, percent: 100 } }
        }));
        addLog('FFmpeg: Аудио успешно извлечено в WAV 16kHz mono');
        await new Promise(resolve => setTimeout(resolve, 600));

        // Этап 2: Транскрипция Whisper для субтитров
        update(s => ({
          ...s,
          currentStage: 'Whisper ASR: распознавание речи и создание субтитров...',
          percentage: 45,
          stageDetails: { ...s.stageDetails, whisper: { percent: 100, stage: 'Генерация SRT субтитров' } }
        }));
        addLog('Whisper: Субтитры успешно сформированы для выделенных диапазонов');
        await new Promise(resolve => setTimeout(resolve, 800));

        // Этап 3: Нарезка и 9:16 кроппинг каждого клипа
        const count = fragments.length || 1;
        for (let clipNo = 1; clipNo <= count; clipNo++) {
          const frag = fragments[clipNo - 1] || { start: 0, end: 15 };
          const dur = (frag.end - frag.start).toFixed(1);
          update(s => ({
            ...s,
            currentStage: `Рендеринг Reels ${clipNo} из ${count} (9:16 Vertical + Субтитры)...`,
            percentage: 50 + Math.floor((clipNo / count) * 45),
            stageDetails: { ...s.stageDetails, renderer: { percent: Math.round((clipNo / count) * 100), stage: `Клип #${clipNo} (${dur}с)` } }
          }));
          addLog(`FFmpeg: Кадрирование 9:16 [crop=ih*(9/16):ih,scale=1080:1920] и вшивание субтитров для клипа #${clipNo}`);
          await new Promise(resolve => setTimeout(resolve, 700));
        }

        const baseDir = config.output_dir || 'output_reels/';
        const generatedClips = fragments.map((f, i) => 
          `${baseDir}reels_clip_${i + 1}_from_${Math.round(f.start)}s_to_${Math.round(f.end)}s_9x16.mp4`
        );

        update(s => ({
          ...s,
          status: 'completed',
          currentStage: 'Экспорт всех Reels успешно завершен!',
          percentage: 100,
          clips: generatedClips
        }));
        addLog(`Экспорт завершен! Готово ${generatedClips.length} клипов в формате 9:16 с субтитрами Whisper.`);
        return generatedClips;
      }

      let lastFfmpegPct = -10;
      let lastWhisperPct = -10;
      let lastRenderPct = -10;

      try {
        const unlistenPipelineLog = await safeListen(
          () => onPipelineLog((p) => {
            appendLog({ message: p.message, stage: p.stage, level: p.level, timestamp: p.timestamp });
          }),
          'pipeline-log'
        );

        const unlistenFfmpeg = await safeListen(
          () => onFfmpegProgress((payload) => {
            update(s => ({
              ...s,
              currentStage: 'Извлечение аудио (FFmpeg)',
              percentage: Math.round(payload.percent * 0.3),
              stageDetails: {
                ...s.stageDetails,
                ffmpeg: {
                  currentTime: payload.current_time,
                  duration: payload.duration,
                  percent: payload.percent
                }
              }
            }));
            const rounded = Math.round(payload.percent);
            if (rounded - lastFfmpegPct >= 5 || rounded === 100) {
              lastFfmpegPct = rounded;
              addLog(`FFmpeg: Извлечение аудио ${rounded}%`);
            }
          }),
          'ffmpeg-progress'
        );

        const unlistenWhisper = await safeListen(
          () => onWhisperProgress((payload) => {
            const stageLabel = payload.stage === 'cached'
              ? 'Используются готовые субтитры из кэша проекта'
              : payload.stage === 'loading_model'
                ? 'Загрузка модели'
                : 'Распознавание речи';
            update(s => ({
              ...s,
              currentStage: `Whisper ASR: ${stageLabel}`,
              percentage: 30 + Math.round(payload.percent * 0.3),
              stageDetails: {
                ...s.stageDetails,
                whisper: {
                  percent: payload.percent,
                  stage: payload.stage
                }
              }
            }));
            const rounded = Math.round(payload.percent);
            if (rounded - lastWhisperPct >= 5 || rounded === 100) {
              lastWhisperPct = rounded;
              addLog(`Whisper: ${stageLabel} - ${rounded}%`);
            }
          }),
          'whisper-progress'
        );

        const unlistenRender = await safeListen(
          () => onRenderProgress((payload) => {
            update(s => ({
              ...s,
              currentStage: `Рендеринг Reels 9:16: ${payload.stage}`,
              percentage: 60 + Math.round(payload.percent * 0.4),
              stageDetails: {
                ...s.stageDetails,
                renderer: {
                  percent: payload.percent,
                  stage: payload.stage
                }
              }
            }));
            const rounded = Math.round(payload.percent);
            if (rounded - lastRenderPct >= 5 || rounded === 100) {
              lastRenderPct = rounded;
              addLog(`Рендеринг: ${payload.stage} - ${rounded}%`);
            }
          }),
          'render-progress'
        );

        const unlistenPipeline = await safeListen(
          () => onPipelineProgress((payload) => {
            update(s => ({
              ...s,
              currentStage: payload.stage,
              percentage: 60 + Math.round(payload.percent * 0.1)
            }));
          }),
          'pipeline-progress'
        );

        const formattedFragments = fragments.map(f => ({ start: f.start, end: f.end }));

        const clipPaths = await renderClipsFromFragments(
            videoPath,
            formattedFragments,
            { ...config, vertical_format: true }
        );

        unlistenPipelineLog();
        unlistenFfmpeg();
        unlistenWhisper();
        unlistenRender();
        unlistenPipeline();

        update(s => ({
          ...s,
          status: 'completed',
          currentStage: 'Экспорт Reels успешно завершен!',
          percentage: 100,
          clips: clipPaths
        }));
        addLog(`Экспорт успешно завершен! Сгенерировано Reels: ${clipPaths.length}`);
        return clipPaths;

      } catch (err: any) {
        const errorMsg = err.message || String(err);
        update(s => ({
          ...s,
          status: 'failed',
          currentStage: 'Ошибка при экспорте фрагментов',
          error: errorMsg
        }));
        addLog(`ОШИБКА ЭКСПОРТА: ${errorMsg}`, 'Pipeline', 'error');
        return [];
      }
    },
    exportAndRenderClips: async (fragments: any[]) => {
      const currentState = get(store);
      if (!currentState.video) return;

      return await exportManualClips(
        currentState.video.path,
        fragments,
        currentState.config
      );
    },
    startPipelineProcess: async () => {
      const currentState = get(store);

      if (!currentState.video) {
        update(s => ({ ...s, error: 'Пожалуйста, выберите видеофайл для обработки!' }));
        return;
      }

      const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;

      if (!isTauri) {
        // Мы в веб-браузере (в нашей песочнице), запускаем красивый продвинутый симулятор
        runBrowserSimulation(currentState.video.name, currentState);
        return;
      }

      // Проверяем, что передан полный системный путь к видеофайлу, а не только имя файла из браузерного input
      const hasFullPath = currentState.video.path.includes('/') || currentState.video.path.includes('\\');
      if (!hasFullPath) {
        const msg = `Не удалось получить полный системный путь к файлу "${currentState.video.name}". Пожалуйста, выберите видео через диалог выбора файла (клик по области загрузки) или перетащите файл в окно программы.`;
        update(s => ({ ...s, error: msg }));
        addLog(msg, 'App', 'error');
        return;
      }

      // Если в Tauri, подписываемся на настоящие события бэкенда
      update(s => ({
        ...s,
        status: 'running',
        currentStage: 'Инициализация...',
        percentage: 1,
        error: null,
        clips: [],
        stageDetails: {}
      }));
      addLog('Запуск профессионального видео-пайплайна в Rust-конвейере...');

      let lastFfmpegPct = -10;
      let lastWhisperPct = -10;
      let lastRenderPct = -10;

      try {
        // Подключаем сквозной сбор всех логов stdout/stderr бэкенда
        const unlistenPipelineLog = await safeListen(
          () => onPipelineLog((p) => {
            appendLog({ message: p.message, stage: p.stage, level: p.level, timestamp: p.timestamp });
          }),
          'pipeline-log'
        );

        // Подключаем слушатели событий прогресса Tauri
        const unlistenFfmpeg = await safeListen(
          () => onFfmpegProgress((payload) => {
            update(s => ({
              ...s,
              currentStage: 'Извлечение аудио (FFmpeg)',
              percentage: Math.max(2, Math.round(payload.percent * 0.20)), // 2 - 20%
              stageDetails: {
                ...s.stageDetails,
                ffmpeg: {
                  currentTime: payload.current_time,
                  duration: payload.duration,
                  percent: payload.percent
                }
              }
            }));
            const rounded = Math.round(payload.percent);
            if (rounded - lastFfmpegPct >= 5 || rounded === 100) {
              lastFfmpegPct = rounded;
              addLog(`FFmpeg: Прогресс ${rounded}% (Осталось ${Math.max(0, Math.round(payload.duration - payload.current_time))} сек)`);
            }
          }),
          'ffmpeg-progress'
        );

        const unlistenWhisper = await safeListen(
          () => onWhisperProgress((payload) => {
            update(s => ({
              ...s,
              currentStage: `Whisper ASR: ${payload.stage === 'loading_model' ? 'Загрузка модели' : 'Распознавание'}`,
              percentage: 20 + Math.round(payload.percent * 0.30), // 20% - 50%
              stageDetails: {
                ...s.stageDetails,
                whisper: {
                  percent: payload.percent,
                  stage: payload.stage
                }
              }
            }));
            const rounded = Math.round(payload.percent);
            if (rounded - lastWhisperPct >= 5 || rounded === 100) {
              lastWhisperPct = rounded;
              addLog(`Whisper [${payload.stage}]: Распознано ${rounded}%`);
            }
          }),
          'whisper-progress'
        );

        const unlistenAnalyzer = await safeListen(
          () => onAnalyzerProgress((payload) => {
            update(s => ({
              ...s,
              currentStage: `Анализ сцен: ${payload.stage}`,
              percentage: 50 + Math.round(payload.percent * 0.25), // 50% - 75%
              stageDetails: {
                ...s.stageDetails,
                analyzer: {
                  percent: payload.percent,
                  stage: payload.stage,
                  currentFrame: payload.current_frame,
                  totalFrames: payload.total_frames
                }
              }
            }));
            addLog(`Анализ сцен: Шаг ${payload.stage} (${payload.current_frame}/${payload.total_frames})`);
          }),
          'analyzer-progress'
        );

        const unlistenPipeline = await safeListen(
          () => onPipelineProgress((payload) => {
            update(s => ({
              ...s,
              currentStage: payload.stage,
              percentage: Math.max(s.percentage, Math.round(payload.percent))
            }));
            addLog(`Пайплайн [${payload.stage}]: Прогресс ${Math.round(payload.percent)}%`);
          }),
          'pipeline-progress'
        );

        const unlistenSuggested = await safeListen(
          () => onSuggestedFragments((payload) => {
            if (payload.fragments && payload.fragments.length > 0) {
              const mapped = payload.fragments.map((f, idx) => ({
                id: Math.random().toString(36).substring(2, 9),
                start: Math.round(f.start * 10) / 10,
                end: Math.round(f.end * 10) / 10,
                title: `Клип #${idx + 1}`
              }));
              fragmentsStore.set(mapped);
              addLog(`Получено ${mapped.length} фрагментов, предложенных ИИ.`);
            }
          }),
          'suggested-fragments'
        );

        const unlistenRender = await safeListen(
          () => onRenderProgress((payload) => {
            update(s => ({
              ...s,
              currentStage: `Рендеринг (FFmpeg): ${payload.stage}`,
              percentage: 75 + Math.round(payload.percent * 0.25), // 75% - 100%
              stageDetails: {
                ...s.stageDetails,
                renderer: {
                  percent: payload.percent,
                  stage: payload.stage
                }
              }
            }));
            const rounded = Math.round(payload.percent);
            if (rounded - lastRenderPct >= 5 || rounded === 100) {
              lastRenderPct = rounded;
              addLog(`FFmpeg Рендеринг: ${payload.stage} - ${rounded}%`);
            }
          }),
          'render-progress'
        );

        addLog(`Отправка IPC-запроса start_pipeline для файла: ${currentState.video.path}`, 'IPC', 'info');

        // Запускаем сам пайплайн
        const result = await startPipeline(currentState.video.path, currentState.config);

        // Гарантированно и синхронно переносим полученные хайлайты в fragmentsStore
        if (result.suggested_fragments && result.suggested_fragments.length > 0) {
          const mapped = result.suggested_fragments.map((f, idx) => ({
            id: Math.random().toString(36).substring(2, 9),
            start: Math.round(f.start * 10) / 10,
            end: Math.round(f.end * 10) / 10,
            title: `Клип #${idx + 1}`
          }));
          fragmentsStore.set(mapped);
          addLog(`✓ Успешно выделено ${mapped.length} хайлайт-фрагментов на таймлайне.`);
        } else {
          addLog('Пайплайн завершил отбор: сформировано 0 фрагментов.', 'Pipeline', 'warn');
        }

        // Даем слушателям событий завершить чтение потока перед отпиской
        setTimeout(() => {
          unlistenPipelineLog();
          unlistenFfmpeg();
          unlistenWhisper();
          unlistenAnalyzer();
          unlistenPipeline();
          unlistenSuggested();
          unlistenRender();
        }, 1200);

        if (currentState.config.mode === 'confirm') {
          update(s => ({
            ...s,
            status: 'idle',
            currentStage: 'Кандидаты загружены. Ожидание подтверждения экспорта',
            percentage: 100
          }));
          addLog('ИИ завершил анализ. Фрагменты перенесены на таймлайн для подтверждения. Нажмите "Экспортировать клипы" после редактирования.');
        } else {
          update(s => ({
            ...s,
            status: 'completed',
            currentStage: 'Успешно завершено!',
            percentage: 100,
            clips: result.clips || []
          }));
          addLog(`Работа завершена! Получено клипов: ${(result.clips || []).length}`);
        }

      } catch (err: any) {
        const errorMsg = err.message || String(err);
        update(s => ({
          ...s,
          status: 'failed',
          currentStage: 'Ошибка выполнения пайплайна',
          error: errorMsg
        }));
        addLog(`КРИТИЧЕСКАЯ ОШИБКА: ${errorMsg}`, 'Pipeline', 'error');
      }
    },
    copyLogsToClipboard: async () => {
      const currentState = get(store);
      const text = currentState.logs.join('\n');
      if (typeof navigator !== 'undefined' && navigator.clipboard) {
        await navigator.clipboard.writeText(text);
        appendLog({ message: 'Все логи успешно скопированы в буфер обмена', stage: 'UI', level: 'info' });
      }
    },
    downloadLogsAsFile: () => {
      const currentState = get(store);
      const header = `=== Video Cutter Pro - Журнал логирования ===\nДата: ${new Date().toLocaleString()}\nВидео: ${currentState.video ? currentState.video.path : 'Не выбрано'}\nКонфигурация: ${JSON.stringify(currentState.config, null, 2)}\n===========================================\n\n`;
      const text = header + currentState.logs.join('\n');
      const blob = new Blob([text], { type: 'text/plain;charset=utf-8' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `video_cutter_logs_${new Date().toISOString().replace(/[:.]/g, '-')}.txt`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      appendLog({ message: 'Лог-файл успешно сохранен на диск', stage: 'UI', level: 'info' });
    },
    clearLogs: () => {
      update(s => ({ ...s, logs: [], detailedLogs: [] }));
    }
  };
};

export const pipelineStore = createPipelineStore();

export async function copyLogsToClipboard(): Promise<void> {
  return await pipelineStore.copyLogsToClipboard();
}

export function downloadLogsAsFile(): void {
  pipelineStore.downloadLogsAsFile();
}

export function clearLogs(): void {
  pipelineStore.clearLogs();
}

export async function exportManualClips(videoPath: string, fragments: Fragment[], config: PipelineConfig): Promise<string[]> {
  return await pipelineStore.exportManualClips(videoPath, fragments, config);
}

export async function cancelPipeline(): Promise<void> {
  return await pipelineStore.cancelPipeline();
}

export async function openFolder(folderPath: string): Promise<void> {
  return await pipelineStore.openFolder(folderPath);
}
