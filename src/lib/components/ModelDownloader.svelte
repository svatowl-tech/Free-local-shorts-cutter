<!-- src/lib/components/ModelDownloader.svelte -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { modelsStore } from '../stores/models';
  import { pipelineStore } from '../stores/pipeline';
  import type { ModelInfo } from '../ipc';
  import { 
    Download, 
    CheckCircle, 
    AlertCircle, 
    Loader2, 
    Folder, 
    RefreshCw, 
    Check,
    Trash2,
    X
  } from '@lucide/svelte';

  export let onClose: () => void = () => {};

  let activeTab: 'all' | 'whisper' | 'llm' | 'vision' = 'all';

  function formatBytes(bytes: number): string {
    if (!bytes || bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  }

  function formatSizeMb(sizeMb: number): string {
    if (sizeMb >= 1000) {
      return `${(sizeMb / 1000).toFixed(2)} ГБ`;
    }
    return `${Math.round(sizeMb)} МБ`;
  }

  function isModelSelected(model: ModelInfo, config: typeof $pipelineStore.config): boolean {
    if (model.category === 'whisper') {
      return Boolean(config.asr_model_path && config.asr_model_path.includes(model.filename));
    }
    if (model.category === 'llm') {
      return Boolean(config.text_model_name && config.text_model_name.includes(model.filename));
    }
    if (model.category === 'vision') {
      return Boolean(config.vision_model_path && config.vision_model_path.includes(model.filename));
    }
    return false;
  }

  function getCategoryBadge(category: string): { label: string; colorClass: string } {
    if (category === 'whisper') {
      return { label: 'WHISPER РЕЧЬ', colorClass: 'text-[#38bdf8] bg-[#38bdf8]/10 border-[#38bdf8]/30' };
    }
    if (category === 'llm') {
      return { label: 'LLM ТЕКСТ ≤3B', colorClass: 'text-[#a855f7] bg-[#a855f7]/10 border-[#a855f7]/30' };
    }
    return { label: 'VISION КАДРЫ', colorClass: 'text-[#f59e0b] bg-[#f59e0b]/10 border-[#f59e0b]/30' };
  }

  onMount(() => {
    modelsStore.init();
  });

  onDestroy(() => {
    modelsStore.cleanup();
  });

  function handleDownload(modelId: string) {
    modelsStore.startDownload(modelId);
  }

  function handleCancelDownload(modelId: string) {
    modelsStore.cancelDownload(modelId);
  }

  function handleDelete(modelId: string) {
    modelsStore.removeModel(modelId);
  }

  function handleSelect(model: ModelInfo) {
    modelsStore.selectModel(model);
  }

  function handleRefresh() {
    modelsStore.init();
  }

  $: filteredModels = $modelsStore.models.filter(m => activeTab === 'all' || m.category === activeTab);
  $: installedCount = $modelsStore.models.filter(m => m.exists).length;
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions a11y_interactive_supports_focus -->
<div 
  class="fixed inset-0 bg-[#0c0c0e]/80 backdrop-blur-sm flex items-center justify-center z-50 p-4" 
  on:click|self={onClose} 
  role="dialog" 
  aria-modal="true" 
  aria-label="Менеджер моделей"
  tabindex="0"
>
  <div class="panel w-full max-w-3xl overflow-hidden shadow-2xl flex flex-col max-h-[90vh] bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[4px]">
    
    <!-- Modal Header -->
    <div class="p-5 sm:p-6 border-b border-[rgba(226,226,224,0.1)] bg-[#0c0c0e]/40 flex items-center justify-between gap-4">
      <div class="flex items-center gap-3">
        <div class="w-10 h-10 bg-[#5865f2] rounded-[4px] flex items-center justify-center text-white shrink-0">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="4" y="4" width="16" height="16" rx="2"/>
            <rect x="8" y="8" width="8" height="8" rx="1"/>
            <path d="M12 2v2M12 20v2M2 12h2M20 12h2"/>
          </svg>
        </div>
        <div>
          <div class="label-mono">ОТКРЫТЫЕ ЛОКАЛЬНЫЕ НЕЙРОСЕТИ (МАКС. 3B • БЕЗ КЛЮЧЕЙ)</div>
          <h2 class="panel-title text-sm">Менеджер ИИ-Моделей ({installedCount} установлено)</h2>
        </div>
      </div>
      
      <div class="flex items-center gap-2">
        <button 
          type="button"
          on:click={handleRefresh}
          class="btn py-1.5 px-3 border border-[rgba(226,226,224,0.15)] bg-transparent text-[rgba(226,226,224,0.7)] hover:text-[#e2e2e4] text-[10px] flex items-center gap-1.5 cursor-pointer rounded-[2px]"
          title="Обновить статусы файлов на диске"
        >
          <RefreshCw class="h-3 w-3 {$modelsStore.loading ? 'animate-spin' : ''}" />
          <span>ОБНОВИТЬ</span>
        </button>
        <button 
          type="button"
          on:click={onClose}
          class="p-1.5 text-[rgba(226,226,224,0.5)] hover:text-[#e2e2e4] cursor-pointer"
          aria-label="Закрыть"
        >
          <X class="h-5 w-5" />
        </button>
      </div>
    </div>

    <!-- Category Filter Tabs -->
    <div class="px-6 pt-4 pb-3 border-b border-[rgba(226,226,224,0.08)] bg-[#0c0c0e]/20 flex flex-wrap gap-2">
      <button
        type="button"
        on:click={() => (activeTab = 'all')}
        class="font-mono text-[11px] px-3 py-1.5 rounded-[2px] border cursor-pointer transition-colors {activeTab === 'all'
          ? 'bg-[#5865f2] text-white border-[#5865f2] font-bold'
          : 'bg-[#0c0c0e] text-[rgba(226,226,224,0.65)] border-[rgba(226,226,224,0.12)] hover:text-[#e2e2e4]'}"
      >
        ВСЕ МОДЕЛИ ({$modelsStore.models.length})
      </button>
      <button
        type="button"
        on:click={() => (activeTab = 'whisper')}
        class="font-mono text-[11px] px-3 py-1.5 rounded-[2px] border cursor-pointer transition-colors {activeTab === 'whisper'
          ? 'bg-[#5865f2] text-white border-[#5865f2] font-bold'
          : 'bg-[#0c0c0e] text-[rgba(226,226,224,0.65)] border-[rgba(226,226,224,0.12)] hover:text-[#e2e2e4]'}"
      >
        СУБТИТРЫ WHISPER ({$modelsStore.models.filter(m => m.category === 'whisper').length})
      </button>
      <button
        type="button"
        on:click={() => (activeTab = 'llm')}
        class="font-mono text-[11px] px-3 py-1.5 rounded-[2px] border cursor-pointer transition-colors {activeTab === 'llm'
          ? 'bg-[#5865f2] text-white border-[#5865f2] font-bold'
          : 'bg-[#0c0c0e] text-[rgba(226,226,224,0.65)] border-[rgba(226,226,224,0.12)] hover:text-[#e2e2e4]'}"
      >
        АНАЛИЗ ДИАЛОГОВ LLM ≤3B ({$modelsStore.models.filter(m => m.category === 'llm').length})
      </button>
      <button
        type="button"
        on:click={() => (activeTab = 'vision')}
        class="font-mono text-[11px] px-3 py-1.5 rounded-[2px] border cursor-pointer transition-colors {activeTab === 'vision'
          ? 'bg-[#5865f2] text-white border-[#5865f2] font-bold'
          : 'bg-[#0c0c0e] text-[rgba(226,226,224,0.65)] border-[rgba(226,226,224,0.12)] hover:text-[#e2e2e4]'}"
      >
        КАДРЫ VISION ({$modelsStore.models.filter(m => m.category === 'vision').length})
      </button>
    </div>

    <!-- Content -->
    <div class="p-6 overflow-y-auto space-y-4 flex-1">
      
      {#if $modelsStore.error}
        <div class="p-4 bg-red-950/20 border-l-2 border-red-500 rounded-[2px] flex items-start gap-3">
          <AlertCircle class="h-4 w-4 text-red-400 shrink-0 mt-0.5" />
          <div class="text-xs font-mono">
            <div class="font-bold text-red-400 uppercase">ОШИБКА РАБОТЫ С МОДЕЛЯМИ</div>
            <p class="text-[rgba(226,226,224,0.7)] mt-1">{$modelsStore.error}</p>
          </div>
        </div>
      {/if}

      <div class="info-card">
        <div class="label-mono text-[#e2e2e4] mb-1">Оптимизировано для обычных ПК и ноутбуков (до 3B параметров)</div>
        Все представленные модели полностью бесплатны, скачиваются напрямую из официальных репозиториев Hugging Face без API-ключей и токенов. Вы можете свободно скачивать новые модели, переключаться между ними кнопкой <strong class="text-[#5865f2] font-mono">ВЫБРАТЬ</strong> и удалять ненужные веса кнопкой <strong class="text-red-400 font-mono">УДАЛИТЬ</strong> для освобождения места на диске.
      </div>

      <div class="space-y-3 pt-1">
        {#if $modelsStore.loading && $modelsStore.models.length === 0}
          <div class="flex flex-col items-center justify-center py-12 text-[rgba(226,226,224,0.4)] gap-2 font-mono">
            <Loader2 class="h-6 w-6 animate-spin text-[#5865f2]" />
            <span class="text-xs">Сканирование моделей...</span>
          </div>
        {:else}
          {#each filteredModels as model (model.id)}
            {@const downloadState = $modelsStore.downloads[model.id]}
            {@const isDeleting = $modelsStore.deleting[model.id]}
            {@const isSelected = isModelSelected(model, $pipelineStore.config)}
            {@const badge = getCategoryBadge(model.category)}
            <div class="bg-[#0c0c0e] border {isSelected && model.exists ? 'border-[#5865f2]/60' : 'border-[rgba(226,226,224,0.1)]'} p-4 rounded-[4px] flex flex-col sm:flex-row sm:items-center justify-between gap-4 transition-colors">
              
              <!-- Info -->
              <div class="flex-1 space-y-1.5">
                <div class="flex flex-wrap items-center gap-2">
                  <span class="label-mono px-1.5 py-0.5 rounded-[2px] border text-[9px] font-bold {badge.colorClass}">
                    {badge.label}
                  </span>
                  <h3 class="text-xs font-bold text-[#e2e2e4] font-syne">{model.name}</h3>
                  {#if model.params}
                    <span class="label-mono text-[#e2e2e4] bg-[rgba(226,226,224,0.08)] px-1.5 py-0.5 rounded-[2px]">
                      {model.params}
                    </span>
                  {/if}
                  <span class="label-mono text-[#5865f2] font-semibold bg-[#5865f2]/10 px-1.5 py-0.5 rounded-[2px]">
                    {formatSizeMb(model.size_mb)}
                  </span>
                  {#if model.exists}
                    <span class="label-mono text-[#23c55e] font-semibold bg-[#23c55e]/10 px-1.5 py-0.5 rounded-[2px] flex items-center gap-1">
                      <Check class="h-2.5 w-2.5" /> Скачана
                    </span>
                  {/if}
                  {#if isSelected}
                    <span class="label-mono text-white font-bold bg-[#5865f2] px-1.5 py-0.5 rounded-[2px]">
                      АКТИВНА
                    </span>
                  {/if}
                </div>
                <p class="text-xs text-[rgba(226,226,224,0.65)] leading-normal">{model.description}</p>
                <div class="flex flex-wrap items-center gap-3 text-[10px] text-[rgba(226,226,224,0.4)] font-mono mt-1">
                  <span class="flex items-center gap-1">
                    <Folder class="h-3 w-3" />
                    <span>Файл: {model.filename}</span>
                  </span>
                </div>
              </div>

              <!-- Actions or progress -->
              <div class="sm:shrink-0 flex items-center gap-2">
                {#if downloadState && downloadState.status === 'downloading'}
                  <div class="w-full sm:w-52 space-y-1.5">
                    <div class="flex justify-between items-center text-[10px] font-mono">
                      <span class="text-[#5865f2] font-bold flex items-center gap-1">
                        <Loader2 class="h-3 w-3 animate-spin" />
                        Загрузка...
                      </span>
                      <span class="text-[rgba(226,226,224,0.7)]">{downloadState.progress}%</span>
                    </div>
                    <div class="w-full h-1.5 bg-[#16161a] overflow-hidden border border-[rgba(226,226,224,0.1)] rounded-[1px]">
                      <div class="h-full bg-[#5865f2] transition-all duration-300" style="width: {downloadState.progress}%"></div>
                    </div>
                    <div class="flex items-center justify-between pt-0.5">
                      <span class="text-[9px] text-[rgba(226,226,224,0.45)] font-mono">
                        {#if downloadState.downloadedBytes > 0}
                          {formatBytes(downloadState.downloadedBytes)} / {formatBytes(downloadState.totalBytes)}
                        {:else}
                          Соединение с Hugging Face...
                        {/if}
                      </span>
                      <button
                        type="button"
                        on:click={() => handleCancelDownload(model.id)}
                        class="text-[9px] font-mono text-red-400 hover:underline cursor-pointer uppercase"
                      >
                        Отмена
                      </button>
                    </div>
                  </div>
                {:else if model.exists}
                  <div class="flex items-center gap-2 w-full sm:w-auto">
                    {#if isSelected}
                      <div class="label-mono text-[#23c55e] font-bold bg-[#23c55e]/10 border border-[#23c55e]/30 px-3 py-2 rounded-[2px] flex items-center gap-1.5">
                        <CheckCircle class="h-3.5 w-3.5" />
                        <span>ИСПОЛЬЗУЕТСЯ</span>
                      </div>
                    {:else}
                      <button
                        type="button"
                        on:click={() => handleSelect(model)}
                        class="btn py-2 px-3 border border-[#5865f2] bg-[#5865f2]/15 hover:bg-[#5865f2] text-[#e2e2e4] hover:text-white rounded-[2px] text-[10px] font-mono font-bold cursor-pointer transition-colors"
                        title="Сделать эту модель активной для обработки"
                      >
                        ВЫБРАТЬ
                      </button>
                    {/if}

                    <button
                      type="button"
                      on:click={() => handleDelete(model.id)}
                      disabled={isDeleting}
                      class="btn py-2 px-2.5 border border-red-500/30 bg-red-950/20 hover:bg-red-900/40 text-red-400 rounded-[2px] flex items-center gap-1 text-[10px] font-mono cursor-pointer transition-colors disabled:opacity-40"
                      title="Удалить модель с диска для освобождения места"
                    >
                      {#if isDeleting}
                        <Loader2 class="h-3.5 w-3.5 animate-spin" />
                      {:else}
                        <Trash2 class="h-3.5 w-3.5" />
                        <span>УДАЛИТЬ</span>
                      {/if}
                    </button>
                  </div>
                {:else}
                  <button 
                    type="button"
                    on:click={() => handleDownload(model.id)}
                    class="btn btn-primary py-2 px-4 rounded-[2px] flex items-center justify-center gap-1.5 w-full sm:w-auto cursor-pointer"
                  >
                    <Download class="h-3.5 w-3.5" />
                    <span>СКАЧАТЬ</span>
                  </button>
                {/if}
              </div>

            </div>
          {/each}
        {/if}
      </div>

    </div>

    <!-- Footer -->
    <div class="p-4 border-t border-[rgba(226,226,224,0.1)] bg-[#0c0c0e] flex items-center justify-between">
      <span class="label-mono">Прямая загрузка открытых GGUF/BIN моделей с Hugging Face (без ключей)</span>
      <button 
        type="button"
        on:click={onClose}
        class="btn py-2 px-4 border border-[rgba(226,226,224,0.2)] bg-transparent text-[#e2e2e4] hover:bg-[#16161a] rounded-[2px] cursor-pointer"
      >
        Закрыть
      </button>
    </div>
  </div>
</div>
