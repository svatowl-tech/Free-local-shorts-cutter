<!-- src/routes/+page.svelte -->
<script lang="ts">
  import FileUpload from '../lib/components/FileUpload.svelte';
  import SettingsPanel from '../lib/components/SettingsPanel.svelte';
  import ProgressDisplay from '../lib/components/ProgressDisplay.svelte';
  import ResultsList from '../lib/components/ResultsList.svelte';
  import VideoPlayerWithTimeline from '../lib/components/VideoPlayerWithTimeline.svelte';
  import { pipelineStore } from '../lib/stores/pipeline';
  import { Sparkles, Terminal, ShieldAlert, Cpu, AlertTriangle, XCircle, Info } from '@lucide/svelte';

  function dismissError() {
    pipelineStore.updateError(null);
  }
</script>

<main class="min-h-screen bg-slate-950 text-slate-100 flex flex-col antialiased selection:bg-indigo-500/20 selection:text-white">
  <header class="border-b border-slate-900 bg-slate-900/40 backdrop-blur-md sticky top-0 z-50 px-4 py-3 md:py-4">
    <div class="max-w-7xl w-full mx-auto flex flex-col sm:flex-row items-center justify-between gap-4">
      <div class="flex items-center gap-3">
        <div class="h-9 w-9 bg-gradient-to-tr from-indigo-500 to-indigo-600 rounded-xl flex items-center justify-center text-white shadow-lg shadow-indigo-500/10 border border-indigo-400/20">
          <Cpu class="h-5 w-5 animate-pulse" />
        </div>
        <div>
          <h1 class="text-xs font-bold tracking-widest text-slate-400 uppercase font-mono leading-tight">Автоматическая нарезка</h1>
          <p class="text-sm font-black text-slate-100 leading-none tracking-tight">Video Cutter Pro <span class="text-indigo-400 font-mono text-xs font-semibold">v2.0</span></p>
        </div>
      </div>

      <div class="flex items-center gap-3.5 text-xs font-mono">
        <span class="flex items-center gap-1 bg-slate-900 border border-slate-800 px-3 py-1.5 rounded-xl text-slate-400">
          <span class="w-1.5 h-1.5 bg-emerald-500 rounded-full animate-ping"></span>
          Бэкенд: Rust Tokio
        </span>
        <span class="flex items-center gap-1 bg-slate-900 border border-slate-800 px-3 py-1.5 rounded-xl text-slate-400">
          BMF: GPU Active
        </span>
      </div>
    </div>
  </header>

  <div class="flex-1 max-w-7xl w-full mx-auto p-4 md:p-6 flex flex-col gap-5">
    {#if $pipelineStore.error}
      <div class="bg-red-950/20 border border-red-900/40 rounded-2xl p-4 flex items-start gap-3.5 relative overflow-hidden animate-in fade-in slide-in-from-top-3 duration-200">
        <div class="p-2 bg-red-950/40 text-red-400 border border-red-900/30 rounded-xl">
          <XCircle class="h-5 w-5" />
        </div>
        <div class="flex-1 pr-8">
          <h4 class="text-xs font-bold text-red-400 font-mono uppercase tracking-wider">Обнаружена ошибка пайплайна</h4>
          <p class="text-xs text-slate-300 mt-1 leading-relaxed">{$pipelineStore.error}</p>
        </div>
        <button on:click={dismissError} class="absolute top-4 right-4 text-slate-500 hover:text-slate-300 transition-colors cursor-pointer">
          <svg class="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
    {/if}

    {#if $pipelineStore.status === 'idle'}
      <div class="grid grid-cols-1 lg:grid-cols-12 gap-6">
        <div class="lg:col-span-7 flex flex-col gap-6">
          <FileUpload />
          {#if $pipelineStore.video}
            <VideoPlayerWithTimeline />
          {/if}
          <div class="bg-slate-900/40 border border-slate-900 rounded-2xl p-5 flex items-start gap-4">
            <div class="bg-indigo-950/40 border border-indigo-900/30 p-2.5 rounded-xl text-indigo-400 hidden sm:block">
              <Info class="h-5 w-5 animate-bounce" style="animation-duration: 2.5s" />
            </div>
            <div class="text-xs">
              <h4 class="font-bold text-slate-200">Инструкция по обработке</h4>
              <p class="text-slate-400 mt-1 leading-relaxed">
                Загрузите исходное видео с диалогами. Пайплайн автоматически извлечет аудио с помощью <strong class="text-slate-200 font-mono text-[11px]">FFmpeg</strong>, распознает речь через <strong class="text-slate-200 font-mono text-[11px]">Whisper</strong>, выполнит ИИ-анализ поведения лиц по модели <strong class="text-slate-200 font-mono text-[11px]">Qwen3-VL 2B</strong> и произведет кроппинг в портретный <strong class="text-indigo-400">9:16</strong> формат на медиадвижке <strong class="text-slate-200 font-mono text-[11px]">BMF</strong>.
              </p>
            </div>
          </div>
        </div>
        <div class="lg:col-span-5">
          <SettingsPanel />
        </div>
      </div>
    {:else}
      <div class="max-w-3xl w-full mx-auto">
        {#if $pipelineStore.status === 'running'}
          <ProgressDisplay />
        {:else if $pipelineStore.status === 'completed'}
          <ResultsList />
        {:else if $pipelineStore.status === 'failed'}
          <div class="bg-slate-900/60 border border-slate-800 rounded-2xl p-6 shadow-xl text-center flex flex-col items-center justify-center gap-4">
            <div class="p-4 bg-red-950/30 border border-red-900/30 text-red-400 rounded-full">
              <AlertTriangle class="h-8 w-8 animate-bounce" />
            </div>
            <div>
              <h2 class="text-base font-bold text-slate-100 font-mono uppercase tracking-wider">Сбой выполнения операции</h2>
              <p class="text-xs text-slate-400 mt-1 max-w-md leading-relaxed">
                Что-то пошло не так во время работы асинхронного Rust Tokio конвейера. Убедитесь, что бинарники FFmpeg и плагин Whisper правильно установлены.
              </p>
            </div>
            <div class="w-full max-w-sm mt-2 flex flex-col gap-2">
              <button on:click={() => pipelineStore.reset()} class="w-full bg-slate-950 hover:bg-slate-925 text-slate-200 border border-slate-850 hover:border-slate-805 font-bold py-2.5 px-4 rounded-xl transition duration-200 text-xs tracking-wider cursor-pointer">
                ПОВТОРИТЬ НАСТРОЙКУ
              </button>
            </div>
          </div>
        {/if}
      </div>
    {/if}
  </div>
  <footer class="border-t border-slate-900 py-4 px-4 bg-slate-950">
    <div class="max-w-7xl w-full mx-auto flex flex-col sm:flex-row justify-between items-center gap-3 text-[10px] font-mono text-slate-500">
      <p>Разработано Старшим AI-Архитектором на Tauri 2 + Rust + SvelteKit</p>
      <p class="text-indigo-400/80">Отказоустойчивые Tokio пайплайны & BMF рендеринг</p>
    </div>
  </footer>
</main>
