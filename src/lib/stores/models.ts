// src/lib/stores/models.ts
import { writable } from 'svelte/store';
import { 
  getModelsStatus, 
  downloadModel, 
  onModelDownloadProgress, 
  type ModelInfo, 
  type ModelDownloadProgressPayload 
} from '../ipc';

export interface ModelDownloadState {
  progress: number; // 0 to 100
  downloadedBytes: number;
  totalBytes: number;
  status: 'idle' | 'downloading' | 'completed' | 'failed';
}

export interface ModelsStoreState {
  models: ModelInfo[];
  downloads: Record<string, ModelDownloadState>;
  loading: boolean;
  error: string | null;
}

const createModelsStore = () => {
  const { subscribe, set, update } = writable<ModelsStoreState>({
    models: [],
    downloads: {},
    loading: false,
    error: null,
  });

  let unlistenProgress: (() => void) | null = null;

  // Инициализация получения статусов моделей и настройка слушателя событий Tauri
  const init = async () => {
    update(s => ({ ...s, loading: true, error: null }));
    
    // Проверяем, запущены ли мы внутри Tauri
    const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;

    if (!isTauri) {
      // Имитируем модели в браузере для демо-превью
      console.log('Браузерная среда: Подключаем демонстрационные модели');
      const demoModels: ModelInfo[] = [
        {
          id: 'whisper-tiny',
          name: 'Whisper Tiny',
          filename: 'ggml-tiny.bin',
          url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin',
          size_mb: 75,
          description: 'Сверхбыстрая компактная модель для распознавания речи (75 MB)',
          exists: false,
          local_path: ''
        },
        {
          id: 'whisper-base',
          name: 'Whisper Base',
          filename: 'ggml-base.bin',
          url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin',
          size_mb: 148,
          description: 'Стандартная и быстрая модель для большинства диалогов (148 MB)',
          exists: true, // Сделаем одну модель "из коробки" существующей для удобства
          local_path: '/stub/models/ggml-base.bin'
        },
        {
          id: 'whisper-small',
          name: 'Whisper Small',
          filename: 'ggml-small.bin',
          url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin',
          size_mb: 466,
          description: 'Высокоточная модель для сложных условий записи (466 MB)',
          exists: false,
          local_path: ''
        },
        {
          id: 'qwen-vl-2b',
          name: 'Qwen3-VL 2B',
          filename: 'qwen-vl-2b.gguf',
          url: 'https://huggingface.co/Qwen/Qwen2-VL-2B-Instruct-GGUF/resolve/main/qwen2-vl-2b-instruct-q4_k_m.gguf',
          size_mb: 1700,
          description: 'Интеллектуальный ИИ-анализатор кадров и поведения лиц (1.7 GB)',
          exists: false,
          local_path: ''
        }
      ];

      update(s => ({
        ...s,
        models: demoModels,
        loading: false
      }));
      return;
    }

    try {
      const models = await getModelsStatus();
      update(s => ({ ...s, models, loading: false }));

      // Подписываемся на прогресс скачивания
      if (!unlistenProgress) {
        unlistenProgress = await onModelDownloadProgress((payload: ModelDownloadProgressPayload) => {
          update(s => {
            const nextDownloads = { ...s.downloads };
            nextDownloads[payload.model_id] = {
              progress: Math.round(payload.percent),
              downloadedBytes: payload.downloaded_bytes,
              totalBytes: payload.total_bytes,
              status: payload.status === 'completed' ? 'completed' : payload.status === 'failed' ? 'failed' : 'downloading'
            };

            // Если загрузка успешно завершена, обновим флаг exists у этой модели
            const nextModels = s.models.map(m => {
              if (m.id === payload.model_id && payload.status === 'completed') {
                return { ...m, exists: true, local_path: `resource://models/${m.filename}` };
              }
              return m;
            });

            return {
              ...s,
              downloads: nextDownloads,
              models: nextModels
            };
          });
        });
      }
    } catch (err: any) {
      update(s => ({ ...s, loading: false, error: err.message || String(err) }));
    }
  };

  // Метод для старта скачивания
  const startDownload = async (modelId: string) => {
    const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;

    if (!isTauri) {
      // Имитируем скачивание в браузере с постепенным прогрессом
      update(s => {
        const nextDownloads = { ...s.downloads };
        nextDownloads[modelId] = {
          progress: 0,
          downloadedBytes: 0,
          totalBytes: 100 * 1024 * 1024,
          status: 'downloading'
        };
        return { ...s, downloads: nextDownloads };
      });

      let currentPct = 0;
      const interval = setInterval(() => {
        currentPct += 10;
        update(s => {
          const nextDownloads = { ...s.downloads };
          if (currentPct >= 100) {
            clearInterval(interval);
            nextDownloads[modelId] = {
              progress: 100,
              downloadedBytes: 100 * 1024 * 1024,
              totalBytes: 100 * 1024 * 1024,
              status: 'completed'
            };
            const nextModels = s.models.map(m => m.id === modelId ? { ...m, exists: true, local_path: `/stub/models/${m.filename}` } : m);
            return { ...s, downloads: nextDownloads, models: nextModels };
          } else {
            nextDownloads[modelId] = {
              progress: currentPct,
              downloadedBytes: Math.round((currentPct / 100) * 100 * 1024 * 1024),
              totalBytes: 100 * 1024 * 1024,
              status: 'downloading'
            };
            return { ...s, downloads: nextDownloads };
          }
        });
      }, 300);

      return;
    }

    try {
      update(s => {
        const nextDownloads = { ...s.downloads };
        nextDownloads[modelId] = {
          progress: 0,
          downloadedBytes: 0,
          totalBytes: 0,
          status: 'downloading'
        };
        return { ...s, downloads: nextDownloads };
      });

      await downloadModel(modelId);
    } catch (err: any) {
      update(s => {
        const nextDownloads = { ...s.downloads };
        nextDownloads[modelId] = {
          progress: 0,
          downloadedBytes: 0,
          totalBytes: 0,
          status: 'failed'
        };
        return { ...s, downloads: nextDownloads, error: err.message || String(err) };
      });
    }
  };

  return {
    subscribe,
    init,
    startDownload,
    cleanup: () => {
      if (unlistenProgress) {
        unlistenProgress();
        unlistenProgress = null;
      }
    }
  };
};

export const modelsStore = createModelsStore();
