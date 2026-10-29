// src/lib/stores/models.ts
import { writable, get } from 'svelte/store';
import { 
  getModelsStatus, 
  downloadModel, 
  deleteModel as ipcDeleteModel,
  cancelModelDownload as ipcCancelModelDownload,
  onModelDownloadProgress, 
  type ModelInfo, 
  type ModelDownloadProgressPayload 
} from '../ipc';
import { pipelineStore } from './pipeline';

export interface ModelDownloadState {
  progress: number; // 0 to 100
  downloadedBytes: number;
  totalBytes: number;
  status: 'idle' | 'downloading' | 'completed' | 'failed' | 'cancelled';
}

export interface ModelsStoreState {
  models: ModelInfo[];
  downloads: Record<string, ModelDownloadState>;
  deleting: Record<string, boolean>;
  loading: boolean;
  error: string | null;
}

export const CATALOG_MODELS: ModelInfo[] = [
  // Whisper ASR
  {
    id: 'whisper-tiny',
    name: 'Whisper Tiny (Ультрабыстрая)',
    filename: 'ggml-tiny.bin',
    url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin',
    size_mb: 75,
    description: 'Сверхбыстрая модель для слабых ПК и мгновенной черновой транскрипции (75 МБ)',
    category: 'whisper',
    params: '39M',
    exists: false,
    local_path: ''
  },
  {
    id: 'whisper-base',
    name: 'Whisper Base (Рекомендуется)',
    filename: 'ggml-base.bin',
    url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin',
    size_mb: 142,
    description: 'Оптимальный баланс скорости и точности для русской и английской речи (142 МБ)',
    category: 'whisper',
    params: '74M',
    exists: false,
    local_path: ''
  },
  {
    id: 'whisper-small-q5',
    name: 'Whisper Small Q5 (Легкая Pro)',
    filename: 'ggml-small-q5_1.bin',
    url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin',
    size_mb: 181,
    description: 'Квантованная Small-модель: высокая точность распознавания при малом весе (181 МБ)',
    category: 'whisper',
    params: '244M Q5',
    exists: false,
    local_path: ''
  },
  {
    id: 'whisper-small',
    name: 'Whisper Small (Высокая точность)',
    filename: 'ggml-small.bin',
    url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin',
    size_mb: 466,
    description: 'Отличное качество для аниме, стримов, сложной дикции и фонового шума (466 МБ)',
    category: 'whisper',
    params: '244M',
    exists: false,
    local_path: ''
  },
  {
    id: 'whisper-medium-q5',
    name: 'Whisper Medium Q5 (Макс. точность RU)',
    filename: 'ggml-medium-q5_0.bin',
    url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium-q5_0.bin',
    size_mb: 514,
    description: 'Квантованная Medium-модель: студийная точность русского языка без перегрузки ОЗУ (514 МБ)',
    category: 'whisper',
    params: '769M Q5',
    exists: false,
    local_path: ''
  },
  {
    id: 'whisper-large-v3-turbo-q5',
    name: 'Whisper Large-v3 Turbo Q5',
    filename: 'ggml-large-v3-turbo-q5_0.bin',
    url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin',
    size_mb: 547,
    description: 'Ускоренная флагманская модель OpenAI Large-v3 Turbo в компактном формате Q5 (547 МБ)',
    category: 'whisper',
    params: '809M Q5',
    exists: false,
    local_path: ''
  },
  // LLM (<= 3B)
  {
    id: 'qwen-0.5b-chat',
    name: 'Qwen 2.5 0.5B Instruct (Ультралегкая)',
    filename: 'qwen2.5-0.5b-instruct-q4_k_m.gguf',
    url: 'https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf',
    size_mb: 469,
    description: 'Мгновенный анализ реплик на любых ПК и ноутбуках даже без дискретной видеокарты (469 МБ)',
    category: 'llm',
    params: '0.5B',
    exists: false,
    local_path: ''
  },
  {
    id: 'llama-3.2-1b',
    name: 'Llama 3.2 1B Instruct (Быстрая)',
    filename: 'Llama-3.2-1B-Instruct-Q4_K_M.gguf',
    url: 'https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf',
    size_mb: 770,
    description: 'Легкая открытая модель Meta Llama 3.2 для быстрой оценки динамики диалогов (770 МБ)',
    category: 'llm',
    params: '1B',
    exists: false,
    local_path: ''
  },
  {
    id: 'qwen-1.5b-chat',
    name: 'Qwen 2.5 1.5B Instruct (Рекомендуется)',
    filename: 'qwen-1.5b-chat.gguf',
    url: 'https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf',
    size_mb: 1065,
    description: 'Оптимальный выбор: отличное понимание русского контекста, юмора и эмоций (1.06 ГБ)',
    category: 'llm',
    params: '1.5B',
    exists: false,
    local_path: ''
  },
  {
    id: 'gemma-2-2b',
    name: 'Google Gemma 2 2B Instruct',
    filename: 'gemma-2-2b-it-Q4_K_M.gguf',
    url: 'https://huggingface.co/bartowski/gemma-2-2b-it-GGUF/resolve/main/gemma-2-2b-it-Q4_K_M.gguf',
    size_mb: 1629,
    description: 'Модель от Google (2B): точное выделение смысловых панчлайнов и ярких реплик (1.63 ГБ)',
    category: 'llm',
    params: '2B',
    exists: false,
    local_path: ''
  },
  {
    id: 'llama-3.2-3b',
    name: 'Llama 3.2 3B Instruct (Продвинутая)',
    filename: 'Llama-3.2-3B-Instruct-Q4_K_M.gguf',
    url: 'https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf',
    size_mb: 1926,
    description: 'Продвинутая 3B модель с глубоким анализом драматургии и сюжетных поворотов (1.93 ГБ)',
    category: 'llm',
    params: '3B',
    exists: false,
    local_path: ''
  },
  {
    id: 'qwen-3b-chat',
    name: 'Qwen 2.5 3B Instruct (Максимум 3B)',
    filename: 'qwen2.5-3b-instruct-q4_k_m.gguf',
    url: 'https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf',
    size_mb: 2007,
    description: 'Максимальное качество анализа русскоязычных диалогов и сленга в пределах 3B (2.0 ГБ)',
    category: 'llm',
    params: '3B',
    exists: false,
    local_path: ''
  },
  // Vision AI (<= 2B)
  {
    id: 'qwen-vl-2b',
    name: 'Qwen2-VL 2B Vision (Анализ кадров)',
    filename: 'qwen-vl-2b.gguf',
    url: 'https://huggingface.co/bartowski/Qwen2-VL-2B-Instruct-GGUF/resolve/main/Qwen2-VL-2B-Instruct-Q4_K_M.gguf',
    size_mb: 940,
    description: 'Компактный локальный визуальный анализатор динамики сцен и ключевых кадров (940 МБ)',
    category: 'vision',
    params: '2B Vision',
    exists: false,
    local_path: ''
  }
];

const createModelsStore = () => {
  const store = writable<ModelsStoreState>({
    models: [...CATALOG_MODELS],
    downloads: {},
    deleting: {},
    loading: false,
    error: null,
  });
  const { subscribe, update } = store;

  let unlistenProgress: (() => void) | null = null;

  const selectModel = (model: ModelInfo) => {
    if (model.category === 'whisper') {
      pipelineStore.updateConfig({ asr_model_path: `models/${model.filename}` });
    } else if (model.category === 'llm') {
      pipelineStore.updateConfig({ text_engine: 'llama', text_model_name: model.filename });
    } else if (model.category === 'vision') {
      pipelineStore.updateConfig({ vision_model_path: `models/${model.filename}` });
    }
  };

  // Инициализация получения статусов моделей и настройка слушателя событий Tauri
  const init = async () => {
    update(s => ({ ...s, loading: true, error: null }));
    
    const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;

    if (!isTauri) {
      update(s => ({
        ...s,
        models: s.models.length > 0 ? s.models : [...CATALOG_MODELS],
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
          let completedModel: ModelInfo | undefined;

          update(s => {
            const nextDownloads = { ...s.downloads };
            nextDownloads[payload.model_id] = {
              progress: Math.round(payload.percent),
              downloadedBytes: payload.downloaded_bytes,
              totalBytes: payload.total_bytes,
              status: payload.status
            };

            const nextModels = s.models.map(m => {
              if (m.id === payload.model_id && payload.status === 'completed') {
                const updated = { ...m, exists: true, local_path: m.local_path || `models/${m.filename}` };
                completedModel = updated;
                return updated;
              }
              return m;
            });

            return {
              ...s,
              downloads: nextDownloads,
              models: nextModels
            };
          });

          if (completedModel) {
            selectModel(completedModel);
            // Обновляем точный локальный путь с бэкенда
            getModelsStatus().then((fresh) => {
              update(s => ({ ...s, models: fresh }));
            }).catch(() => {});
          }
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
      // В режиме браузерного превью симулируем быструю загрузку для проверки UI
      update(s => ({
        ...s,
        error: null,
        downloads: {
          ...s.downloads,
          [modelId]: { progress: 25, downloadedBytes: 0, totalBytes: 0, status: 'downloading' }
        }
      }));
      setTimeout(() => {
        update(s => {
          const nextModels = s.models.map(m => m.id === modelId ? { ...m, exists: true, local_path: `models/${m.filename}` } : m);
          const found = nextModels.find(m => m.id === modelId);
          if (found) selectModel(found);
          return {
            ...s,
            models: nextModels,
            downloads: {
              ...s.downloads,
              [modelId]: { progress: 100, downloadedBytes: 0, totalBytes: 0, status: 'completed' }
            }
          };
        });
      }, 600);
      return;
    }

    try {
      update(s => ({
        ...s,
        error: null,
        downloads: {
          ...s.downloads,
          [modelId]: {
            progress: 0,
            downloadedBytes: 0,
            totalBytes: 0,
            status: 'downloading'
          }
        }
      }));

      await downloadModel(modelId);
    } catch (err: any) {
      update(s => ({
        ...s,
        downloads: {
          ...s.downloads,
          [modelId]: {
            progress: 0,
            downloadedBytes: 0,
            totalBytes: 0,
            status: 'failed'
          }
        },
        error: err.message || String(err)
      }));
    }
  };

  // Отмена активной загрузки
  const cancelDownload = async (modelId: string) => {
    const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;
    if (isTauri) {
      try {
        await ipcCancelModelDownload(modelId);
      } catch (err: any) {
        console.warn('Ошибка отмены загрузки:', err);
      }
    }
    update(s => {
      const nextDownloads = { ...s.downloads };
      delete nextDownloads[modelId];
      return { ...s, downloads: nextDownloads };
    });
  };

  // Удаление скачанной модели с диска и переключение активной модели при необходимости
  const removeModel = async (modelId: string) => {
    const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;
    const currentState = get(store);
    const targetModel = currentState.models.find(m => m.id === modelId);

    update(s => ({
      ...s,
      error: null,
      deleting: { ...s.deleting, [modelId]: true }
    }));

    try {
      if (isTauri) {
        await ipcDeleteModel(modelId);
      }

      update(s => {
        const nextDeleting = { ...s.deleting };
        delete nextDeleting[modelId];
        const nextDownloads = { ...s.downloads };
        delete nextDownloads[modelId];

        const nextModels = s.models.map(m =>
          m.id === modelId ? { ...m, exists: false, local_path: '' } : m
        );

        // Если удаленная модель была выбрана как активная — переключаем на другую скачанную модель той же категории
        if (targetModel) {
          const pipeState = get(pipelineStore);
          const sameCategoryInstalled = nextModels.find(
            m => m.category === targetModel.category && m.exists && m.id !== modelId
          );

          if (targetModel.category === 'whisper' && pipeState.config.asr_model_path?.includes(targetModel.filename)) {
            pipelineStore.updateConfig({
              asr_model_path: sameCategoryInstalled ? `models/${sameCategoryInstalled.filename}` : 'models/ggml-base.bin'
            });
          } else if (targetModel.category === 'llm' && pipeState.config.text_model_name?.includes(targetModel.filename)) {
            if (sameCategoryInstalled) {
              pipelineStore.updateConfig({
                text_engine: 'llama',
                text_model_name: sameCategoryInstalled.filename
              });
            } else {
              pipelineStore.updateConfig({
                text_model_name: 'qwen-1.5b-chat.gguf'
              });
            }
          }
        }

        return {
          ...s,
          deleting: nextDeleting,
          downloads: nextDownloads,
          models: nextModels
        };
      });

      if (isTauri) {
        const fresh = await getModelsStatus();
        update(s => ({ ...s, models: fresh }));
      }
    } catch (err: any) {
      update(s => {
        const nextDeleting = { ...s.deleting };
        delete nextDeleting[modelId];
        return {
          ...s,
          deleting: nextDeleting,
          error: err.message || String(err)
        };
      });
    }
  };

  return {
    subscribe,
    init,
    startDownload,
    cancelDownload,
    removeModel,
    selectModel,
    cleanup: () => {
      if (unlistenProgress) {
        unlistenProgress();
        unlistenProgress = null;
      }
    }
  };
};

export const modelsStore = createModelsStore();
