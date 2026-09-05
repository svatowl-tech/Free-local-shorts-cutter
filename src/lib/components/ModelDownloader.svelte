<!-- src/lib/components/ModelDownloader.svelte -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { modelsStore, type ModelDownloadState } from '../stores/models';
  import { 
    Download, 
    CheckCircle, 
    AlertCircle, 
    Loader2, 
    Folder, 
    HelpCircle, 
    RefreshCw, 
    Cpu, 
    Check,
    Compass
  } from '@lucide/svelte';

  export let onClose: () => void = () => {};

  // Функция форматирования байтов
  function formatBytes(bytes: number): string {
    if (!bytes || bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
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

  function handleRefresh() {
    modelsStore.init();
  }
</script>

<div class="fixed inset-0 bg-slate-950/80 backdrop-blur-sm flex items-center justify-center z-50 p-4" on:click|self={onClose} role="dialog" aria-modal="true" aria-label="Менеджер моделей">
  <div class="bg-slate-900 border border-slate-800 rounded-3xl w-full max-w-2xl overflow-hidden shadow-2xl flex flex-col max-h-[90vh] animate-in fade-in zoom-in-95 duration-200">
    <!-- Шапка модального окна -->
    <div class="p-6 border-b border-slate-850 bg-slate-900/40 flex items-center justify-between">
      <div class="flex items-center gap-3">
        <div class="h-10 w-10 bg-indigo-950/40 border border-indigo-900/30 rounded-xl flex items-center justify-center text-indigo-400">
          <Cpu class="h-5 w-5" />
        </div>
        <div>
          <h2 class="text-sm font-bold tracking-widest text-slate-200 uppercase font-mono">Менеджер ИИ-Моделей</h2>
          <p class="text-xs text-slate-500 mt-0.5">Скачивайте веса распознавания Whisper и анализа Qwen в 1 клик</p>
        </div>
      </div>
      
      <button 
        on:click={handleRefresh}
        class="p-2 bg-slate-950 border border-slate-800 rounded-lg text-slate-400 hover:text-slate-200 transition-colors flex items-center gap-1 cursor-pointer text-xs font-mono"
        title="Обновить статусы файлов"
      >
        <RefreshCw class="h-3.5 w-3.5 {$modelsStore.loading ? 'animate-spin' : ''}" />
        Обновить
      </button>
    </div>

    <!-- Основной контент скроллируемый -->
    <div class="p-6 overflow-y-auto space-y-4 flex-1">
      
      {#if $modelsStore.error}
        <div class="p-4 bg-red-950/20 border border-red-900/40 rounded-xl flex items-start gap-3">
          <AlertCircle class="h-5 w-5 text-red-400 shrink-0" />
          <div class="text-xs">
            <h4 class="font-bold text-red-400 uppercase tracking-wide">Ошибка при работе с моделями</h4>
            <p class="text-slate-300 mt-1 leading-normal">{$modelsStore.error}</p>
          </div>
        </div>
      {/if}

      <div class="bg-indigo-950/20 border border-indigo-900/30 p-4 rounded-xl flex gap-3.5 items-start">
        <Compass class="h-5 w-5 text-indigo-400 shrink-0 mt-0.5" />
        <div class="text-xs">
          <h4 class="font-bold text-indigo-300">Локальное хранение весов нейросетей</h4>
          <p class="text-slate-400 mt-1 leading-relaxed">
            Все веса и бинарники сохраняются напрямую в системную папку ресурсов приложения <code class="bg-slate-950/60 text-slate-300 font-mono px-1 py-0.5 rounded text-[11px]">/models</code>. Это гарантирует 100% автономную и безопасную работу без интернета после их первой загрузки!
          </p>
        </div>
      </div>

      <div class="space-y-3.5 pt-2">
        {#if $modelsStore.loading && $modelsStore.models.length === 0}
          <div class="flex flex-col items-center justify-center py-12 text-slate-500 gap-2">
            <Loader2 class="h-8 w-8 animate-spin text-indigo-500" />
            <span class="text-xs">Сканирование папки ресурсов и загрузка конфигураций...</span>
          </div>
        {:else}
          {#each $modelsStore.models as model}
            {@const downloadState = $modelsStore.downloads[model.id]}
            <div class="bg-slate-950 border border-slate-850/70 p-4.5 rounded-2xl flex flex-col sm:flex-row sm:items-center justify-between gap-4 transition-all hover:border-slate-800">
              
              <!-- Инфо модели -->
              <div class="flex-1 space-y-1">
                <div class="flex items-center gap-2">
                  <h3 class="text-sm font-bold text-slate-200">{model.name}</h3>
                  <span class="text-[10px] font-mono font-semibold bg-slate-900 border border-slate-800 px-2 py-0.5 rounded text-indigo-400">
                    {model.size_mb} MB
                  </span>
                  {#if model.exists}
                    <span class="text-[10px] font-mono font-bold uppercase bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 px-1.5 py-0.5 rounded flex items-center gap-0.5">
                      <Check class="h-3 w-3" /> Доступно
                    </span>
                  {/if}
                </div>
                <p class="text-xs text-slate-400 leading-normal">{model.description}</p>
                <div class="flex items-center gap-1.5 text-[10px] text-slate-600 font-mono mt-1">
                  <Folder class="h-3.5 w-3.5" />
                  <span>Файл: {model.filename}</span>
                </div>
              </div>

              <!-- Кнопка действия / лоадер -->
              <div class="sm:shrink-0 flex items-center">
                {#if downloadState && downloadState.status === 'downloading'}
                  <!-- Состояние загрузки (Прогресс-бар) -->
                  <div class="w-full sm:w-48 space-y-1.5">
                    <div class="flex justify-between text-[11px] font-mono">
                      <span class="text-indigo-400 font-bold">Скачивание...</span>
                      <span class="text-slate-400">{downloadState.progress}%</span>
                    </div>
                    <!-- Фоновый контейнер -->
                    <div class="w-full h-1.5 bg-slate-900 rounded-full overflow-hidden border border-slate-800">
                      <div class="h-full bg-indigo-500 rounded-full transition-all duration-300" style="width: {downloadState.progress}%"></div>
                    </div>
                    {#if downloadState.downloadedBytes > 0}
                      <p class="text-[10px] text-slate-500 text-right font-mono">
                        {formatBytes(downloadState.downloadedBytes)} / {formatBytes(downloadState.totalBytes)}
                      </p>
                    {/if}
                  </div>
                {:else if model.exists}
                  <!-- Уже скачано -->
                  <div class="flex items-center gap-1.5 text-xs text-emerald-400/90 font-bold bg-emerald-500/5 px-3.5 py-2 border border-emerald-500/10 rounded-xl">
                    <CheckCircle class="h-4 w-4" />
                    <span>ГОТОВО</span>
                  </div>
                {:else}
                  <!-- Кнопка "Скачать" -->
                  <button 
                    on:click={() => handleDownload(model.id)}
                    class="w-full sm:w-auto cursor-pointer bg-indigo-600 hover:bg-slate-850 border border-indigo-500/30 text-white font-bold text-xs py-2 px-4 rounded-xl transition duration-200 flex items-center justify-center gap-1.5 shadow-lg shadow-indigo-600/10"
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

    <!-- Подвал модального окна -->
    <div class="p-5 border-t border-slate-850/60 bg-slate-950/20 flex items-center justify-between text-[10px] font-mono text-slate-500">
      <span class="flex items-center gap-1">
        <HelpCircle class="h-3.5 w-3.5 text-indigo-400" />
        Веса загружаются напрямую с защищенного репозитория Hugging Face
      </span>
      <button 
        on:click={onClose}
        class="bg-slate-800 hover:bg-slate-755 text-slate-200 border border-slate-700 font-bold px-4 py-2 rounded-xl text-xs transition-colors cursor-pointer"
      >
        Закрыть
      </button>
    </div>
  </div>
</div>
