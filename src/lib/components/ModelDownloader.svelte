<!-- src/lib/components/ModelDownloader.svelte -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { modelsStore } from '../stores/models';
  import { 
    Download, 
    CheckCircle, 
    AlertCircle, 
    Loader2, 
    Folder, 
    RefreshCw, 
    Check,
    X
  } from '@lucide/svelte';

  export let onClose: () => void = () => {};

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

<!-- Backdrop -->
<div 
  class="fixed inset-0 bg-[#0c0c0e]/80 backdrop-blur-sm flex items-center justify-center z-50 p-4" 
  on:click|self={onClose} 
  role="dialog" 
  aria-modal="true" 
  aria-label="Менеджер моделей"
>
  <div class="panel w-full max-w-2xl overflow-hidden shadow-2xl flex flex-col max-h-[90vh] bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[4px]">
    
    <!-- Modal Header -->
    <div class="p-6 border-b border-[rgba(226,226,224,0.1)] bg-[#0c0c0e]/40 flex items-center justify-between">
      <div class="flex items-center gap-3">
        <div class="w-10 h-10 bg-[#5865f2] rounded-[4px] flex items-center justify-center text-white">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="4" y="4" width="16" height="16" rx="2"/>
            <rect x="8" y="8" width="8" height="8" rx="1"/>
            <path d="M12 2v2M12 20v2M2 12h2M20 12h2"/>
          </svg>
        </div>
        <div>
          <div class="label-mono">АВТОНОМНЫЕ ВЕСА</div>
          <h2 class="panel-title text-sm">Менеджер ИИ-Моделей</h2>
        </div>
      </div>
      
      <div class="flex items-center gap-2">
        <button 
          on:click={handleRefresh}
          class="btn py-1.5 px-3 border border-[rgba(226,226,224,0.15)] bg-transparent text-[rgba(226,226,224,0.7)] hover:text-[#e2e2e4] text-[10px] flex items-center gap-1.5 cursor-pointer rounded-[2px]"
          title="Обновить статусы файлов"
        >
          <RefreshCw class="h-3 w-3 {$modelsStore.loading ? 'animate-spin' : ''}" />
          <span>ОБНОВИТЬ</span>
        </button>
        <button 
          on:click={onClose}
          class="p-1.5 text-[rgba(226,226,224,0.5)] hover:text-[#e2e2e4] cursor-pointer"
          aria-label="Закрыть"
        >
          <X class="h-5 w-5" />
        </button>
      </div>
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
        <div class="label-mono text-[#e2e2e4] mb-1">Локальное хранение весов нейросетей</div>
        Все веса и бинарники сохраняются напрямую в системную папку ресурсов приложения <strong class="text-[#5865f2] font-mono">/models</strong>. Это гарантирует 100% автономную и приватную работу без интернета после их первой загрузки!
      </div>

      <div class="space-y-3 pt-2">
        {#if $modelsStore.loading && $modelsStore.models.length === 0}
          <div class="flex flex-col items-center justify-center py-12 text-[rgba(226,226,224,0.4)] gap-2 font-mono">
            <Loader2 class="h-6 w-6 animate-spin text-[#5865f2]" />
            <span class="text-xs">Сканирование моделей...</span>
          </div>
        {:else}
          {#each $modelsStore.models as model}
            {@const downloadState = $modelsStore.downloads[model.id]}
            <div class="bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] p-4 rounded-[4px] flex flex-col sm:flex-row sm:items-center justify-between gap-4">
              
              <!-- Info -->
              <div class="flex-1 space-y-1">
                <div class="flex items-center gap-2">
                  <h3 class="text-xs font-bold text-[#e2e2e4] font-syne">{model.name}</h3>
                  <span class="label-mono text-[#5865f2] font-semibold bg-[#5865f2]/10 px-1.5 py-0.5 rounded-[2px]">
                    {model.size_mb} MB
                  </span>
                  {#if model.exists}
                    <span class="label-mono text-[#23c55e] font-semibold bg-[#23c55e]/10 px-1.5 py-0.5 rounded-[2px] flex items-center gap-1">
                      <Check class="h-2.5 w-2.5" /> Доступно
                    </span>
                  {/if}
                </div>
                <p class="text-xs text-[rgba(226,226,224,0.6)] leading-normal">{model.description}</p>
                <div class="flex items-center gap-1 text-[10px] text-[rgba(226,226,224,0.4)] font-mono mt-1">
                  <Folder class="h-3 w-3" />
                  <span>Файл: {model.filename}</span>
                </div>
              </div>

              <!-- Action button or progress -->
              <div class="sm:shrink-0 flex items-center">
                {#if downloadState && downloadState.status === 'downloading'}
                  <div class="w-full sm:w-48 space-y-1">
                    <div class="flex justify-between text-[10px] font-mono">
                      <span class="text-[#5865f2] font-bold">Скачивание...</span>
                      <span class="text-[rgba(226,226,224,0.5)]">{downloadState.progress}%</span>
                    </div>
                    <div class="w-full h-1.5 bg-[#16161a] overflow-hidden border border-[rgba(226,226,224,0.1)]">
                      <div class="h-full bg-[#5865f2] transition-all duration-300" style="width: {downloadState.progress}%"></div>
                    </div>
                    {#if downloadState.downloadedBytes > 0}
                      <p class="text-[9px] text-[rgba(226,226,224,0.4)] text-right font-mono">
                        {formatBytes(downloadState.downloadedBytes)} / {formatBytes(downloadState.totalBytes)}
                      </p>
                    {/if}
                  </div>
                {:else if model.exists}
                  <div class="label-mono text-[#23c55e] font-bold bg-[#23c55e]/10 border border-[#23c55e]/30 px-3 py-1.5 rounded-[2px] flex items-center gap-1">
                    <CheckCircle class="h-3.5 w-3.5" />
                    <span>ГОТОВО</span>
                  </div>
                {:else}
                  <button 
                    on:click={() => handleDownload(model.id)}
                    class="btn btn-primary py-2 px-4 rounded-[2px] flex items-center justify-center gap-1.5 w-full sm:w-auto"
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
      <span class="label-mono">Загрузка с защищенного Hugging Face репозитория</span>
      <button 
        on:click={onClose}
        class="btn py-2 px-4 border border-[rgba(226,226,224,0.2)] bg-transparent text-[#e2e2e4] hover:bg-[#16161a] rounded-[2px]"
      >
        Закрыть
      </button>
    </div>
  </div>
</div>
