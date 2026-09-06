<!-- src/lib/components/ProgressDisplay.svelte -->
<script lang="ts">
  import { pipelineStore } from '../stores/pipeline';
  import { onDestroy, tick } from 'svelte';

  let logContainer: HTMLDivElement;

  $: if ($pipelineStore.logs.length > 0 && logContainer) {
    (async () => {
      await tick();
      logContainer.scrollTop = logContainer.scrollHeight;
    })();
  }

  function handleCancel() {
    pipelineStore.reset();
  }
</script>

<div class="panel p-6 sm:p-8 flex flex-col gap-6">
  <!-- Header -->
  <div class="panel-title flex items-center justify-between pb-4 border-b border-[rgba(226,226,224,0.1)]">
    <div class="flex items-center gap-2">
      <div class="w-3 h-3 rounded-full bg-[#5865f2] animate-ping"></div>
      <span>Выполнение конвейера</span>
    </div>
    <div class="label-mono text-[#5865f2] font-semibold">АКТИВНО</div>
  </div>

  <!-- Stage and Main Progress -->
  <div class="p-5 bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] rounded-[4px] space-y-3">
    <div class="flex justify-between items-center text-xs font-mono">
      <span class="text-[#e2e2e4] uppercase tracking-wider font-semibold">Этап: {$pipelineStore.currentStage}</span>
      <span class="text-[#5865f2] font-bold">{$pipelineStore.percentage}%</span>
    </div>

    <!-- Minimal Geometric Progress Bar -->
    <div class="w-full bg-[#16161a] h-2 rounded-none overflow-hidden border border-[rgba(226,226,224,0.1)]">
      <div 
        class="bg-[#5865f2] h-full transition-all duration-300" 
        style="width: {$pipelineStore.percentage}%"
      ></div>
    </div>
  </div>

  <!-- Sub-stages Breakdown -->
  {#if $pipelineStore.stageDetails}
    <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
      {#if $pipelineStore.stageDetails.ffmpeg}
        {@const f = $pipelineStore.stageDetails.ffmpeg}
        <div class="p-3.5 bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] rounded-[4px] space-y-2">
          <div class="flex justify-between items-center text-xs font-mono">
            <span class="text-[rgba(226,226,224,0.7)]">FFmpeg: Конвертация</span>
            <span class="text-[#5865f2]">{Math.round(f.percent)}%</span>
          </div>
          <div class="w-full bg-[#16161a] h-1.5 overflow-hidden">
            <div class="bg-[#5865f2] h-full" style="width: {f.percent}%"></div>
          </div>
          <p class="text-[10px] text-[rgba(226,226,224,0.4)] font-mono">
            {f.currentTime}с / {f.duration}с
          </p>
        </div>
      {/if}

      {#if $pipelineStore.stageDetails.whisper}
        {@const w = $pipelineStore.stageDetails.whisper}
        <div class="p-3.5 bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] rounded-[4px] space-y-2">
          <div class="flex justify-between items-center text-xs font-mono">
            <span class="text-[rgba(226,226,224,0.7)]">Whisper: {w.stage}</span>
            <span class="text-[#5865f2]">{Math.round(w.percent)}%</span>
          </div>
          <div class="w-full bg-[#16161a] h-1.5 overflow-hidden">
            <div class="bg-[#5865f2] h-full" style="width: {w.percent}%"></div>
          </div>
          <p class="text-[10px] text-[rgba(226,226,224,0.4)] font-mono">
            Локальная расшифровка ASR
          </p>
        </div>
      {/if}

      {#if $pipelineStore.stageDetails.analyzer}
        {@const a = $pipelineStore.stageDetails.analyzer}
        <div class="p-3.5 bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] rounded-[4px] space-y-2">
          <div class="flex justify-between items-center text-xs font-mono">
            <span class="text-[rgba(226,226,224,0.7)]">Qwen-VL: {a.stage}</span>
            <span class="text-[#5865f2]">{Math.round(a.percent)}%</span>
          </div>
          <div class="w-full bg-[#16161a] h-1.5 overflow-hidden">
            <div class="bg-[#5865f2] h-full" style="width: {a.percent}%"></div>
          </div>
          <p class="text-[10px] text-[rgba(226,226,224,0.4)] font-mono">
            Кадры: {a.currentFrame} / {a.totalFrames}
          </p>
        </div>
      {/if}

      {#if $pipelineStore.stageDetails.renderer}
        {@const r = $pipelineStore.stageDetails.renderer}
        <div class="p-3.5 bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] rounded-[4px] space-y-2">
          <div class="flex justify-between items-center text-xs font-mono">
            <span class="text-[rgba(226,226,224,0.7)]">BMF Рендер: {r.stage}</span>
            <span class="text-[#23c55e]">{Math.round(r.percent)}%</span>
          </div>
          <div class="w-full bg-[#16161a] h-1.5 overflow-hidden">
            <div class="bg-[#23c55e] h-full" style="width: {r.percent}%"></div>
          </div>
          <p class="text-[10px] text-[rgba(226,226,224,0.4)] font-mono">
            Нарезка и кроппинг (9:16)
          </p>
        </div>
      {/if}
    </div>
  {/if}

  <!-- Monospace Log Terminal -->
  <div class="space-y-2 flex-1 flex flex-col min-h-[160px]">
    <div class="label-mono flex items-center justify-between">
      <span>Консоль логирования бэкенда</span>
      <span class="text-[#5865f2]">Tokio Channel</span>
    </div>
    <div 
      bind:this={logContainer}
      class="w-full flex-1 bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] rounded-[4px] p-4 text-xs font-mono text-[rgba(226,226,224,0.7)] leading-relaxed overflow-y-auto max-h-[220px] space-y-1"
    >
      {#each $pipelineStore.logs as log}
        <div class="whitespace-pre-wrap select-text selection:bg-[#5865f2] selection:text-white">
          {log}
        </div>
      {/each}
      {#if $pipelineStore.logs.length === 0}
        <div class="text-[rgba(226,226,224,0.3)] italic">Ожидание инициализации логирования...</div>
      {/if}
    </div>
  </div>

  <!-- Cancel Action -->
  <button 
    on:click={handleCancel}
    class="font-mono p-3 border border-[rgba(226,226,224,0.15)] bg-transparent hover:bg-red-500/10 hover:border-red-500/40 text-red-400 uppercase text-xs font-bold tracking-[0.1em] cursor-pointer transition-colors rounded-[2px]"
  >
    Отменить операцию
  </button>
</div>
