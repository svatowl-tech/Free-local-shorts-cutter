<!-- src/lib/components/SettingsPanel.svelte -->
<script lang="ts">
  import { pipelineStore } from '../stores/pipeline';
  import { fragmentsStore } from '../stores/fragments';
  import { Settings, Sliders, Smartphone, Cpu, ShieldAlert, ChevronDown, ChevronUp, Play } from '@lucide/svelte';

  let showAdvanced = false;

  // Локальные методы мутации стора
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

<div class="bg-slate-900/60 border border-slate-800 rounded-2xl p-5 shadow-xl backdrop-blur-md flex flex-col gap-5">
  <div class="flex items-center justify-between border-b border-slate-850 pb-3">
    <div class="flex items-center gap-2">
      <Settings class="h-5 w-5 text-indigo-400" />
      <h2 class="text-sm font-bold tracking-widest text-slate-200 uppercase font-mono">Параметры обработки</h2>
    </div>
    <div class="px-2 py-0.5 bg-indigo-950/40 border border-indigo-900/30 rounded text-[10px] text-indigo-300 font-mono">
      tokio-engine v2.0
    </div>
  </div>

  <div class="space-y-4">
    <!-- Ползунок 1: Порог интенсивности AI -->
    <div class="space-y-1.5">
      <div class="flex justify-between items-center text-xs">
        <span class="text-slate-300 font-medium flex items-center gap-1">
          <Sliders class="h-3.5 w-3.5 text-indigo-400" />
          Порог интенсивности эмоций (AI)
        </span>
        <span class="text-indigo-400 font-bold font-mono bg-slate-950 px-2 py-0.5 rounded border border-slate-850">
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
        class="w-full h-1.5 bg-slate-950 rounded-lg appearance-none cursor-pointer accent-indigo-500 disabled:opacity-40"
      />
      <div class="flex justify-between text-[10px] text-slate-500 font-mono">
        <span>0.0 (Все подряд)</span>
        <span>Рекомендуется: 0.6</span>
        <span>1.0 (Только яркие пики)</span>
      </div>
    </div>

    <!-- Ползунок 2: Максимальная длительность клипа -->
    <div class="space-y-1.5">
      <div class="flex justify-between items-center text-xs">
        <span class="text-slate-300 font-medium flex items-center gap-1">
          <Smartphone class="h-3.5 w-3.5 text-indigo-400" />
          Макс. длительность клипа (с)
        </span>
        <span class="text-indigo-400 font-bold font-mono bg-slate-950 px-2 py-0.5 rounded border border-slate-850">
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
        class="w-full h-1.5 bg-slate-950 rounded-lg appearance-none cursor-pointer accent-indigo-500 disabled:opacity-40"
      />
      <div class="flex justify-between text-[10px] text-slate-500 font-mono">
        <span>5 с (Шортсы)</span>
        <span>До 60 сек</span>
      </div>
    </div>

    <!-- Чекбокс Вертикальный формат -->
    <label class="flex items-center gap-3 p-3 bg-slate-950/60 border border-slate-850 rounded-xl cursor-pointer hover:bg-slate-950 transition-colors select-none">
      <input 
        type="checkbox" 
        checked={$pipelineStore.config.vertical_format}
        on:change={handleVerticalChange}
        disabled={$pipelineStore.status === 'running'}
        class="hidden peer"
      />
      <div class="w-9 h-5 bg-slate-800 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-slate-400 peer-checked:after:bg-white after:border-slate-300 after:border after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-indigo-600 relative flex items-center">
      </div>
      <div class="flex flex-col">
        <span class="text-xs font-semibold text-slate-200">Вертикальный формат (9:16 Shorts)</span>
        <span class="text-[10px] text-slate-500">Автоматический кроп кадра AI-фокусировкой по лицам</span>
      </div>
    </label>
  </div>

  <!-- Расширенные настройки -->
  <div class="border-t border-slate-850/60 pt-2">
    <button 
      on:click={toggleAdvanced}
      class="flex items-center gap-1.5 text-xs text-slate-400 hover:text-slate-200 font-mono transition-colors focus:outline-none"
    >
      {#if showAdvanced}
        <ChevronUp class="h-3.5 w-3.5 text-indigo-400" />
        Скрыть расширенные настройки
      {:else}
        <ChevronDown class="h-3.5 w-3.5 text-indigo-400" />
        Показать расширенные настройки
      {/if}
    </button>

    {#if showAdvanced}
      <div class="space-y-3.5 mt-4 p-4 bg-slate-950/50 border border-slate-850/60 rounded-xl">
        <!-- Whisper ASR путь -->
        <div class="space-y-1">
          <label for="asr_model" class="text-[10px] font-bold font-mono text-slate-400 uppercase tracking-wider block">Путь к модели Whisper</label>
          <input 
            id="asr_model"
            type="text" 
            value={$pipelineStore.config.asr_model_path}
            on:input={handleAsrModelChange}
            disabled={$pipelineStore.status === 'running'}
            class="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs text-slate-300 font-mono placeholder-slate-700 focus:border-indigo-500 focus:outline-none"
          />
        </div>

        <!-- Qwen3-VL путь -->
        <div class="space-y-1">
          <label for="vision_model" class="text-[10px] font-bold font-mono text-slate-400 uppercase tracking-wider block">Путь к модели Qwen3-VL (llamа)</label>
          <input 
            id="vision_model"
            type="text" 
            value={$pipelineStore.config.vision_model_path}
            on:input={handleVisionModelChange}
            disabled={$pipelineStore.status === 'running'}
            class="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs text-slate-300 font-mono placeholder-slate-700 focus:border-indigo-500 focus:outline-none"
          />
        </div>

        <!-- NLP Text Engine -->
        <div class="space-y-1">
          <label for="text_engine" class="text-[10px] font-bold font-mono text-slate-400 uppercase tracking-wider block">Движок текстового анализа</label>
          <select 
            id="text_engine"
            value={$pipelineStore.config.text_engine}
            on:change={handleTextEngineChange}
            disabled={$pipelineStore.status === 'running'}
            class="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs text-slate-300 font-mono focus:border-indigo-500 focus:outline-none"
          >
            <option value="auto">Автоопределение</option>
            <option value="ollama">Ollama (Server)</option>
            <option value="llama">llama-cli (Local)</option>
          </select>
        </div>

        <!-- NLP Model path or name -->
        <div class="space-y-1">
          <label for="text_model" class="text-[10px] font-bold font-mono text-slate-400 uppercase tracking-wider block">Модель текста (имя или путь GGUF)</label>
          <input 
            id="text_model"
            type="text" 
            value={$pipelineStore.config.text_model_name}
            on:input={handleTextModelChange}
            disabled={$pipelineStore.status === 'running'}
            class="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs text-slate-300 font-mono placeholder-slate-700 focus:border-indigo-500 focus:outline-none"
          />
        </div>

        {#if $pipelineStore.config.text_engine === 'ollama' || $pipelineStore.config.text_engine === 'auto'}
        <div class="space-y-1">
          <label for="ollama_url" class="text-[10px] font-bold font-mono text-slate-400 uppercase tracking-wider block">URL Ollama API</label>
          <input 
            id="ollama_url"
            type="text" 
            value={$pipelineStore.config.ollama_url}
            on:input={handleOllamaUrlChange}
            disabled={$pipelineStore.status === 'running'}
            class="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs text-slate-300 font-mono placeholder-slate-700 focus:border-indigo-500 focus:outline-none"
          />
        </div>
        {/if}

        <!-- Директория экспорта -->
        <div class="space-y-1">
          <label for="output_dir" class="text-[10px] font-bold font-mono text-slate-400 uppercase tracking-wider block">Директория экспорта клипов</label>
          <input 
            id="output_dir"
            type="text" 
            value={$pipelineStore.config.output_dir}
            on:input={handleOutputDirChange}
            disabled={$pipelineStore.status === 'running'}
            class="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs text-slate-300 font-mono placeholder-slate-700 focus:border-indigo-500 focus:outline-none"
          />
        </div>
      </div>
    {/if}

    <!-- Режим работы -->
    <div class="space-y-2 mt-4 pt-4 border-t border-slate-800">
      <span class="text-[10px] font-bold font-mono text-slate-400 uppercase tracking-wider block">Режим обработки</span>
      <div class="grid grid-cols-3 gap-2">
        <label class="flex flex-col items-center justify-center p-3 rounded-xl border cursor-pointer transition-colors {$pipelineStore.config.mode === 'auto' ? 'bg-indigo-500/20 border-indigo-500/50 text-indigo-300' : 'bg-slate-900 border-slate-800 text-slate-400 hover:border-slate-700'}">
          <input type="radio" name="mode" value="auto" class="hidden" checked={$pipelineStore.config.mode === 'auto'} on:change={() => handleModeChange('auto')}>
          <span class="text-xs font-semibold">Автоматический</span>
        </label>
        <label class="flex flex-col items-center justify-center p-3 rounded-xl border cursor-pointer transition-colors {$pipelineStore.config.mode === 'confirm' ? 'bg-indigo-500/20 border-indigo-500/50 text-indigo-300' : 'bg-slate-900 border-slate-800 text-slate-400 hover:border-slate-700'}">
          <input type="radio" name="mode" value="confirm" class="hidden" checked={$pipelineStore.config.mode === 'confirm'} on:change={() => handleModeChange('confirm')}>
          <span class="text-xs font-semibold text-center leading-tight">С подтверждением</span>
        </label>
        <label class="flex flex-col items-center justify-center p-3 rounded-xl border cursor-pointer transition-colors {$pipelineStore.config.mode === 'manual' ? 'bg-indigo-500/20 border-indigo-500/50 text-indigo-300' : 'bg-slate-900 border-slate-800 text-slate-400 hover:border-slate-700'}">
          <input type="radio" name="mode" value="manual" class="hidden" checked={$pipelineStore.config.mode === 'manual'} on:change={() => handleModeChange('manual')}>
          <span class="text-xs font-semibold">Ручной</span>
        </label>
      </div>
    </div>
  </div>

  <div class="flex flex-col gap-2 mt-2">
    <!-- Кнопка запуска -->
    <button 
      on:click={startProcess}
      disabled={!$pipelineStore.video || $pipelineStore.status === 'running' || $pipelineStore.config.mode === 'manual'}
      class="w-full cursor-pointer bg-gradient-to-r from-indigo-600 to-indigo-700 hover:from-indigo-500 hover:to-indigo-600 active:from-indigo-700 disabled:from-slate-800 disabled:to-slate-850 disabled:cursor-not-allowed text-white font-semibold py-3 px-4 rounded-xl shadow-lg transition-all flex items-center justify-center gap-2"
    >
      {#if $pipelineStore.status === 'running'}
        <div class="h-4 w-4 border-2 border-white/20 border-t-white rounded-full animate-spin"></div>
        <span>ОБРАБОТКА...</span>
      {:else if $pipelineStore.config.mode === 'manual'}
        <Play class="h-4.5 w-4.5" />
        <span class="tracking-wide text-xs">ИИ-АНАЛИЗ ОТКЛЮЧЕН</span>
      {:else}
        <Play class="h-4.5 w-4.5" />
        <span class="tracking-wide text-xs">ЗАПУСТИТЬ ИИ-АНАЛИЗ</span>
      {/if}
    </button>

    <button 
      on:click={exportClips}
      disabled={!$pipelineStore.video || $pipelineStore.status === 'running' || ($pipelineStore.config.mode === 'auto' && $pipelineStore.status !== 'completed')}
      class="w-full cursor-pointer bg-emerald-600 hover:bg-emerald-500 active:bg-emerald-700 disabled:bg-slate-800 disabled:text-slate-600 disabled:cursor-not-allowed text-white font-semibold py-3 px-4 rounded-xl shadow-lg transition-all flex items-center justify-center gap-2"
    >
      <span class="tracking-wide text-xs">ЭКСПОРТИРОВАТЬ КЛИПЫ</span>
    </button>
  </div>
</div>
