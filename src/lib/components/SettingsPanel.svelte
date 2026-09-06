<!-- src/lib/components/SettingsPanel.svelte -->
<script lang="ts">
  import { pipelineStore } from '../stores/pipeline';
  import { fragmentsStore } from '../stores/fragments';
  import { ChevronDown, ChevronUp } from '@lucide/svelte';

  export let onOpenModelManager: () => void = () => {};

  let showAdvanced = false;

  function handleIntensityChange(e: Event) {
    const value = parseFloat((e.target as HTMLInputElement).value);
    pipelineStore.updateConfig({ min_intensity_threshold: value });
  }

  function handleDurationChange(e: Event) {
    const value = parseInt((e.target as HTMLInputElement).value, 10);
    pipelineStore.updateConfig({ max_clip_duration_sec: value });
  }

  function handleVerticalChange(e: Event) {
    const value = (e.target as HTMLInputElement).checked;
    pipelineStore.updateConfig({ vertical_format: value });
  }

  function handleAsrModelChange(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    pipelineStore.updateConfig({ asr_model_path: value });
  }

  function handleVisionModelChange(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    pipelineStore.updateConfig({ vision_model_path: value });
  }

  function handleOutputDirChange(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    pipelineStore.updateConfig({ output_dir: value });
  }

  function handleTextEngineChange(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    pipelineStore.updateConfig({ text_engine: value });
  }

  function handleTextModelChange(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    pipelineStore.updateConfig({ text_model_name: value });
  }

  function handleOllamaUrlChange(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    pipelineStore.updateConfig({ ollama_url: value });
  }

  function handleModeChange(mode: 'auto' | 'manual' | 'confirm') {
    pipelineStore.updateConfig({ mode });
  }

  function exportClips() {
    let currentFragments: any[] = [];
    fragmentsStore.subscribe(f => currentFragments = f)();
    pipelineStore.exportAndRenderClips(currentFragments);
  }

  function toggleAdvanced() {
    showAdvanced = !showAdvanced;
  }

  function startProcess() {
    pipelineStore.startPipelineProcess();
  }
</script>

<div class="panel p-6 sm:p-8 flex flex-col justify-between h-full">
  <div>
    <!-- Panel Header -->
    <div class="panel-title flex items-center justify-between pb-4 mb-6 border-b border-[rgba(226,226,224,0.1)]">
      <span>Параметры обработки</span>
      <span class="label-mono font-mono">tokio-engine v2.0</span>
    </div>

    <!-- Controls Stack -->
    <div class="flex flex-col gap-6">
      
      <!-- Control 1: Emotion Intensity -->
      <div class="flex flex-col gap-2">
        <div class="flex justify-between items-baseline">
          <span class="label-mono text-[#e2e2e4]">Интенсивность эмоций</span>
          <span class="font-mono text-[#5865f2] text-xs font-semibold">
            {$pipelineStore.config.min_intensity_threshold}
          </span>
        </div>
        <input 
          type="range" 
          min="0" 
          max="1" 
          step="0.05" 
          value={$pipelineStore.config.min_intensity_threshold}
          on:input={handleIntensityChange}
          disabled={$pipelineStore.status === 'running'}
        />
        <div class="flex justify-between items-center text-[10px] text-[rgba(226,226,224,0.4)] font-mono">
          <span>0.0 (Все подряд)</span>
          <span>1.0 (Пики)</span>
        </div>
      </div>

      <!-- Control 2: Max Clip Duration -->
      <div class="flex flex-col gap-2">
        <div class="flex justify-between items-baseline">
          <span class="label-mono text-[#e2e2e4]">Макс. длительность</span>
          <span class="font-mono text-[#5865f2] text-xs font-semibold">
            {$pipelineStore.config.max_clip_duration_sec} сек
          </span>
        </div>
        <input 
          type="range" 
          min="5" 
          max="60" 
          step="1" 
          value={$pipelineStore.config.max_clip_duration_sec}
          on:input={handleDurationChange}
          disabled={$pipelineStore.status === 'running'}
        />
        <div class="flex justify-between items-center text-[10px] text-[rgba(226,226,224,0.4)] font-mono">
          <span>5 с (Шортсы)</span>
          <span>60 сек</span>
        </div>
      </div>

      <!-- Checkbox: Vertical Format (9:16) -->
      <label class="flex items-center gap-3.5 p-3.5 border border-[rgba(226,226,224,0.1)] rounded-[4px] cursor-pointer hover:border-[rgba(226,226,224,0.2)] bg-[#0c0c0e]/60 transition-colors select-none">
        <input 
          type="checkbox" 
          checked={$pipelineStore.config.vertical_format}
          on:change={handleVerticalChange}
          disabled={$pipelineStore.status === 'running'}
          class="w-4 h-4 rounded-[2px] bg-[#16161a] border-[rgba(226,226,224,0.3)] text-[#5865f2] focus:ring-0 focus:ring-offset-0 cursor-pointer accent-[#5865f2]"
        />
        <div>
          <div class="label-mono text-[#e2e2e4] normal-case tracking-normal text-xs font-medium">Вертикальный формат (9:16)</div>
          <div class="text-[11px] text-[rgba(226,226,224,0.5)] mt-0.5 font-sans">AI-фокусировка по лицам</div>
        </div>
      </label>

      <!-- Mode Selector -->
      <div class="flex flex-col gap-2">
        <div class="label-mono">Режим обработки</div>
        <div class="grid grid-cols-3 gap-2">
          <button 
            type="button"
            on:click={() => handleModeChange('auto')}
            class="font-mono py-2.5 px-2 border-none uppercase text-xs font-bold tracking-wider cursor-pointer transition-colors rounded-[2px] {
              $pipelineStore.config.mode === 'auto' 
                ? 'bg-[#5865f2] text-white' 
                : 'bg-[rgba(226,226,224,0.1)] text-[rgba(226,226,224,0.6)] hover:text-[#e2e2e4]'
            }"
          >
            Авто
          </button>
          <button 
            type="button"
            on:click={() => handleModeChange('confirm')}
            class="font-mono py-2.5 px-2 border-none uppercase text-xs font-bold tracking-wider cursor-pointer transition-colors rounded-[2px] {
              $pipelineStore.config.mode === 'confirm' 
                ? 'bg-[#5865f2] text-white' 
                : 'bg-[rgba(226,226,224,0.1)] text-[rgba(226,226,224,0.6)] hover:text-[#e2e2e4]'
            }"
          >
            Подтв.
          </button>
          <button 
            type="button"
            on:click={() => handleModeChange('manual')}
            class="font-mono py-2.5 px-2 border-none uppercase text-xs font-bold tracking-wider cursor-pointer transition-colors rounded-[2px] {
              $pipelineStore.config.mode === 'manual' 
                ? 'bg-[#5865f2] text-white' 
                : 'bg-[rgba(226,226,224,0.1)] text-[rgba(226,226,224,0.6)] hover:text-[#e2e2e4]'
            }"
          >
            Ручной
          </button>
        </div>
      </div>

    </div>

    <!-- Advanced Settings Drawer -->
    <div class="mt-4 pt-4 border-t border-[rgba(226,226,224,0.1)]">
      <button 
        type="button"
        on:click={toggleAdvanced}
        class="label-mono flex items-center justify-between w-full text-left cursor-pointer hover:text-[#e2e2e4] transition-colors"
      >
        <span>Расширенные настройки нейросетей</span>
        {#if showAdvanced}
          <ChevronUp class="h-3.5 w-3.5 text-[#5865f2]" />
        {:else}
          <ChevronDown class="h-3.5 w-3.5 text-[#5865f2]" />
        {/if}
      </button>

      {#if showAdvanced}
        <div class="mt-3.5 p-4 bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] rounded-[4px] flex flex-col gap-3">
          <div class="flex items-center justify-between">
            <span class="label-mono text-[#5865f2]">Менеджер весов</span>
            <button 
              type="button"
              on:click={onOpenModelManager}
              class="label-mono text-[#5865f2] hover:underline cursor-pointer"
            >
              [ОТКРЫТЬ МЕНЕДЖЕР МОДЕЛЕЙ]
            </button>
          </div>

          <!-- ASR model -->
          <div class="flex flex-col gap-1">
            <label for="asr_model" class="label-mono">Путь к модели Whisper</label>
            <input 
              id="asr_model"
              type="text" 
              value={$pipelineStore.config.asr_model_path}
              on:input={handleAsrModelChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            />
          </div>

          <!-- Vision model -->
          <div class="flex flex-col gap-1">
            <label for="vision_model" class="label-mono">Путь к модели Qwen-VL (llama)</label>
            <input 
              id="vision_model"
              type="text" 
              value={$pipelineStore.config.vision_model_path}
              on:input={handleVisionModelChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            />
          </div>

          <!-- NLP Text Engine -->
          <div class="flex flex-col gap-1">
            <label for="text_engine" class="label-mono">Движок текстового анализа</label>
            <select 
              id="text_engine"
              value={$pipelineStore.config.text_engine}
              on:change={handleTextEngineChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            >
              <option value="auto">Автоопределение</option>
              <option value="ollama">Ollama (Server)</option>
              <option value="llama">llama-cli (Local)</option>
            </select>
          </div>

          <!-- NLP Model -->
          <div class="flex flex-col gap-1">
            <label for="text_model" class="label-mono">Имя модели текста</label>
            <input 
              id="text_model"
              type="text" 
              value={$pipelineStore.config.text_model_name}
              on:input={handleTextModelChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            />
          </div>

          <!-- Output dir -->
          <div class="flex flex-col gap-1">
            <label for="output_dir" class="label-mono">Папка сохранения</label>
            <input 
              id="output_dir"
              type="text" 
              value={$pipelineStore.config.output_dir}
              on:input={handleOutputDirChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            />
          </div>
        </div>
      {/if}
    </div>
  </div>

  <!-- Action Buttons Stack (Variation 2 Design) -->
  <div class="mt-6 flex flex-col gap-3">
    <button 
      on:click={startProcess}
      disabled={!$pipelineStore.video || $pipelineStore.status === 'running' || $pipelineStore.config.mode === 'manual'}
      class="font-mono p-4 border-none uppercase text-xs font-bold tracking-[0.1em] cursor-pointer transition-opacity rounded-[2px] bg-[#5865f2] text-white disabled:opacity-30 disabled:cursor-not-allowed flex items-center justify-center gap-2"
    >
      {#if $pipelineStore.status === 'running'}
        <div class="h-3.5 w-3.5 border-2 border-white/20 border-t-white rounded-full animate-spin"></div>
        <span>Идет обработка...</span>
      {:else}
        <span>Запустить ИИ-анализ</span>
      {/if}
    </button>

    <button 
      on:click={exportClips}
      disabled={!$pipelineStore.video || $pipelineStore.status === 'running' || ($pipelineStore.config.mode === 'auto' && $pipelineStore.status !== 'completed')}
      class="font-mono p-4 border-none uppercase text-xs font-bold tracking-[0.1em] cursor-pointer transition-opacity rounded-[2px] bg-[#23c55e] text-white disabled:opacity-30 disabled:cursor-not-allowed flex items-center justify-center gap-2"
    >
      <span>Экспортировать клипы</span>
    </button>
  </div>
</div>
