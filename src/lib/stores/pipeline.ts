// src/lib/stores/pipeline.ts
import { writable } from 'svelte/store';
import { 
  startPipeline, 
  renderClipsFromFragments,
  onFfmpegProgress, 
  onWhisperProgress, 
  onAnalyzerProgress, 
  onPipelineProgress,
  onRenderProgress,
  onSuggestedFragments,
  type PipelineConfig,
  type SubtitleStyle
} from '../ipc';
import { fragmentsStore, type Fragment } from './fragments';

export interface SelectedVideo {
  name: string;
  path: string;
  size: number;
  duration?: number;
}

export interface StageDetails {
  ffmpeg?: { currentTime: number; duration: number; percent: number };
  whisper?: { percent: number; stage: string };
  analyzer?: { percent: number; stage: string; currentFrame: number; totalFrames: number };
  renderer?: { percent: number; stage: string };
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
}

const DEFAULT_CONFIG: PipelineConfig = {
  min_intensity_threshold: 0.6,
  max_clip_duration_sec: 15,
  vertical_format: true,
  asr_model_path: 'models/ggml-base.bin',
  vision_model_path: 'models/qwen-vl-2b.gguf',
  text_engine: 'auto',
  text_model_name: 'qwen2.5:1.5b',
  ollama_url: 'http://localhost:11434',
  output_dir: 'output_cuts/',
  subtitle_style: {
    font_name: 'Inter',
    font_size: 24,
    color: '#FFFFFF',
    placement: 'bottom'
  },
  mode: 'confirm'
};

const createPipelineStore = () => {
  const { subscribe, set, update } = writable<PipelineState>({
    video: null,
    config: { ...DEFAULT_CONFIG },
    status: 'idle',
    currentStage: '',
    percentage: 0,
    stageDetails: {},
    clips: [],
    error: null,
    logs: []
  });

  // Вспомогательная функция добавления логов
  const addLog = (text: string) => {
    update(s => ({
      ...s,
      logs: [...s.logs, `[${new Date().toLocaleTimeString()}] ${text}`]
    }));
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
      
      // Вкидываем фейковые сцены в fragmentsStore
      fragmentsStore.set([
        { id: Math.random().toString(), start: 10, end: 20 },
        { id: Math.random().toString(), start: 35, end: 42 }
      ]);
      addLog('Фрагменты добавлены на таймлайн.');

      if (state.config.mode === 'confirm') {
        update(s => ({
          ...s,
          status: 'idle',
          currentStage: 'Ожидание подтверждения',
          percentage: 0
        }));
        addLog('Режим подтверждения: ожидание решения пользователя. Можно редактировать шкалу и нажать "Экспортировать".');
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
      addLog(`BMF: Успешно экспортирован клип ${clipNo} из 3`);
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
        addLog(`Выбран исходный видеофайл: ${video.name} (${Math.round(video.size / (1024 * 1024))} МБ)`);
      } else {
        addLog('Выбор видеофайла сброшен');
      }
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
        logs: []
      });
      addLog('Состояние сброшено к начальным настройкам');
    },
    exportManualClips: async (videoPath: string, fragments: Fragment[], config: PipelineConfig): Promise<string[]> => {
      const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;

      update(s => ({
        ...s,
        status: 'running',
        currentStage: 'Инициализация и запуск конвейера экспорта...',
        percentage: 0,
        error: null,
        clips: []
      }));
      addLog('Запуск ручной/подтвержденной нарезки клипов...');

      if (!isTauri) {
        update(s => ({
          ...s,
          status: 'running',
          currentStage: 'Экспорт и рендеринг клипов (BMF Engine)',
          percentage: 85,
          stageDetails: { ...s.stageDetails, renderer: { percent: 0, stage: 'Инициализация кодеков' } }
        }));
        addLog('Запуск Babit Multimedia Framework (BMF) для экспорта нарезки...');

        // Симуляция рендеринга
        const count = fragments.length || 1;
        for (let clipNo = 1; clipNo <= count; clipNo++) {
          await new Promise(resolve => setTimeout(resolve, 800));
          const clipPct = Math.round((clipNo / count) * 100);
          update(s => ({
            ...s,
            percentage: 85 + Math.floor((clipPct / 100) * 14),
            stageDetails: { ...s.stageDetails, renderer: { percent: clipPct, stage: `Генерация клипа ${clipNo}/${count}` } }
          }));
          addLog(`BMF: Успешно экспортирован клип ${clipNo} из ${count}`);
        }

        await new Promise(resolve => setTimeout(resolve, 800));
        const generatedClips = Array.from({ length: count }, (_, i) => `${config.output_dir || 'output_cuts/'}clip_00${i+1}_export.mp4`);

        update(s => ({
          ...s,
          status: 'completed',
          currentStage: 'Экспорт успешно завершен!',
          percentage: 100,
          clips: generatedClips
        }));
        addLog(`Пайплайн успешно отработал! Сгенерировано клипов: ${generatedClips.length}`);
        return generatedClips;
      }

      try {
        const unlistenFfmpeg = await onFfmpegProgress((payload) => {
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
          addLog(`FFmpeg: Извлечение аудио ${Math.round(payload.percent)}%`);
        });

        const unlistenWhisper = await onWhisperProgress((payload) => {
          update(s => ({
            ...s,
            currentStage: `Whisper ASR: ${payload.stage === 'loading_model' ? 'Загрузка модели' : 'Распознавание'}`,
            percentage: 30 + Math.round(payload.percent * 0.3),
            stageDetails: {
              ...s.stageDetails,
              whisper: {
                percent: payload.percent,
                stage: payload.stage
              }
            }
          }));
          addLog(`Whisper: ${payload.stage} - ${Math.round(payload.percent)}%`);
        });

        const unlistenRender = await onRenderProgress((payload) => {
          update(s => ({
            ...s,
            currentStage: `Рендеринг BMF: ${payload.stage}`,
            percentage: 60 + Math.round(payload.percent * 0.4),
            stageDetails: {
              ...s.stageDetails,
              renderer: {
                percent: payload.percent,
                stage: payload.stage
              }
            }
          }));
          addLog(`BMF Рендеринг: ${payload.stage} - ${Math.round(payload.percent)}%`);
        });

        const unlistenPipeline = await onPipelineProgress((payload) => {
          update(s => ({
            ...s,
            currentStage: payload.stage,
            percentage: 60 + Math.round(payload.percent * 0.1)
          }));
        });

        const formattedFragments = fragments.map(f => ({ start: f.start, end: f.end }));

        const clipPaths = await renderClipsFromFragments(
          videoPath,
          formattedFragments,
          config
        );

        unlistenFfmpeg();
        unlistenWhisper();
        unlistenRender();
        unlistenPipeline();

        update(s => ({
          ...s,
          status: 'completed',
          currentStage: 'Успешно экспортировано!',
          percentage: 100,
          clips: clipPaths
        }));
        addLog(`Экспорт успешно завершен! Сгенерировано клипов: ${clipPaths.length}`);
        return clipPaths;

      } catch (err: any) {
        const errorMsg = err.message || String(err);
        update(s => ({
          ...s,
          status: 'failed',
          currentStage: 'Ошибка при экспорте фрагментов',
          error: errorMsg
        }));
        addLog(`ОШИБКА ЭКСПОРТА: ${errorMsg}`);
        throw err;
      }
    },
    exportAndRenderClips: async (fragments: any[]) => {
      let currentState!: PipelineState;
      subscribe(s => { currentState = s; })();

      if (!currentState.video) return;

      return await pipelineStore.exportManualClips(
        currentState.video.path,
        fragments,
        currentState.config
      );
    },
    startPipelineProcess: async () => {
      let currentState!: PipelineState;
      subscribe(s => { currentState = s; })();

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


      // Если в Tauri, подписываемся на настоящие события бэкенда
      update(s => ({
        ...s,
        status: 'running',
        currentStage: 'Инициализация...',
        percentage: 0,
        error: null,
        clips: []
      }));
      addLog('Запуск профессионального видео-пайплайна в Rust-конвейере...');

      try {
        // Подключаем слушатели событий прогресса Tauri
        const unlistenFfmpeg = await onFfmpegProgress((payload) => {
          update(s => ({
            ...s,
            currentStage: 'Извлечение аудио (FFmpeg)',
            percentage: Math.round(payload.percent * 0.25), // займет первые 25% общего прогресса
            stageDetails: {
              ...s.stageDetails,
              ffmpeg: {
                currentTime: payload.current_time,
                duration: payload.duration,
                percent: payload.percent
              }
            }
          }));
          addLog(`FFmpeg: Прогресс ${Math.round(payload.percent)}% (Осталось ${Math.round(payload.duration - payload.current_time)} сек)`);
        });

        const unlistenWhisper = await onWhisperProgress((payload) => {
          update(s => ({
            ...s,
            currentStage: `Whisper ASR: ${payload.stage === 'loading_model' ? 'Загрузка модели' : 'Распознавание'}`,
            percentage: 25 + Math.round(payload.percent * 0.2), // займет 25% - 45% общего прогресса
            stageDetails: {
              ...s.stageDetails,
              whisper: {
                percent: payload.percent,
                stage: payload.stage
              }
            }
          }));
          addLog(`Whisper [${payload.stage}]: Распознано ${Math.round(payload.percent)}%`);
        });

        const unlistenAnalyzer = await onAnalyzerProgress((payload) => {
          update(s => ({
            ...s,
            currentStage: `Qwen3-VL Анализ: ${payload.stage}`,
            percentage: 45 + Math.round(payload.percent * 0.3), // займет 45% - 75% общего прогресса
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
          addLog(`Qwen3-VL: Шаг ${payload.stage} (${payload.current_frame}/${payload.total_frames} кадров)`);
        });

        const unlistenPipeline = await onPipelineProgress((payload) => {
          update(s => ({
            ...s,
            currentStage: payload.stage,
            percentage: 75 + Math.round(payload.percent * 0.1) // займет 75% - 85% общего прогресса
          }));
          addLog(`Пайплайн [${payload.stage}]: Прогресс ${Math.round(payload.percent)}%`);
        });

        const unlistenSuggested = await onSuggestedFragments((payload) => {
          if (payload.fragments && payload.fragments.length > 0) {
            const mapped = payload.fragments.map(f => ({
              id: Math.random().toString(),
              start: f.start,
              end: f.end
            }));
            fragmentsStore.set(mapped);
            addLog(`Получено ${mapped.length} фрагментов, предложенных ИИ.`);
          }
        });

        const unlistenRender = await onRenderProgress((payload) => {
          update(s => ({
            ...s,
            currentStage: `Рендеринг BMF: ${payload.stage}`,
            percentage: 85 + Math.round(payload.percent * 0.15), // займет последние 85% - 100%
            stageDetails: {
              ...s.stageDetails,
              renderer: {
                percent: payload.percent,
                stage: payload.stage
              }
            }
          }));
          addLog(`BMF Рендеринг: ${payload.stage} - ${Math.round(payload.percent)}%`);
        });

        // Запускаем сам пайплайн
        const clipPaths = await startPipeline(currentState.video.path, currentState.config);

        // Отписываемся от слушателей
        unlistenFfmpeg();
        unlistenWhisper();
        unlistenAnalyzer();
        unlistenPipeline();
        unlistenSuggested();
        unlistenRender();

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
            clips: clipPaths
          }));
          addLog(`Работа завершена! Получено клипов: ${clipPaths.length}`);
        }

      } catch (err: any) {
        const errorMsg = err.message || String(err);
        update(s => ({
          ...s,
          status: 'failed',
          currentStage: 'Ошибка выполнения пайплайна',
          error: errorMsg
        }));
        addLog(`КРИТИЧЕСКАЯ ОШИБКА: ${errorMsg}`);
      }
    }
  };
};

export const pipelineStore = createPipelineStore();

export async function exportManualClips(videoPath: string, fragments: Fragment[], config: PipelineConfig): Promise<string[]> {
  return await pipelineStore.exportManualClips(videoPath, fragments, config);
}
