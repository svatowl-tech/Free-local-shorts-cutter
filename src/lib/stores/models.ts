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
import { isTauriRuntime, joinRelativePath, formatErrorMessage } from '../utils/path';

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
  // Модели распознавания речи
  {
    id: 'whisper-tiny',
    name: 'Whisper Tiny (Ультрабыстрая)',
    filename: 'ggml-tiny.bin',
    url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin',
    size_mb: 75,
    description: 'Максимальная скорость работы для слабых ПК и быстрой разметки речи (75 МБ)',
    category: 'whisper',
    params: 'Быстрая',
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
    params: 'Баланс',
    exists: false,
    local_path: ''
  },
  {
    id: 'whisper-small-q5',
    name: 'Whisper Small Q5 (Компактная)',
    filename: 'ggml-small-q5_1.bin',
    url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin',
    size_mb: 181,
    description: 'Компактная версия с высокой точностью распознавания и низким расходом памяти (181 МБ)',
    category: 'whisper',
    params: 'Точная',
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
    params: 'Высокая',
    exists: false,
    local_path: ''
  },
  {
    id: 'whisper-medium-q5',
    name: 'Whisper Medium Q5 (Макс. точность RU)',
    filename: 'ggml-medium-q5_0.bin',
    url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium-q5_0.bin',
    size_mb: 514,
    description: 'Студийная точность распознавания русской речи без перегрузки оперативной памяти (514 МБ)',
    category: 'whisper',
    params: 'Студийная RU',
    exists: false,
    local_path: ''
  },
  {
    id: 'whisper-large-v3-turbo-q5',
    name: 'Whisper Large-v3 Turbo',
    filename: 'ggml-large-v3-turbo-q5_0.bin',
    url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin',
    size_mb: 547,
    description: 'Ускоренная флагманская модель распознавания речи в компактном размере (547 МБ)',
    category: 'whisper',
    params: 'Флагман',
    exists: false,
    local_path: ''
  },
  // Модели анализа диалогов и текстов
  {
    id: 'vikhr-qwen-2.5-0.5b',
    name: 'Vikhr 0.5B (RU Быстрая)',
    filename: 'Vikhr-Qwen-2.5-0.5B-instruct-Q5_K_M.gguf',
    url: 'https://huggingface.co/Vikhrmodels/Vikhr-Qwen-2.5-0.5B-Instruct-GGUF/resolve/main/Vikhr-Qwen-2.5-0.5B-instruct-Q5_K_M.gguf',
    size_mb: 400,
    description: 'Сверхбыстрая русскоязычная модель для моментального отбора реплик даже на ноутбуках без видеокарты (400 МБ)',
    category: 'llm',
    params: '0.5B RU',
    exists: false,
    local_path: ''
  },
  {
    id: 'qwen-0.5b-chat',
    name: 'Qwen 2.5 0.5B (Ультралегкая)',
    filename: 'qwen2.5-0.5b-instruct-q4_k_m.gguf',
    url: 'https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf',
    size_mb: 469,
    description: 'Лёгкий и быстрый анализ диалогов на любых компьютерах и ноутбуках (469 МБ)',
    category: 'llm',
    params: '0.5B',
    exists: false,
    local_path: ''
  },
  {
    id: 'vikhr-qwen-2.5-1.5b',
    name: 'Vikhr 1.5B (RU Диалоги и Юмор)',
    filename: 'Vikhr-Qwen-2.5-1.5B-Instruct.Q4_K_M.gguf',
    url: 'https://huggingface.co/QuantFactory/Vikhr-Qwen-2.5-1.5B-Instruct-GGUF/resolve/main/Vikhr-Qwen-2.5-1.5B-Instruct.Q4_K_M.gguf',
    size_mb: 1065,
    description: 'Сбалансированная русскоязычная модель: хорошо понимает шутки, сарказм и развитие сцен (1.06 ГБ)',
    category: 'llm',
    params: '1.5B RU',
    exists: false,
    local_path: ''
  },
  {
    id: 'qwen-1.5b-chat',
    name: 'Qwen 2.5 1.5B (Рекомендуется)',
    filename: 'qwen-1.5b-chat.gguf',
    url: 'https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf',
    size_mb: 1066,
    description: 'Оптимальный выбор по скорости и качеству: точное понимание контекста, юмора и эмоций (1.06 ГБ)',
    category: 'llm',
    params: '1.5B',
    exists: false,
    local_path: ''
  },
  {
    id: 'vikhr-qwen-2.5-1.5b-q5',
    name: 'Vikhr 1.5B Pro (RU Повышенная точность)',
    filename: 'Vikhr-Qwen-2.5-1.5B-Instruct.Q5_K_M.gguf',
    url: 'https://huggingface.co/QuantFactory/Vikhr-Qwen-2.5-1.5B-Instruct-GGUF/resolve/main/Vikhr-Qwen-2.5-1.5B-Instruct.Q5_K_M.gguf',
    size_mb: 1225,
    description: 'Улучшенная точность подбора ярких концовок и живых описаний для публикаций (1.22 ГБ)',
    category: 'llm',
    params: '1.5B Pro',
    exists: false,
    local_path: ''
  },
  {
    id: 'vikhr-qwen-2.5-1.5b-q8',
    name: 'Vikhr 1.5B Studio (RU Максимум)',
    filename: 'Vikhr-Qwen-2.5-1.5B-Instruct.Q8_0.gguf',
    url: 'https://huggingface.co/QuantFactory/Vikhr-Qwen-2.5-1.5B-Instruct-GGUF/resolve/main/Vikhr-Qwen-2.5-1.5B-Instruct.Q8_0.gguf',
    size_mb: 1806,
    description: 'Максимальная точность для проверки связности диалогов и создания ярких заголовков (1.8 ГБ)',
    category: 'llm',
    params: '1.5B Studio',
    exists: false,
    local_path: ''
  },
  {
    id: 'qwen-3b-chat',
    name: 'Qwen 2.5 3B (Мощная)',
    filename: 'qwen2.5-3b-instruct-q4_k_m.gguf',
    url: 'https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf',
    size_mb: 2007,
    description: 'Глубокий анализ длинных диалогов, иронии и драматургии с повышенными требованиями к памяти (2.0 ГБ)',
    category: 'llm',
    params: '3B',
    exists: false,
    local_path: ''
  },
  {
    id: 'phi-3-mini-4k',
    name: 'Phi-3 Mini (Логика и Сюжет)',
    filename: 'Phi-3-mini-4k-instruct-q4.gguf',
    url: 'https://huggingface.co/microsoft/Phi-3-mini-4k-instruct-gguf/resolve/main/Phi-3-mini-4k-instruct-q4.gguf',
    size_mb: 2282,
    description: 'Сильная логическая проверка законченности мысли и структуры диалога (2.28 ГБ)',
    category: 'llm',
    params: '3.8B',
    exists: false,
    local_path: ''
  },
  {
    id: 'qwen-3b-q5',
    name: 'Qwen 2.5 3B Pro (Флагман)',
    filename: 'qwen2.5-3b-instruct-q5_k_m.gguf',
    url: 'https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q5_k_m.gguf',
    size_mb: 2326,
    description: 'Максимальное качество отбора сцен, понимания юмора и подготовки текстов для публикаций (2.33 ГБ)',
    category: 'llm',
    params: '3B Pro',
    exists: false,
    local_path: ''
  },
  // Модели визуального анализа кадров
  {
    id: 'moondream2',
    name: 'Moondream 2 Vision (Быстрая)',
    filename: 'moondream2-q4_k.gguf',
    url: 'https://huggingface.co/salivosa/moondream2-gguf/resolve/main/moondream2-q4_k.gguf',
    size_mb: 877,
    description: 'Быстрый визуальный анализ кадров без высокой нагрузки на компьютер (1.74 ГБ)',
    category: 'vision',
    params: '1.8B',
    exists: false,
    local_path: '',
    mmproj_filename: 'moondream2-mmproj-f16.gguf',
    mmproj_url: 'https://huggingface.co/salivosa/moondream2-gguf/resolve/main/moondream2-mmproj-f16.gguf',
    mmproj_size_mb: 868
  },
  {
    id: 'llava-phi-3-mini',
    name: 'LLaVA Phi-3 Mini Vision (Флагман)',
    filename: 'llava-phi-3-mini-int4.gguf',
    url: 'https://huggingface.co/xtuner/llava-phi-3-mini-gguf/resolve/main/llava-phi-3-mini-int4.gguf',
    size_mb: 2211,
    description: 'Высокая точность распознавания сцен, эмоций персонажей и динамичных событий в кадре (2.79 ГБ)',
    category: 'vision',
    params: '3.8B',
    exists: false,
    local_path: '',
    mmproj_filename: 'llava-phi-3-mini-mmproj-f16.gguf',
    mmproj_url: 'https://huggingface.co/xtuner/llava-phi-3-mini-gguf/resolve/main/llava-phi-3-mini-mmproj-f16.gguf',
    mmproj_size_mb: 579
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
      pipelineStore.updateConfig({ asr_model_path: joinRelativePath('models', model.filename) });
    } else if (model.category === 'llm') {
      pipelineStore.updateConfig({ text_engine: 'llama', text_model_name: model.filename });
    } else if (model.category === 'vision') {
      pipelineStore.updateConfig({ vision_model_path: joinRelativePath('models', model.filename) });
    }
  };

  // Инициализация получения статусов моделей и настройка слушателя событий Tauri
  const init = async () => {
    update(s => ({ ...s, loading: true, error: null }));
    
    const isTauri = isTauriRuntime();

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

      // Автоматическая синхронизация выбранных моделей со скачанными весами (предпочитаем более точные модели Small/Medium/3B)
      const pipeState = get(pipelineStore);
      const installedWhisper = models.filter(m => m.category === 'whisper' && m.exists);
      const installedLlm = models.filter(m => m.category === 'llm' && m.exists);
      const installedVision = models.filter(m => m.category === 'vision' && m.exists);

      if (installedWhisper.length > 0) {
        const currentAsr = pipeState.config.asr_model_path || '';
        const currentAsrInstalled = installedWhisper.some(m => currentAsr.includes(m.filename));
        const bestWhisper = installedWhisper[installedWhisper.length - 1];
        if (!currentAsrInstalled || (currentAsr.includes('ggml-base.bin') && bestWhisper.filename !== 'ggml-base.bin' && bestWhisper.filename !== 'ggml-tiny.bin')) {
          pipelineStore.updateConfig({ asr_model_path: `models/${bestWhisper.filename}` });
        }
      }

      if (installedLlm.length > 0) {
        const bestLlm = installedLlm[installedLlm.length - 1];
        const currentLlm = pipeState.config.text_model_name || '';
        const currentVerifier = pipeState.config.verifier_model_name || '';
        const currentSmm = pipeState.config.smm_model_name || '';
        const nextLlmPatch: Record<string, string> = {};

        if (!installedLlm.some(m => currentLlm.includes(m.filename))) {
          nextLlmPatch.text_model_name = bestLlm.filename;
        }
        if (!installedLlm.some(m => currentVerifier.includes(m.filename))) {
          nextLlmPatch.verifier_model_name = bestLlm.filename;
        }
        if (!installedLlm.some(m => currentSmm.includes(m.filename))) {
          nextLlmPatch.smm_model_name = bestLlm.filename;
        }
        if (Object.keys(nextLlmPatch).length > 0) {
          pipelineStore.updateConfig(nextLlmPatch);
        }
      }

      const currentVision = pipeState.config.vision_model_path || '';
      if (installedVision.length > 0) {
        const currentVisionInstalled = installedVision.some(m => currentVision.includes(m.filename));
        if (!currentVisionInstalled) {
          const bestVision = installedVision[installedVision.length - 1];
          pipelineStore.updateConfig({ vision_model_path: `models/${bestVision.filename}` });
        }
      } else if (currentVision.toLowerCase().includes('qwen')) {
        pipelineStore.updateConfig({ vision_model_path: 'models/moondream2-q4_k.gguf' });
      }

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
                const updated = { ...m, exists: true, local_path: m.local_path || joinRelativePath('models', m.filename) };
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
            }).catch((err: unknown) => {
              console.warn('Не удалось обновить статус моделей после скачивания:', formatErrorMessage(err));
            });
          }
        });
      }
    } catch (err: unknown) {
      update(s => ({ ...s, loading: false, error: formatErrorMessage(err) }));
    }
  };

  // Метод для старта скачивания
  const startDownload = async (modelId: string) => {
    const isTauri = isTauriRuntime();

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
    } catch (err: unknown) {
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
        error: formatErrorMessage(err)
      }));
    }
  };

  // Отмена активной загрузки
  const cancelDownload = async (modelId: string) => {
    const isTauri = isTauriRuntime();
    if (isTauri) {
      try {
        await ipcCancelModelDownload(modelId);
      } catch (err: unknown) {
        console.warn('Ошибка отмены загрузки:', formatErrorMessage(err));
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
    const isTauri = isTauriRuntime();
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
          } else if (targetModel.category === 'llm') {
            const fallbackFilename = sameCategoryInstalled ? sameCategoryInstalled.filename : 'Vikhr-Qwen-2.5-1.5B-Instruct.Q4_K_M.gguf';
            const nextLlmPatch: Record<string, string> = {};
            if (pipeState.config.text_model_name?.includes(targetModel.filename)) {
              nextLlmPatch.text_model_name = fallbackFilename;
            }
            if (pipeState.config.verifier_model_name?.includes(targetModel.filename)) {
              nextLlmPatch.verifier_model_name = fallbackFilename;
            }
            if (pipeState.config.smm_model_name?.includes(targetModel.filename)) {
              nextLlmPatch.smm_model_name = fallbackFilename;
            }
            if (Object.keys(nextLlmPatch).length > 0) {
              if (sameCategoryInstalled) {
                nextLlmPatch.text_engine = 'llama';
              }
              pipelineStore.updateConfig(nextLlmPatch);
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
    } catch (err: unknown) {
      update(s => {
        const nextDeleting = { ...s.deleting };
        delete nextDeleting[modelId];
        return {
          ...s,
          deleting: nextDeleting,
          error: formatErrorMessage(err)
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
