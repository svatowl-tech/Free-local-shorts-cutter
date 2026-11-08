<!-- src/lib/components/BumperModal.svelte -->
<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { pipelineStore } from '../stores/pipeline';
  import { renderBumperPreview, type BumperGlobalConfig, type BumperPreset, type BumperItemConfig } from '../ipc';
  import { X, Upload, Download, Sparkles, Film, Music, Image as ImageIcon, Trash2, Plus, Check, RefreshCw, Play } from '@lucide/svelte';

  export let show = false;
  const dispatch = createEventDispatcher();

  let activeTab: 'v916' | 'v169' = 'v916';

  const createDefaultItem = (title: string, sub: string): BumperItemConfig => ({
    enabled: true,
    mode: 'generate',
    custom_video_path: null,
    custom_audio_path: null,
    title,
    subtitle: sub,
    duration_sec: 2.5,
    bg_image_path: null,
    logo_image_path: null,
    logo_scale: 0.5,
    logo_x_percent: 50,
    logo_y_percent: 40,
    title_color: '#FFFFFF',
    title_size: 32,
    title_x_percent: 50,
    title_y_percent: 75,
    transition: 'crossfade'
  });

  const createDefaultPreset = (id: string, name: string): BumperPreset => ({
    id,
    name,
    v916: {
      preroll: createDefaultItem('Аниме Студия', 'Преролл 9:16'),
      postroll: createDefaultItem('Подпишись на канал', 'Постролл 9:16')
    },
    v169: {
      preroll: createDefaultItem('Аниме Студия', 'Преролл 16:9'),
      postroll: createDefaultItem('Спасибо за просмотр', 'Постролл 16:9')
    },
    created_at: Date.now()
  });

  let config: BumperGlobalConfig = {
    enabled: true,
    active_preset_id: 'default',
    presets: [createDefaultPreset('default', 'Аниме Студия (По умолчанию)')]
  };

  let isRendering = false;
  let renderFeedback = '';
  let renderError = '';
  let newPresetName = '';
  let showNewPresetInput = false;

  let previewVideoUrl: { preroll: string | null; postroll: string | null } = {
    preroll: null,
    postroll: null
  };

  let isRenderingSingle: { preroll: boolean; postroll: boolean } = {
    preroll: false,
    postroll: false
  };

  $: if ($pipelineStore.config.bumper_config) {
    config = JSON.parse(JSON.stringify($pipelineStore.config.bumper_config));
  } else {
    config = {
      enabled: true,
      active_preset_id: 'default',
      presets: [createDefaultPreset('default', 'Аниме Студия (По умолчанию)')]
    };
  }

  $: activePreset = config.presets.find(p => p.id === config.active_preset_id) || config.presets[0] || createDefaultPreset('default', 'По умолчанию');
  $: category = activePreset[activeTab];

  function saveConfigToStore() {
    pipelineStore.updateConfig({ bumper_config: config });
  }

  function handleClose() {
    saveConfigToStore();
    dispatch('close');
  }

  function handleGlobalToggle(e: Event) {
    config.enabled = (e.target as HTMLInputElement).checked;
    saveConfigToStore();
  }

  function handleSelectPreset(id: string) {
    config.active_preset_id = id;
    previewVideoUrl = { preroll: null, postroll: null };
    saveConfigToStore();
  }

  function handleCreatePreset() {
    if (!newPresetName.trim()) return;
    const newId = `preset_${Date.now().toString(36)}`;
    const newP = createDefaultPreset(newId, newPresetName.trim());
    config.presets = [...config.presets, newP];
    config.active_preset_id = newId;
    newPresetName = '';
    showNewPresetInput = false;
    saveConfigToStore();
  }

  function handleDeletePreset(id: string) {
    if (config.presets.length <= 1) return;
    config.presets = config.presets.filter(p => p.id !== id);
    if (config.active_preset_id === id) {
      config.active_preset_id = config.presets[0].id;
    }
    saveConfigToStore();
  }

  async function handleRenderAndPlaySingle(bumperType: 'preroll' | 'postroll') {
    const item = category[bumperType];
    const isVertical = activeTab === 'v916';
    const sourceVideo = $pipelineStore.video?.path || '';

    isRenderingSingle[bumperType] = true;
    renderError = '';
    try {
      const videoPath = await renderBumperPreview(item, isVertical, sourceVideo, activePreset.id, bumperType);
      previewVideoUrl[bumperType] = videoPath;
    } catch (err: any) {
      renderError = err.message || 'Ошибка генерации заставки';
    } finally {
      isRenderingSingle[bumperType] = false;
    }
  }

  async function handlePreRenderAll() {
    if (isRendering) return;
    isRendering = true;
    renderFeedback = '';
    renderError = '';

    const sourceVideo = $pipelineStore.video?.path || '';

    try {
      const curPreset = activePreset;
      renderFeedback = '⚡ Отрендеринг заставок 9:16 (Преролл)...';
      if (curPreset.v916.preroll.enabled) {
        await renderBumperPreview(curPreset.v916.preroll, true, sourceVideo, curPreset.id, 'preroll');
      }

      renderFeedback = '⚡ Отрендеринг заставок 9:16 (Постролл)...';
      if (curPreset.v916.postroll.enabled) {
        await renderBumperPreview(curPreset.v916.postroll, true, sourceVideo, curPreset.id, 'postroll');
      }

      renderFeedback = '⚡ Отрендеринг заставок 16:9 (Преролл)...';
      if (curPreset.v169.preroll.enabled) {
        await renderBumperPreview(curPreset.v169.preroll, false, sourceVideo, curPreset.id, 'preroll');
      }

      renderFeedback = '⚡ Отрендеринг заставок 16:9 (Постролл)...';
      if (curPreset.v169.postroll.enabled) {
        await renderBumperPreview(curPreset.v169.postroll, false, sourceVideo, curPreset.id, 'postroll');
      }

      renderFeedback = '✓ Все заставки кэшированы! Экспорт роликов будет мгновенным.';
      setTimeout(() => {
        renderFeedback = '';
      }, 4000);
    } catch (err: any) {
      renderError = err.message || 'Ошибка рендеринга заставок';
    } finally {
      isRendering = false;
    }
  }

  function exportPresetToFile() {
    const dataStr = JSON.stringify(activePreset, null, 2);
    const blob = new Blob([dataStr], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `bumper_preset_${activePreset.name.replace(/[^a-z0-9а-я]/gi, '_')}.vcp-preset`;
    a.click();
    URL.revokeObjectURL(url);
  }

  function handleImportPresetFile(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (event) => {
      try {
        const parsed = JSON.parse(event.target?.result as string) as BumperPreset;
        if (parsed && parsed.v916 && parsed.v169) {
          const importedId = `preset_imp_${Date.now().toString(36)}`;
          parsed.id = importedId;
          parsed.name = `${parsed.name} (Импорт)`;
          config.presets = [...config.presets, parsed];
          config.active_preset_id = importedId;
          saveConfigToStore();
          renderFeedback = '✓ Пресет успешно импортирован!';
          setTimeout(() => (renderFeedback = ''), 3000);
        }
      } catch (err) {
        renderError = 'Некорректный файл пресета';
      }
    };
    reader.readAsText(file);
  }

  function triggerFileInput(id: string) {
    document.getElementById(id)?.click();
  }

  function handleFileUpload(e: Event, updateFn: (path: string) => void) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (ev) => {
      const url = ev.target?.result as string;
      updateFn((file as any).path || url);
      saveConfigToStore();
    };
    reader.readAsDataURL(file);
  }
</script>

{#if show}
  <!-- Backdrop -->
  <div class="fixed inset-0 bg-black/80 backdrop-blur-sm z-50 flex items-center justify-center p-4">
    <!-- Modal Shell -->
    <div class="bg-[#121215] border border-[rgba(226,226,224,0.15)] rounded-lg w-full max-w-5xl max-h-[90vh] flex flex-col shadow-2xl overflow-hidden">
      
      <!-- Modal Header -->
      <div class="p-5 border-b border-[rgba(226,226,224,0.1)] flex items-center justify-between bg-[#18181c]">
        <div class="flex items-center gap-3">
          <div class="w-8 h-8 rounded bg-[#5865f2]/20 border border-[#5865f2]/50 flex items-center justify-center text-[#5865f2]">
            <Film class="w-4 h-4" />
          </div>
          <div>
            <h2 class="text-base font-semibold text-white">Настройка Преролла и Постролла (Bumpers)</h2>
            <p class="text-xs text-slate-400">Интерактивный предпросмотр заставок в реальном времени</p>
          </div>
        </div>

        <div class="flex items-center gap-3">
          <label class="flex items-center gap-2 cursor-pointer select-none px-3 py-1.5 bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] rounded">
            <input 
              type="checkbox" 
              checked={config.enabled} 
              on:change={handleGlobalToggle} 
              class="w-4 h-4 text-[#5865f2] rounded accent-[#5865f2]" 
            />
            <span class="text-xs font-mono text-white">Включить бамперы</span>
          </label>

          <button 
            on:click={handleClose} 
            class="text-slate-400 hover:text-white p-1 rounded hover:bg-[#222228] transition-colors cursor-pointer"
          >
            <X class="w-5 h-5" />
          </button>
        </div>
      </div>

      <!-- Presets Toolbar -->
      <div class="px-5 py-3 bg-[#0c0c0e] border-b border-[rgba(226,226,224,0.1)] flex flex-wrap items-center justify-between gap-3 text-xs">
        <div class="flex items-center gap-3">
          <span class="font-mono text-slate-400">Глобальный пресет:</span>
          <select 
            value={config.active_preset_id} 
            on:change={(e) => handleSelectPreset(e.currentTarget.value)}
            class="bg-[#18181c] border border-[rgba(226,226,224,0.2)] text-white font-mono rounded px-3 py-1.5 focus:border-[#5865f2] outline-none cursor-pointer"
          >
            {#each config.presets as p}
              <option value={p.id}>{p.name}</option>
            {/each}
          </select>

          {#if showNewPresetInput}
            <div class="flex items-center gap-1.5">
              <input 
                type="text" 
                placeholder="Имя пресета..." 
                bind:value={newPresetName}
                class="bg-[#18181c] border border-[#5865f2] text-white px-2 py-1 rounded text-xs outline-none"
              />
              <button on:click={handleCreatePreset} class="p-1.5 bg-[#5865f2] text-white rounded hover:bg-[#4752c4] cursor-pointer">
                <Check class="w-3.5 h-3.5" />
              </button>
            </div>
          {:else}
            <button 
              on:click={() => (showNewPresetInput = true)} 
              class="flex items-center gap-1 px-2.5 py-1.5 bg-[#18181c] hover:bg-[#222228] border border-[rgba(226,226,224,0.15)] text-slate-300 rounded font-mono cursor-pointer"
            >
              <Plus class="w-3.5 h-3.5" />
              <span>Новый</span>
            </button>
          {/if}

          {#if config.presets.length > 1}
            <button 
              on:click={() => handleDeletePreset(config.active_preset_id)}
              class="p-1.5 text-red-400 hover:text-red-300 hover:bg-red-500/10 rounded cursor-pointer"
              title="Удалить пресет"
            >
              <Trash2 class="w-3.5 h-3.5" />
            </button>
          {/if}
        </div>

        <div class="flex items-center gap-2">
          <!-- Pre-render Button -->
          <button 
            on:click={handlePreRenderAll}
            disabled={isRendering}
            class="flex items-center gap-1.5 px-3 py-1.5 bg-[#5865f2] hover:bg-[#4752c4] text-white font-mono font-medium rounded transition-colors disabled:opacity-50 cursor-pointer"
          >
            {#if isRendering}
              <RefreshCw class="w-3.5 h-3.5 animate-spin" />
              <span>Рендеринг...</span>
            {:else}
              <Sparkles class="w-3.5 h-3.5" />
              <span>⚡ Отрендерить заставки</span>
            {/if}
          </button>

          <!-- Export Preset -->
          <button 
            on:click={exportPresetToFile}
            class="flex items-center gap-1 px-2.5 py-1.5 bg-[#18181c] hover:bg-[#222228] border border-[rgba(226,226,224,0.15)] text-slate-300 rounded font-mono cursor-pointer"
            title="Экспортировать выбранный пресет в файл .vcp-preset"
          >
            <Download class="w-3.5 h-3.5" />
            <span>Экспорт</span>
          </button>

          <!-- Import Preset -->
          <label class="flex items-center gap-1 px-2.5 py-1.5 bg-[#18181c] hover:bg-[#222228] border border-[rgba(226,226,224,0.15)] text-slate-300 rounded font-mono cursor-pointer">
            <Upload class="w-3.5 h-3.5" />
            <span>Импорт</span>
            <input type="file" accept=".vcp-preset,.json" on:change={handleImportPresetFile} class="hidden" />
          </label>
        </div>
      </div>

      {#if renderFeedback}
        <div class="px-5 py-2 bg-emerald-500/10 border-b border-emerald-500/20 text-emerald-400 text-xs font-mono">
          {renderFeedback}
        </div>
      {/if}
      {#if renderError}
        <div class="px-5 py-2 bg-red-500/10 border-b border-red-500/20 text-red-400 text-xs font-mono">
          {renderError}
        </div>
      {/if}

      <!-- Format Tabs (9:16 vs 16:9) -->
      <div class="flex border-b border-[rgba(226,226,224,0.1)] bg-[#16161a]">
        <button 
          on:click={() => { activeTab = 'v916'; previewVideoUrl = { preroll: null, postroll: null }; }}
          class="flex-1 py-3 text-xs font-mono font-medium text-center border-b-2 transition-colors cursor-pointer {
            activeTab === 'v916' 
              ? 'border-[#5865f2] text-[#5865f2] bg-[#121215]' 
              : 'border-transparent text-slate-400 hover:text-white'
          }"
        >
          📱 Шортсы / Reels / TikTok (9:16 Vertical)
        </button>

        <button 
          on:click={() => { activeTab = 'v169'; previewVideoUrl = { preroll: null, postroll: null }; }}
          class="flex-1 py-3 text-xs font-mono font-medium text-center border-b-2 transition-colors cursor-pointer {
            activeTab === 'v169' 
              ? 'border-[#5865f2] text-[#5865f2] bg-[#121215]' 
              : 'border-transparent text-slate-400 hover:text-white'
          }"
        >
          🎬 Видео / YouTube (16:9 Horizontal)
        </button>
      </div>

      <!-- Tab Content Area -->
      <div class="p-6 overflow-y-auto flex-1 space-y-6">
        <div class="grid grid-cols-1 lg:grid-cols-2 gap-6">
          
          <!-- Section 1: PREROLL -->
          <div class="bg-[#18181c] border border-[rgba(226,226,224,0.1)] rounded-lg p-5 flex flex-col gap-4">
            <div class="flex items-center justify-between pb-3 border-b border-[rgba(226,226,224,0.1)]">
              <div class="flex items-center gap-2">
                <span class="text-xs font-mono font-bold uppercase tracking-wider text-[#5865f2]">
                  ◀ Преролл ({activeTab === 'v916' ? '9:16' : '16:9'})
                </span>
              </div>
              <label class="flex items-center gap-2 cursor-pointer select-none">
                <input 
                  type="checkbox" 
                  bind:checked={category.preroll.enabled} 
                  on:change={saveConfigToStore}
                  class="w-4 h-4 text-[#5865f2] rounded accent-[#5865f2]" 
                />
                <span class="text-xs font-mono text-slate-300">Включить</span>
              </label>
            </div>

            {#if category.preroll.enabled}
              <!-- LIVE INTERACTIVE VISUAL PREVIEW BOX (PREROLL) -->
              <div class="relative bg-black rounded-lg overflow-hidden border border-[rgba(226,226,224,0.2)] flex items-center justify-center shadow-inner group">
                <div class="relative w-full overflow-hidden flex items-center justify-center bg-gradient-to-br from-slate-950 via-indigo-950/70 to-slate-950 {activeTab === 'v916' ? 'aspect-[9/16] max-h-[260px]' : 'aspect-[16/9] max-h-[220px]'}">
                  
                  {#if previewVideoUrl.preroll}
                    <video 
                      src={previewVideoUrl.preroll} 
                      controls 
                      autoplay 
                      class="w-full h-full object-contain bg-black"
                    >
                      <track kind="captions" />
                    </video>
                  {:else if category.preroll.mode === 'custom_video' && category.preroll.custom_video_path}
                    <video 
                      src={category.preroll.custom_video_path} 
                      controls 
                      class="w-full h-full object-contain bg-black"
                    >
                      <track kind="captions" />
                    </video>
                  {:else}
                    <!-- Simulated Background -->
                    {#if category.preroll.bg_image_path}
                      <img src={category.preroll.bg_image_path} alt="Background" class="absolute inset-0 w-full h-full object-cover" />
                    {:else}
                      <div class="absolute inset-0 bg-[#121218] opacity-90 blur-md scale-110"></div>
                      <div class="absolute inset-0 bg-gradient-to-b from-black/40 via-transparent to-black/60"></div>
                    {/if}

                    <!-- Logo Overlay -->
                    {#if category.preroll.logo_image_path}
                      <div 
                        class="absolute left-1/2 transition-all duration-75 pointer-events-none"
                        style="top: {category.preroll.logo_y_percent}%; transform: translate(-50%, -50%) scale({category.preroll.logo_scale});"
                      >
                        <img src={category.preroll.logo_image_path} alt="Logo" class="max-w-[120px] max-h-[120px] object-contain drop-shadow-lg" />
                      </div>
                    {/if}

                    <!-- Title & Subtitle Box -->
                    {#if category.preroll.title || category.preroll.subtitle}
                      <div 
                        class="absolute left-1/2 -translate-x-1/2 -translate-y-1/2 text-center transition-all duration-75 max-w-[90%] px-3 py-1.5 rounded bg-black/50 backdrop-blur-md border border-white/10 shadow-lg pointer-events-none"
                        style="top: {category.preroll.title_y_percent}%;"
                      >
                        {#if category.preroll.title}
                          <div 
                            class="font-bold leading-tight drop-shadow" 
                            style="color: {category.preroll.title_color}; font-size: {Math.round(category.preroll.title_size * (activeTab === 'v916' ? 0.45 : 0.38))}px;"
                          >
                            {category.preroll.title}
                          </div>
                        {/if}
                        {#if category.preroll.subtitle}
                          <div class="text-[11px] text-slate-300 font-mono mt-0.5 opacity-90">
                            {category.preroll.subtitle}
                          </div>
                        {/if}
                      </div>
                    {/if}

                    <!-- Live Watermark -->
                    <div class="absolute top-2 left-2 px-2 py-0.5 bg-black/60 backdrop-blur text-[10px] font-mono text-slate-300 rounded border border-white/10 flex items-center gap-1 pointer-events-none select-none">
                      <span class="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse"></span>
                      <span>Интерактивное превью</span>
                    </div>
                  {/if}
                </div>

                <!-- Test Video Button -->
                <div class="absolute bottom-2 right-2 z-10">
                  <button 
                    on:click={() => handleRenderAndPlaySingle('preroll')}
                    disabled={isRenderingSingle.preroll}
                    class="px-2.5 py-1 bg-[#5865f2] hover:bg-[#4752c4] text-white font-mono text-[11px] font-semibold rounded shadow-md flex items-center gap-1.5 transition-colors disabled:opacity-50 cursor-pointer"
                    title="Сгенерировать и воспроизвести готовое видео с аудиофайлом"
                  >
                    {#if isRenderingSingle.preroll}
                      <RefreshCw class="w-3 h-3 animate-spin" />
                      <span>Рендер...</span>
                    {:else}
                      <Play class="w-3 h-3 fill-current" />
                      <span>{previewVideoUrl.preroll ? 'Перерендерить' : 'Тест видео'}</span>
                    {/if}
                  </button>
                </div>
              </div>

              <!-- Mode Selection -->
              <div class="flex gap-2 p-1 bg-[#0c0c0e] rounded border border-[rgba(226,226,224,0.1)]">
                <button 
                  on:click={() => { category.preroll.mode = 'generate'; saveConfigToStore(); }}
                  class="flex-1 py-1.5 text-xs font-mono rounded transition-colors cursor-pointer {
                    category.preroll.mode === 'generate' ? 'bg-[#5865f2] text-white font-medium' : 'text-slate-400 hover:text-white'
                  }"
                >
                  Генератор заставки
                </button>
                <button 
                  on:click={() => { category.preroll.mode = 'custom_video'; saveConfigToStore(); }}
                  class="flex-1 py-1.5 text-xs font-mono rounded transition-colors cursor-pointer {
                    category.preroll.mode === 'custom_video' ? 'bg-[#5865f2] text-white font-medium' : 'text-slate-400 hover:text-white'
                  }"
                >
                  Кастомное видео
                </button>
              </div>

              {#if category.preroll.mode === 'custom_video'}
                <!-- Custom Video Upload -->
                <div class="flex flex-col gap-2">
                  <span class="text-xs font-mono text-slate-300">Загрузить видеоролик (.mp4, .mov, .webm)</span>
                  <div class="flex gap-2">
                    <input 
                      type="text" 
                      placeholder="Путь к видеофайлу..." 
                      bind:value={category.preroll.custom_video_path}
                      on:change={saveConfigToStore}
                      class="flex-1 bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-xs font-mono text-white px-3 py-2 rounded focus:border-[#5865f2] outline-none"
                    />
                    <button 
                      on:click={() => triggerFileInput('file_pre_vid_' + activeTab)}
                      class="px-3 py-2 bg-[#222228] hover:bg-[#2c2c34] text-xs font-mono text-white rounded border border-[rgba(226,226,224,0.15)] cursor-pointer"
                    >
                      Обзор
                    </button>
                    <input 
                      id={'file_pre_vid_' + activeTab} 
                      type="file" 
                      accept="video/*" 
                      on:change={(e) => handleFileUpload(e, (p) => (category.preroll.custom_video_path = p))} 
                      class="hidden" 
                    />
                  </div>
                </div>
              {:else}
                <!-- In-App Generator Controls -->
                <div class="space-y-3 text-xs">
                  <!-- Title & Subtitle -->
                  <div class="grid grid-cols-2 gap-2">
                    <div>
                      <span class="text-slate-400 font-mono block mb-1">Заголовок</span>
                      <input 
                        type="text" 
                        bind:value={category.preroll.title}
                        on:input={saveConfigToStore}
                        class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono"
                      />
                    </div>
                    <div>
                      <span class="text-slate-400 font-mono block mb-1">Подзаголовок</span>
                      <input 
                        type="text" 
                        bind:value={category.preroll.subtitle}
                        on:input={saveConfigToStore}
                        class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono"
                      />
                    </div>
                  </div>

                  <!-- Text Styling & Position -->
                  <div class="grid grid-cols-3 gap-2">
                    <div>
                      <span class="text-slate-400 font-mono block mb-1">Цвет текста</span>
                      <input 
                        type="color" 
                        bind:value={category.preroll.title_color}
                        on:change={saveConfigToStore}
                        class="w-full h-8 bg-transparent cursor-pointer rounded"
                      />
                    </div>
                    <div>
                      <span class="text-slate-400 font-mono block mb-1">Размер (px)</span>
                      <input 
                        type="number" 
                        min="16" max="72"
                        bind:value={category.preroll.title_size}
                        on:change={saveConfigToStore}
                        class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2 py-1.5 rounded font-mono"
                      />
                    </div>
                    <div>
                      <span class="text-slate-400 font-mono block mb-1">Позиция Y (%)</span>
                      <input 
                        type="range" 
                        min="10" max="90"
                        bind:value={category.preroll.title_y_percent}
                        on:input={saveConfigToStore}
                        class="w-full accent-[#5865f2]"
                      />
                    </div>
                  </div>

                  <!-- Custom Audio Track -->
                  <div>
                    <div class="flex justify-between items-center mb-1">
                      <span class="text-slate-400 font-mono flex items-center gap-1">
                        <Music class="w-3.5 h-3.5 text-[#5865f2]" />
                        <span>Свой звук (.mp3, .wav)</span>
                      </span>
                      {#if category.preroll.custom_audio_path}
                        <span class="text-[10px] text-emerald-400 font-mono">✓ Авто-длительность по аудио</span>
                      {/if}
                    </div>
                    <div class="flex gap-2">
                      <input 
                        type="text" 
                        placeholder="Путь к аудиофайлу..." 
                        bind:value={category.preroll.custom_audio_path}
                        on:change={saveConfigToStore}
                        class="flex-1 bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono text-[11px]"
                      />
                      <button 
                        on:click={() => triggerFileInput('file_pre_aud_' + activeTab)}
                        class="px-2.5 py-1.5 bg-[#222228] text-white rounded font-mono text-[11px] border border-[rgba(226,226,224,0.15)] cursor-pointer"
                      >
                        Выбрать
                      </button>
                      <input 
                        id={'file_pre_aud_' + activeTab} 
                        type="file" 
                        accept="audio/*" 
                        on:change={(e) => handleFileUpload(e, (p) => (category.preroll.custom_audio_path = p))} 
                        class="hidden" 
                      />
                    </div>
                  </div>

                  <!-- Logo Overlay Upload -->
                  <div>
                    <span class="text-slate-400 font-mono flex items-center gap-1 mb-1">
                      <ImageIcon class="w-3.5 h-3.5 text-[#5865f2]" />
                      <span>Накладываемое лого/картинка (PNG/GIF/JPG)</span>
                    </span>
                    <div class="flex gap-2 mb-2">
                      <input 
                        type="text" 
                        placeholder="Путь к картинке..." 
                        bind:value={category.preroll.logo_image_path}
                        on:change={saveConfigToStore}
                        class="flex-1 bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono text-[11px]"
                      />
                      <button 
                        on:click={() => triggerFileInput('file_pre_logo_' + activeTab)}
                        class="px-2.5 py-1.5 bg-[#222228] text-white rounded font-mono text-[11px] border border-[rgba(226,226,224,0.15)] cursor-pointer"
                      >
                        Загрузить
                      </button>
                      <input 
                        id={'file_pre_logo_' + activeTab} 
                        type="file" 
                        accept="image/*" 
                        on:change={(e) => handleFileUpload(e, (p) => (category.preroll.logo_image_path = p))} 
                        class="hidden" 
                      />
                    </div>
                    {#if category.preroll.logo_image_path}
                      <div class="grid grid-cols-2 gap-2 p-2 bg-[#0c0c0e] rounded">
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono">Масштаб лого</span>
                          <input type="range" min="0.1" max="1.2" step="0.05" bind:value={category.preroll.logo_scale} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono">Позиция Y</span>
                          <input type="range" min="10" max="90" bind:value={category.preroll.logo_y_percent} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                      </div>
                    {/if}
                  </div>
                </div>
              {/if}
            {/if}
          </div>

          <!-- Section 2: POSTROLL -->
          <div class="bg-[#18181c] border border-[rgba(226,226,224,0.1)] rounded-lg p-5 flex flex-col gap-4">
            <div class="flex items-center justify-between pb-3 border-b border-[rgba(226,226,224,0.1)]">
              <div class="flex items-center gap-2">
                <span class="text-xs font-mono font-bold uppercase tracking-wider text-[#5865f2]">
                  ▶ Постролл ({activeTab === 'v916' ? '9:16' : '16:9'})
                </span>
              </div>
              <label class="flex items-center gap-2 cursor-pointer select-none">
                <input 
                  type="checkbox" 
                  bind:checked={category.postroll.enabled} 
                  on:change={saveConfigToStore}
                  class="w-4 h-4 text-[#5865f2] rounded accent-[#5865f2]" 
                />
                <span class="text-xs font-mono text-slate-300">Включить</span>
              </label>
            </div>

            {#if category.postroll.enabled}
              <!-- LIVE INTERACTIVE VISUAL PREVIEW BOX (POSTROLL) -->
              <div class="relative bg-black rounded-lg overflow-hidden border border-[rgba(226,226,224,0.2)] flex items-center justify-center shadow-inner group">
                <div class="relative w-full overflow-hidden flex items-center justify-center bg-gradient-to-br from-slate-950 via-indigo-950/70 to-slate-950 {activeTab === 'v916' ? 'aspect-[9/16] max-h-[260px]' : 'aspect-[16/9] max-h-[220px]'}">
                  
                  {#if previewVideoUrl.postroll}
                    <video 
                      src={previewVideoUrl.postroll} 
                      controls 
                      autoplay 
                      class="w-full h-full object-contain bg-black"
                    >
                      <track kind="captions" />
                    </video>
                  {:else if category.postroll.mode === 'custom_video' && category.postroll.custom_video_path}
                    <video 
                      src={category.postroll.custom_video_path} 
                      controls 
                      class="w-full h-full object-contain bg-black"
                    >
                      <track kind="captions" />
                    </video>
                  {:else}
                    <!-- Simulated Background -->
                    {#if category.postroll.bg_image_path}
                      <img src={category.postroll.bg_image_path} alt="Background" class="absolute inset-0 w-full h-full object-cover" />
                    {:else}
                      <div class="absolute inset-0 bg-[#121218] opacity-90 blur-md scale-110"></div>
                      <div class="absolute inset-0 bg-gradient-to-b from-black/40 via-transparent to-black/60"></div>
                    {/if}

                    <!-- Logo Overlay -->
                    {#if category.postroll.logo_image_path}
                      <div 
                        class="absolute left-1/2 transition-all duration-75 pointer-events-none"
                        style="top: {category.postroll.logo_y_percent}%; transform: translate(-50%, -50%) scale({category.postroll.logo_scale});"
                      >
                        <img src={category.postroll.logo_image_path} alt="Logo" class="max-w-[120px] max-h-[120px] object-contain drop-shadow-lg" />
                      </div>
                    {/if}

                    <!-- Title & Subtitle Box -->
                    {#if category.postroll.title || category.postroll.subtitle}
                      <div 
                        class="absolute left-1/2 -translate-x-1/2 -translate-y-1/2 text-center transition-all duration-75 max-w-[90%] px-3 py-1.5 rounded bg-black/50 backdrop-blur-md border border-white/10 shadow-lg pointer-events-none"
                        style="top: {category.postroll.title_y_percent}%;"
                      >
                        {#if category.postroll.title}
                          <div 
                            class="font-bold leading-tight drop-shadow" 
                            style="color: {category.postroll.title_color}; font-size: {Math.round(category.postroll.title_size * (activeTab === 'v916' ? 0.45 : 0.38))}px;"
                          >
                            {category.postroll.title}
                          </div>
                        {/if}
                        {#if category.postroll.subtitle}
                          <div class="text-[11px] text-slate-300 font-mono mt-0.5 opacity-90">
                            {category.postroll.subtitle}
                          </div>
                        {/if}
                      </div>
                    {/if}

                    <!-- Live Watermark -->
                    <div class="absolute top-2 left-2 px-2 py-0.5 bg-black/60 backdrop-blur text-[10px] font-mono text-slate-300 rounded border border-white/10 flex items-center gap-1 pointer-events-none select-none">
                      <span class="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse"></span>
                      <span>Интерактивное превью</span>
                    </div>
                  {/if}
                </div>

                <!-- Test Video Button -->
                <div class="absolute bottom-2 right-2 z-10">
                  <button 
                    on:click={() => handleRenderAndPlaySingle('postroll')}
                    disabled={isRenderingSingle.postroll}
                    class="px-2.5 py-1 bg-[#5865f2] hover:bg-[#4752c4] text-white font-mono text-[11px] font-semibold rounded shadow-md flex items-center gap-1.5 transition-colors disabled:opacity-50 cursor-pointer"
                    title="Сгенерировать и воспроизвести готовое видео с аудиофайлом"
                  >
                    {#if isRenderingSingle.postroll}
                      <RefreshCw class="w-3 h-3 animate-spin" />
                      <span>Рендер...</span>
                    {:else}
                      <Play class="w-3 h-3 fill-current" />
                      <span>{previewVideoUrl.postroll ? 'Перерендерить' : 'Тест видео'}</span>
                    {/if}
                  </button>
                </div>
              </div>

              <!-- Mode Selection -->
              <div class="flex gap-2 p-1 bg-[#0c0c0e] rounded border border-[rgba(226,226,224,0.1)]">
                <button 
                  on:click={() => { category.postroll.mode = 'generate'; saveConfigToStore(); }}
                  class="flex-1 py-1.5 text-xs font-mono rounded transition-colors cursor-pointer {
                    category.postroll.mode === 'generate' ? 'bg-[#5865f2] text-white font-medium' : 'text-slate-400 hover:text-white'
                  }"
                >
                  Генератор заставки
                </button>
                <button 
                  on:click={() => { category.postroll.mode = 'custom_video'; saveConfigToStore(); }}
                  class="flex-1 py-1.5 text-xs font-mono rounded transition-colors cursor-pointer {
                    category.postroll.mode === 'custom_video' ? 'bg-[#5865f2] text-white font-medium' : 'text-slate-400 hover:text-white'
                  }"
                >
                  Кастомное видео
                </button>
              </div>

              {#if category.postroll.mode === 'custom_video'}
                <!-- Custom Video Upload -->
                <div class="flex flex-col gap-2">
                  <span class="text-xs font-mono text-slate-300">Загрузить видеоролик (.mp4, .mov, .webm)</span>
                  <div class="flex gap-2">
                    <input 
                      type="text" 
                      placeholder="Путь к видеофайлу..." 
                      bind:value={category.postroll.custom_video_path}
                      on:change={saveConfigToStore}
                      class="flex-1 bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-xs font-mono text-white px-3 py-2 rounded focus:border-[#5865f2] outline-none"
                    />
                    <button 
                      on:click={() => triggerFileInput('file_post_vid_' + activeTab)}
                      class="px-3 py-2 bg-[#222228] hover:bg-[#2c2c34] text-xs font-mono text-white rounded border border-[rgba(226,226,224,0.15)] cursor-pointer"
                    >
                      Обзор
                    </button>
                    <input 
                      id={'file_post_vid_' + activeTab} 
                      type="file" 
                      accept="video/*" 
                      on:change={(e) => handleFileUpload(e, (p) => (category.postroll.custom_video_path = p))} 
                      class="hidden" 
                    />
                  </div>
                </div>
              {:else}
                <!-- In-App Generator Controls -->
                <div class="space-y-3 text-xs">
                  <!-- Title & Subtitle -->
                  <div class="grid grid-cols-2 gap-2">
                    <div>
                      <span class="text-slate-400 font-mono block mb-1">Заголовок</span>
                      <input 
                        type="text" 
                        bind:value={category.postroll.title}
                        on:input={saveConfigToStore}
                        class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono"
                      />
                    </div>
                    <div>
                      <span class="text-slate-400 font-mono block mb-1">Подзаголовок</span>
                      <input 
                        type="text" 
                        bind:value={category.postroll.subtitle}
                        on:input={saveConfigToStore}
                        class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono"
                      />
                    </div>
                  </div>

                  <!-- Text Styling & Position -->
                  <div class="grid grid-cols-3 gap-2">
                    <div>
                      <span class="text-slate-400 font-mono block mb-1">Цвет текста</span>
                      <input 
                        type="color" 
                        bind:value={category.postroll.title_color}
                        on:change={saveConfigToStore}
                        class="w-full h-8 bg-transparent cursor-pointer rounded"
                      />
                    </div>
                    <div>
                      <span class="text-slate-400 font-mono block mb-1">Размер (px)</span>
                      <input 
                        type="number" 
                        min="16" max="72"
                        bind:value={category.postroll.title_size}
                        on:change={saveConfigToStore}
                        class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2 py-1.5 rounded font-mono"
                      />
                    </div>
                    <div>
                      <span class="text-slate-400 font-mono block mb-1">Позиция Y (%)</span>
                      <input 
                        type="range" 
                        min="10" max="90"
                        bind:value={category.postroll.title_y_percent}
                        on:input={saveConfigToStore}
                        class="w-full accent-[#5865f2]"
                      />
                    </div>
                  </div>

                  <!-- Custom Audio Track -->
                  <div>
                    <div class="flex justify-between items-center mb-1">
                      <span class="text-slate-400 font-mono flex items-center gap-1">
                        <Music class="w-3.5 h-3.5 text-[#5865f2]" />
                        <span>Свой звук (.mp3, .wav)</span>
                      </span>
                      {#if category.postroll.custom_audio_path}
                        <span class="text-[10px] text-emerald-400 font-mono">✓ Авто-длительность по аудио</span>
                      {/if}
                    </div>
                    <div class="flex gap-2">
                      <input 
                        type="text" 
                        placeholder="Путь к аудиофайлу..." 
                        bind:value={category.postroll.custom_audio_path}
                        on:change={saveConfigToStore}
                        class="flex-1 bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono text-[11px]"
                      />
                      <button 
                        on:click={() => triggerFileInput('file_post_aud_' + activeTab)}
                        class="px-2.5 py-1.5 bg-[#222228] text-white rounded font-mono text-[11px] border border-[rgba(226,226,224,0.15)] cursor-pointer"
                      >
                        Выбрать
                      </button>
                      <input 
                        id={'file_post_aud_' + activeTab} 
                        type="file" 
                        accept="audio/*" 
                        on:change={(e) => handleFileUpload(e, (p) => (category.postroll.custom_audio_path = p))} 
                        class="hidden" 
                      />
                    </div>
                  </div>

                  <!-- Logo Overlay Upload -->
                  <div>
                    <span class="text-slate-400 font-mono flex items-center gap-1 mb-1">
                      <ImageIcon class="w-3.5 h-3.5 text-[#5865f2]" />
                      <span>Накладываемое лого/картинка (PNG/GIF/JPG)</span>
                    </span>
                    <div class="flex gap-2 mb-2">
                      <input 
                        type="text" 
                        placeholder="Путь к картинке..." 
                        bind:value={category.postroll.logo_image_path}
                        on:change={saveConfigToStore}
                        class="flex-1 bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono text-[11px]"
                      />
                      <button 
                        on:click={() => triggerFileInput('file_post_logo_' + activeTab)}
                        class="px-2.5 py-1.5 bg-[#222228] text-white rounded font-mono text-[11px] border border-[rgba(226,226,224,0.15)] cursor-pointer"
                      >
                        Загрузить
                      </button>
                      <input 
                        id={'file_post_logo_' + activeTab} 
                        type="file" 
                        accept="image/*" 
                        on:change={(e) => handleFileUpload(e, (p) => (category.postroll.logo_image_path = p))} 
                        class="hidden" 
                      />
                    </div>
                    {#if category.postroll.logo_image_path}
                      <div class="grid grid-cols-2 gap-2 p-2 bg-[#0c0c0e] rounded">
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono">Масштаб лого</span>
                          <input type="range" min="0.1" max="1.2" step="0.05" bind:value={category.postroll.logo_scale} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono">Позиция Y</span>
                          <input type="range" min="10" max="90" bind:value={category.postroll.logo_y_percent} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                      </div>
                    {/if}
                  </div>
                </div>
              {/if}
            {/if}
          </div>

        </div>
      </div>

      <!-- Modal Footer -->
      <div class="p-4 bg-[#18181c] border-t border-[rgba(226,226,224,0.1)] flex justify-end gap-3">
        <button 
          on:click={handleClose}
          class="px-5 py-2 bg-[#5865f2] hover:bg-[#4752c4] text-white font-mono text-xs font-semibold uppercase tracking-wider rounded transition-colors cursor-pointer"
        >
          Готово
        </button>
      </div>

    </div>
  </div>
{/if}
