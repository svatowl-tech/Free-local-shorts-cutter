<!-- src/routes/+page.svelte -->
<script lang="ts">
  import FileUpload from '../lib/components/FileUpload.svelte';
  import SettingsPanel from '../lib/components/SettingsPanel.svelte';
  import ProgressDisplay from '../lib/components/ProgressDisplay.svelte';
  import ResultsList from '../lib/components/ResultsList.svelte';
  import VideoPlayerWithTimeline from '../lib/components/VideoPlayerWithTimeline.svelte';
  import ModelDownloader from '../lib/components/ModelDownloader.svelte';
  import { pipelineStore } from '../lib/stores/pipeline';
  import { AlertTriangle, X } from '@lucide/svelte';

  let showModelDownloader = false;

  function dismissError() {
    pipelineStore.updateError(null);
  }

  function openModelManager() {
    showModelDownloader = true;
  }

  function closeModelManager() {
    showModelDownloader = false;
  }
</script>

<div class="min-h-screen bg-[#0c0c0e] text-[#e2e2e4] flex flex-col antialiased selection:bg-[#5865f2] selection:text-white">
  
  <!-- Header matching Variation 2 design HTML -->
  <header class="py-5 px-6 sm:px-8 border-b-[1.5px] border-[rgba(226,226,224,0.1)] flex flex-col sm:flex-row justify-between items-start sm:items-center gap-4 bg-gradient-to-b from-[#0c0c0e] to-transparent sticky top-0 z-40 backdrop-blur-md">
    <div class="flex items-center gap-3.5">
      <div class="w-[42px] h-[42px] bg-[#5865f2] rounded-[4px] flex items-center justify-center shrink-0 shadow-sm">
        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2">
          <rect x="4" y="4" width="16" height="16" rx="2"/>
          <rect x="8" y="8" width="8" height="8" rx="1"/>
          <path d="M12 2v2M12 20v2M2 12h2M20 12h2"/>
        </svg>
      </div>
      <div>
        <div class="label-mono leading-none mb-1">Автоматическая нарезка</div>
        <h1 class="font-syne text-lg sm:text-xl font-extrabold tracking-[-0.03em] leading-tight text-[#e2e2e4]">
          Video Cutter Pro v2.0
        </h1>
      </div>
    </div>

    <!-- Status Badges & Quick Action -->
    <div class="flex flex-wrap items-center gap-4 sm:gap-6">
      <button 
        on:click={openModelManager}
        class="label-mono hover:text-[#5865f2] transition-colors cursor-pointer border border-[rgba(226,226,224,0.15)] px-2.5 py-1 rounded-[2px] bg-[#16161a]"
      >
        Модели ИИ
      </button>
      <div class="label-mono flex items-center gap-1.5">
        <span class="text-[#23c55e]">●</span> Бэкенд: Rust Tokio
      </div>
      <div class="label-mono">
        BMF: GPU Active
      </div>
    </div>
  </header>

  <!-- Error Alert Banner -->
  {#if $pipelineStore.error}
    <div class="max-w-[1600px] w-full mx-auto px-6 sm:px-8 pt-4">
      <div class="p-4 bg-red-950/20 border-l-[3px] border-red-500 rounded-[2px] flex items-start justify-between gap-3 animate-in fade-in duration-200">
        <div class="text-xs font-mono flex-1">
          <div class="font-bold text-red-400 uppercase tracking-wider">ОБНАРУЖЕНА ОШИБКА ПАЙПЛАЙНА</div>
          <p class="text-[rgba(226,226,224,0.8)] mt-1 leading-relaxed">{$pipelineStore.error}</p>
        </div>
        <button on:click={dismissError} class="text-[rgba(226,226,224,0.4)] hover:text-white cursor-pointer" aria-label="Закрыть ошибку">
          <X class="h-4 w-4" />
        </button>
      </div>
    </div>
  {/if}

  <!-- Main Content Layout (Variation 2 grid) -->
  <main class="flex-1 w-full max-w-[1600px] mx-auto p-4 sm:p-6 md:p-8 flex flex-col">
    {#if $pipelineStore.status === 'idle'}
      <div class="grid grid-cols-1 lg:grid-cols-[1fr_400px] gap-6 flex-1">
        
        <!-- Left Column: Dropzone, Video Player, and Pipeline Info Card -->
        <div class="flex flex-col gap-6">
          <FileUpload />
          
          {#if $pipelineStore.video}
            <VideoPlayerWithTimeline />
          {/if}

          <!-- Instruction Info Card from Variation 2 -->
          <div class="info-card">
            <div class="label-mono text-[#e2e2e4] mb-1.5">Инструкция по обработке</div>
            <p class="text-[rgba(226,226,224,0.7)] leading-relaxed">
              Загрузите исходное видео. Пайплайн автоматически извлечет аудио с помощью <strong class="text-[#e2e2e4] font-mono">FFmpeg</strong>, распознает речь через <strong class="text-[#e2e2e4] font-mono">Whisper</strong>, выполнит ИИ-анализ поведения лиц по модели <strong class="text-[#e2e2e4] font-mono">Qwen3-VL 2B</strong> и произведет кроппинг в портретный <span class="text-[#5865f2] font-semibold">9:16</span> формат на медиадвижке <strong class="text-[#e2e2e4] font-mono">BMF</strong>.
            </p>
          </div>
        </div>

        <!-- Right Column: Controls Panel -->
        <div class="h-full">
          <SettingsPanel onOpenModelManager={openModelManager} />
        </div>
      </div>

    {:else}
      <!-- Active Execution or Results State -->
      <div class="max-w-4xl w-full mx-auto my-auto py-4">
        {#if $pipelineStore.status === 'running'}
          <ProgressDisplay />
        {:else if $pipelineStore.status === 'completed'}
          <ResultsList />
        {:else if $pipelineStore.status === 'failed'}
          <div class="panel p-8 text-center flex flex-col items-center justify-center gap-4">
            <div class="w-12 h-12 bg-red-500/10 border border-red-500/30 text-red-400 rounded-full flex items-center justify-center">
              <AlertTriangle class="h-6 w-6" />
            </div>
            <div>
              <h2 class="panel-title text-base text-red-400">Сбой выполнения операции</h2>
              <p class="text-xs text-[rgba(226,226,224,0.6)] font-mono mt-1 max-w-md leading-relaxed">
                Во время выполнения пайплайна произошла непредвиденная ошибка. Проверьте логи или статус доступности моделей.
              </p>
            </div>
            <button 
              on:click={() => pipelineStore.reset()} 
              class="btn btn-primary py-3 px-8 rounded-[2px] mt-2 font-mono text-xs"
            >
              Повторить настройку
            </button>
          </div>
        {/if}
      </div>
    {/if}
  </main>

  <!-- Footer matching Variation 2 -->
  <footer class="py-4 px-6 sm:px-8 border-t border-[rgba(226,226,224,0.1)] flex flex-col sm:flex-row justify-between items-center gap-3">
    <div class="label-mono">Tauri 2 + Rust + SvelteKit</div>
    <div class="label-mono text-[#5865f2]">Tokio pipelines & BMF rendering</div>
  </footer>

  <!-- Model Manager Modal -->
  {#if showModelDownloader}
    <ModelDownloader onClose={closeModelManager} />
  {/if}
</div>
