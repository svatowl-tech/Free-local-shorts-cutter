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
      update(s => ({
        ...s,
        models: [],
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
      console.warn('Скачивание моделей доступно только в десктопном приложении Tauri');
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
