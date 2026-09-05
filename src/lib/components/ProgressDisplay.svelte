<!-- src/lib/components/ProgressDisplay.svelte -->
<script lang="ts">
  import { pipelineStore } from '../stores/pipeline';
  import { Cpu, Terminal, RefreshCw, AudioLines, Sparkles, Film, Loader2 } from '@lucide/svelte';
  import { onDestroy, tick } from 'svelte';

  let logContainer: HTMLDivElement;

  // Автоскролл консоли логов вниз при обновлении
  $: if ($pipelineStore.logs.length > 0 && logContainer) {
    (async () => {
      await tick();
      logContainer.scrollTop = logContainer.scrollHeight;
    })();
  }

  function handleCancel() {
    // В реальном приложении отправляет сигнал отмены,
    // здесь мы просто перезапускаем состояние.
    pipelineStore.reset();
  }
</script>

<div class="bg-slate-900/60 border border-slate-800 rounded-2xl p-5 shadow-xl backdrop-blur-md flex flex-col gap-5">
  <div class="flex items-center justify-between">
    <div class="flex items-center gap-2">
      <Cpu class="h-5 w-5 text-indigo-400 animate-spin" style="animation-duration: 3s" />
      <h2 class="text-sm font-bold tracking-widest text-slate-200 uppercase font-mono">Выполнение конвейера</h2>
    </div>
    <div class="flex items-center gap-1.5 text-xs text-indigo-400 font-mono">
      <Loader2 class="h-3 w-3 animate-spin text-indigo-400" />
      АКТИВНО
    </div>
  </div>

  <!-- Главный индикатор прогресса -->
  <div class="bg-slate-950 border border-slate-850 p-4 rounded-xl space-y-3">
    <div class="flex justify-between items-center text-xs font-mono">
      <span class="text-indigo-300 font-semibold uppercase tracking-wider">Этап: {$pipelineStore.currentStage}</span>
      <span class="text-slate-400 font-bold">{$pipelineStore.percentage}%</span>
    </div>

    <!-- Текстурный прогрессбар -->
    <div class="w-full bg-slate-900 rounded-full h-3.5 overflow-hidden border border-slate-800 p-0.5">
      <div 
        class="bg-gradient-to-r from-indigo-500 via-sky-400 to-emerald-400 h-full rounded-full transition-all duration-300 shadow-[0_0_10px_rgba(99,102,241,0.5)]" 
        style="width: {$pipelineStore.percentage}%"
      ></div>
    </div>
  </div>

  <!-- Вложенные специфические детали текущего этапа -->
  {#if $pipelineStore.stageDetails}
    <div class="space-y-4">
      <!-- 1. FFmpeg Details -->
      {#if $pipelineStore.stageDetails.ffmpeg}
        {@const f = $pipelineStore.stageDetails.ffmpeg}
        <div class="bg-slate-950/40 border border-slate-850/60 rounded-xl p-3.5 space-y-2.5">
          <div class="flex justify-between items-center text-xs font-mono">
            <span class="text-slate-300 flex items-center gap-1.5">
              <AudioLines class="h-3.5 w-3.5 text-indigo-400 animate-pulse" />
              FFmpeg: Конвертация аудио
            </span>
            <span class="text-indigo-400 font-bold">{Math.round(f.percent)}%</span>
          </div>
          <div class="w-full bg-slate-950 rounded-full h-1.5 overflow-hidden">
            <div class="bg-indigo-500 h-full" style="width: {f.percent}%"></div>
          </div>
          <p class="text-[10px] text-slate-500 font-mono">
            Обработано: <span class="text-slate-300">{f.currentTime}с</span> из <span class="text-slate-300">{f.duration}с</span> видео дорожки.
          </p>
        </div>
      {/if}

      <!-- 2. Whisper Details -->
      {#if $pipelineStore.stageDetails.whisper}
        {@const w = $pipelineStore.stageDetails.whisper}
        <div class="bg-slate-950/40 border border-slate-850/60 rounded-xl p-3.5 space-y-3">
          <div class="flex justify-between items-center text-xs font-mono">
            <span class="text-slate-300 flex items-center gap-1.5">
              <Sparkles class="h-3.5 w-3.5 text-indigo-400" />
              Whisper CPP: {w.stage}
            </span>
            <span class="text-indigo-400 font-bold">{Math.round(w.percent)}%</span>
          </div>
          <div class="w-full bg-slate-950 rounded-full h-1.5 overflow-hidden">
            <div class="bg-sky-400 h-full" style="width: {w.percent}%"></div>
          </div>
          <p class="text-[10px] text-slate-500 font-mono">
            Локальный ASR: Распознавание субтитров на основе ИИ-транскрипции речи.
          </p>
        </div>
      {/if}

      <!-- 3. Qwen Analyzer Details -->
      {#if $pipelineStore.stageDetails.analyzer}
        {@const a = $pipelineStore.stageDetails.analyzer}
        <div class="bg-slate-950/40 border border-slate-850/60 rounded-xl p-3.5 space-y-2.5">
          <div class="flex justify-between items-center text-xs font-mono">
            <span class="text-slate-300 flex items-center gap-1.5">
              <Cpu class="h-3.5 w-3.5 text-indigo-400" />
              Qwen3-VL 2B: {a.stage}
            </span>
            <span class="text-indigo-400 font-bold">{Math.round(a.percent)}%</span>
          </div>
          <div class="w-full bg-slate-950 rounded-full h-1.5 overflow-hidden">
            <div class="bg-violet-500 h-full" style="width: {a.percent}%"></div>
          </div>
          <p class="text-[10px] text-slate-500 font-mono">
            Кадры: <span class="text-slate-300">{a.currentFrame}</span> / <span class="text-slate-300">{a.totalFrames}</span> (Инференс через llama.cpp)
          </p>
        </div>
      {/if}

      <!-- 4. Renderer Details -->
      {#if $pipelineStore.stageDetails.renderer}
        {@const r = $pipelineStore.stageDetails.renderer}
        <div class="bg-slate-950/40 border border-slate-850/60 rounded-xl p-3.5 space-y-2.5">
          <div class="flex justify-between items-center text-xs font-mono">
            <span class="text-slate-300 flex items-center gap-1.5">
              <Film class="h-3.5 w-3.5 text-indigo-400 animate-pulse" />
              BMF мультимедиа-движок: {r.stage}
            </span>
            <span class="text-indigo-400 font-bold">{Math.round(r.percent)}%</span>
          </div>
          <div class="w-full bg-slate-950 rounded-full h-1.5 overflow-hidden">
            <div class="bg-emerald-400 h-full" style="width: {r.percent}%"></div>
          </div>
          <p class="text-[10px] text-slate-500 font-mono">
            Генерация клипов с авто-кропом (HW NVENC / VideoToolbox ускорение)
          </p>
        </div>
      {/if}
    </div>
  {/if}

  <!-- Лог-консоль (Коды инициализации и выполнения) -->
  <div class="space-y-1.5 flex-1 flex flex-col min-h-[160px]">
    <div class="flex items-center gap-1.5 text-[10px] font-bold font-mono text-slate-400 uppercase tracking-widest pl-1">
      <Terminal class="h-3.5 w-3.5 text-slate-500" />Консоль логирования бэкенда
    </div>
    <div 
      bind:this={logContainer}
      class="w-full flex-1 bg-slate-950 border border-slate-850 rounded-xl p-3 text-[11px] font-mono text-indigo-300/90 leading-relaxed overflow-y-auto max-h-[180px] space-y-1 scrollbar-thin scrollbar-thumb-slate-850"
    >
      {#each $pipelineStore.logs as log}
        <div class="whitespace-pre-wrap select-text selection:bg-indigo-500/30 selection:text-white">
          {log}
        </div>
      {/each}
      {#if $pipelineStore.logs.length === 0}
        <div class="text-slate-600 italic">Ожидание инициализации логирования...</div>
      {/if}
    </div>
  </div>

  <!-- Кнопка Отмена -->
  <button 
    on:click={handleCancel}
    class="w-full border border-red-900/30 cursor-pointer bg-red-950/10 hover:bg-red-950/30 text-red-400 hover:text-red-300 font-medium py-2 rounded-xl transition duration-200 text-xs tracking-wider"
  >
    ОТМЕНИТЬ ОПЕРАЦИЮ
  </button>
</div>
