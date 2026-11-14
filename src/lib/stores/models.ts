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
  // LLM (<= 3.8B, 100% нативно совместимые с llama-cli b3201: архитектуры qwen2 и phi3)
  {
    id: 'vikhr-qwen-2.5-0.5b',
    name: 'Vikhr Qwen 2.5 0.5B Instruct (RU Ультрабыстрая Q5)',
    filename: 'Vikhr-Qwen-2.5-0.5B-instruct-Q5_K_M.gguf',
    url: 'https://huggingface.co/Vikhrmodels/Vikhr-Qwen-2.5-0.5B-Instruct-GGUF/resolve/main/Vikhr-Qwen-2.5-0.5B-instruct-Q5_K_M.gguf',
    size_mb: 400,
    description: 'Сверхбыстрая русскоязычная модель Vikhr 0.5B Q5_K_M (архитектура qwen2): мгновенный отбор реплик и сленга на любых ПК и ноутбуках (400 МБ)',
    category: 'llm',
    params: '0.5B RU Q5',
    exists: false,
    local_path: ''
  },
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
    id: 'vikhr-qwen-2.5-1.5b',
    name: 'Vikhr Qwen 2.5 1.5B (RU Диалоги и Юмор)',
    filename: 'Vikhr-Qwen-2.5-1.5B-Instruct.Q4_K_M.gguf',
    url: 'https://huggingface.co/QuantFactory/Vikhr-Qwen-2.5-1.5B-Instruct-GGUF/resolve/main/Vikhr-Qwen-2.5-1.5B-Instruct.Q4_K_M.gguf',
    size_mb: 1065,
    description: 'Русскоязычная дообученная Qwen 2.5 1.5B от проекта Vikhr: отличное чувство юмора, сарказма и сюжетного накала (1.06 ГБ)',
    category: 'llm',
    params: '1.5B RU Q4',
    exists: false,
    local_path: ''
  },
  {
    id: 'qwen-1.5b-chat',
    name: 'Qwen 2.5 1.5B Instruct (Рекомендуется)',
    filename: 'qwen-1.5b-chat.gguf',
    url: 'https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf',
    size_mb: 1066,
    description: 'Оптимальный выбор: отличное понимание русского контекста, юмора и эмоций (1.06 ГБ)',
    category: 'llm',
    params: '1.5B Q4',
    exists: false,
    local_path: ''
  },
  {
    id: 'vikhr-qwen-2.5-1.5b-q5',
    name: 'Vikhr Qwen 2.5 1.5B Q5_K_M (RU Повышенная точность)',
    filename: 'Vikhr-Qwen-2.5-1.5B-Instruct.Q5_K_M.gguf',
    url: 'https://huggingface.co/QuantFactory/Vikhr-Qwen-2.5-1.5B-Instruct-GGUF/resolve/main/Vikhr-Qwen-2.5-1.5B-Instruct.Q5_K_M.gguf',
    size_mb: 1225,
    description: 'Квантование Q5_K_M русскоязычной модели Vikhr 1.5B: минимум потерь точности при оценке панчлайнов и SMM-описаний (1.22 ГБ)',
    category: 'llm',
    params: '1.5B RU Q5',
    exists: false,
    local_path: ''
  },
  {
    id: 'vikhr-qwen-2.5-1.5b-q8',
    name: 'Vikhr Qwen 2.5 1.5B Q8_0 (RU Студийный эталон)',
    filename: 'Vikhr-Qwen-2.5-1.5B-Instruct.Q8_0.gguf',
    url: 'https://huggingface.co/QuantFactory/Vikhr-Qwen-2.5-1.5B-Instruct-GGUF/resolve/main/Vikhr-Qwen-2.5-1.5B-Instruct.Q8_0.gguf',
    size_mb: 1806,
    description: 'Эталонное 8-битное качество (Q8_0) без деградации весов: идеально для финальной проверки связности и остроумного SMM-копирайтинга (1.8 ГБ)',
    category: 'llm',
    params: '1.5B RU Q8',
    exists: false,
    local_path: ''
  },
  {
    id: 'qwen-3b-chat',
    name: 'Qwen 2.5 3B Instruct Q4_K_M (Мощная 3B)',
    filename: 'qwen2.5-3b-instruct-q4_k_m.gguf',
    url: 'https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf',
    size_mb: 2007,
    description: 'Глубокий анализ русскоязычных диалогов, иронии и драматургии в классе 3B (2.0 ГБ)',
    category: 'llm',
    params: '3B Q4',
    exists: false,
    local_path: ''
  },
  {
    id: 'phi-3-mini-4k',
    name: 'Microsoft Phi-3 Mini 4K Instruct Q4 (Логика и Сюжет)',
    filename: 'Phi-3-mini-4k-instruct-q4.gguf',
    url: 'https://huggingface.co/microsoft/Phi-3-mini-4k-instruct-gguf/resolve/main/Phi-3-mini-4k-instruct-q4.gguf',
    size_mb: 2282,
    description: 'Официальная модель Microsoft Phi-3 Mini (3.8B, архитектура phi3): сильная логика для проверки законченности мысли и структуры диалога (2.28 ГБ)',
    category: 'llm',
    params: '3.8B Q4',
    exists: false,
    local_path: ''
  },
  {
    id: 'qwen-3b-q5',
    name: 'Qwen 2.5 3B Instruct Q5_K_M (Флагман Pro)',
    filename: 'qwen2.5-3b-instruct-q5_k_m.gguf',
    url: 'https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q5_k_m.gguf',
    size_mb: 2326,
    description: 'Максимальное качество анализа диалогов, юмора и генерации креативных SMM-заголовков на базе Qwen 2.5 3B Q5_K_M (2.33 ГБ)',
    category: 'llm',
    params: '3B Q5',
    exists: false,
    local_path: ''
  },
  // Vision AI (Совместимые с llama-vision / llava-cli модели + проектор mmproj)
  {
    id: 'moondream2',
    name: 'Moondream 2 Vision Q4_K (Быстрая + mmproj)',
    filename: 'moondream2-q4_k.gguf',
    url: 'https://huggingface.co/salivosa/moondream2-gguf/resolve/main/moondream2-q4_k.gguf',
    size_mb: 877,
    description: 'Быстрая и легкая Vision-нейросеть (архитектура Phi-2 + CLIP), нативно совместимая с llama-vision: сканирует кадры видео без перегрузки ПК (1.74 ГБ комплект с mmproj).',
    category: 'vision',
    params: '1.8B + mmproj',
    exists: false,
    local_path: '',
    mmproj_filename: 'moondream2-mmproj-f16.gguf',
    mmproj_url: 'https://huggingface.co/salivosa/moondream2-gguf/resolve/main/moondream2-mmproj-f16.gguf',
    mmproj_size_mb: 868
  },
  {
    id: 'llava-phi-3-mini',
    name: 'LLaVA Phi-3 Mini Vision INT4 (Флагман Pro + mmproj)',
    filename: 'llava-phi-3-mini-int4.gguf',
    url: 'https://huggingface.co/xtuner/llava-phi-3-mini-gguf/resolve/main/llava-phi-3-mini-int4.gguf',
    size_mb: 2211,
    description: 'Флагманская мультимодальная модель LLaVA (XTuner Phi-3 Mini + CLIP): высокая точность распознавания сцен, эмоций персонажей и экшена (2.79 ГБ комплект с mmproj).',
    category: 'vision',
    params: '3.8B INT4 + mmproj',
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
