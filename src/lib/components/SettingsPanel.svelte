<!-- src/lib/components/SettingsPanel.svelte -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { pipelineStore } from '../stores/pipeline';
  import { fragmentsStore, type Fragment } from '../stores/fragments';
  import { modelsStore } from '../stores/models';
  import { getOpenAiProfiles, saveOpenAiProfiles, fetchOpenAiModels, type OpenAiProfile } from '../ipc';
  import { isTauriRuntime, joinRelativePath, formatErrorMessage } from '../utils/path';
  import { ChevronDown, ChevronUp, Settings } from '@lucide/svelte';
  import BumperModal from './BumperModal.svelte';

  export let onOpenModelManager: () => void = () => {};

  let showAdvanced = false;
  let showBumperModal = false;

  let savedOpenAiProfiles: OpenAiProfile[] = [];
  let selectedProfileId = '';
  let profileName = 'OpenAI';
  let openaiBaseUrl = 'https://api.openai.com/v1';
  let openaiApiKey = '';
  let openaiModel = 'gpt-4o-mini';
  let showApiKey = false;
  let saveFeedback = '';
  let fetchedModels: string[] = [];
  let modelSearchQuery = '';
  let isFetchingModels = false;
  let fetchModelsError = '';
  let fetchModelsSuccess = '';

  $: filteredFetchedModels = modelSearchQuery.trim()
    ? fetchedModels.filter(m => m.toLowerCase().includes(modelSearchQuery.trim().toLowerCase()))
    : fetchedModels;

  async function initOpenAiProfiles() {
    if (isTauriRuntime()) {
      const diskList = await getOpenAiProfiles();
      if (diskList && diskList.length > 0) {
        savedOpenAiProfiles = diskList;
      }
    }

    const cfg = $pipelineStore.config;
    if (cfg.openai_base_url) openaiBaseUrl = cfg.openai_base_url;
    if (cfg.openai_api_key) openaiApiKey = cfg.openai_api_key;
    if (cfg.openai_model) openaiModel = cfg.openai_model;

    if (cfg.openai_profile_id && savedOpenAiProfiles.some(p => p.id === cfg.openai_profile_id)) {
      selectOpenAiProfile(cfg.openai_profile_id, true);
    } else if (savedOpenAiProfiles.length > 0) {
      const matched = savedOpenAiProfiles.find(
        p => p.base_url === openaiBaseUrl && p.api_key === openaiApiKey
      ) || savedOpenAiProfiles[0];
      selectOpenAiProfile(matched.id, true);
    }
  }

  async function persistProfilesList(list: OpenAiProfile[]) {
    savedOpenAiProfiles = list;
    if (isTauriRuntime()) {
      try {
        await saveOpenAiProfiles(list);
      } catch (err: unknown) {
        const msg = formatErrorMessage(err);
        console.warn('Ошибка сохранения профилей OpenAI на диск:', msg);
        fetchModelsError = `Ошибка сохранения профиля: ${msg}`;
      }
    }
  }

  function selectOpenAiProfile(id: string, logChange = true) {
    selectedProfileId = id;
    fetchModelsError = '';
    fetchModelsSuccess = '';
    modelSearchQuery = '';
    if (id === '__new__') {
      profileName = 'Новый API-ключ';
      openaiBaseUrl = 'https://api.openai.com/v1';
      openaiApiKey = '';
      openaiModel = 'gpt-4o-mini';
      fetchedModels = [];
      if (logChange) {
        pipelineStore.updateConfig({
          openai_profile_id: '',
          openai_base_url: openaiBaseUrl,
          openai_api_key: openaiApiKey,
          openai_model: openaiModel
        });
      }
      return;
    }
    const found = savedOpenAiProfiles.find(p => p.id === id);
    if (found) {
      profileName = found.name;
      openaiBaseUrl = found.base_url;
      openaiApiKey = found.api_key;
      openaiModel = found.model;
      fetchedModels = found.available_models || [];
      if (logChange) {
        pipelineStore.updateConfig({
          openai_profile_id: found.id,
          openai_base_url: found.base_url,
          openai_api_key: found.api_key,
          openai_model: found.model
        });
      }
    }
  }

  function handleProfileSelectChange(e: Event) {
    const val = (e.target as HTMLSelectElement).value;
    selectOpenAiProfile(val, true);
  }

  function syncOpenAiFieldsToStore() {
    pipelineStore.updateConfig({
      openai_base_url: openaiBaseUrl.trim() || 'https://api.openai.com/v1',
      openai_api_key: openaiApiKey.trim(),
      openai_model: openaiModel.trim() || 'gpt-4o-mini',
      openai_profile_id: selectedProfileId && selectedProfileId !== '__new__' ? selectedProfileId : ''
    });
  }

  async function handleFetchModelsFromServer() {
    if (isFetchingModels) return;
    isFetchingModels = true;
    fetchModelsError = '';
    fetchModelsSuccess = '';

    try {
      const cleanUrl = openaiBaseUrl.trim() || 'https://api.openai.com/v1';
      const cleanKey = openaiApiKey.trim();
      const models = await fetchOpenAiModels(cleanUrl, cleanKey);
      fetchedModels = models;
      modelSearchQuery = '';

      if (models.length > 0) {
        if (!openaiModel.trim() || !models.includes(openaiModel.trim())) {
          openaiModel = models[0];
          syncOpenAiFieldsToStore();
        }
        fetchModelsSuccess = `✓ Загружено моделей: ${models.length}`;

        // Если выбран сохраненный профиль — сразу сохраняем список моделей в него
        if (selectedProfileId && selectedProfileId !== '__new__') {
          const idx = savedOpenAiProfiles.findIndex(p => p.id === selectedProfileId);
          if (idx >= 0) {
            const nextList = [...savedOpenAiProfiles];
            nextList[idx] = {
              ...nextList[idx],
              model: openaiModel,
              available_models: models
            };
            await persistProfilesList(nextList);
          }
        }
      } else {
        fetchModelsError = 'Сервер не вернул ни одной модели';
      }
    } catch (err: unknown) {
      const msg = formatErrorMessage(err);
      fetchModelsError = msg.replace(/^Error:\s*/i, '');
    } finally {
      isFetchingModels = false;
    }
  }

  function handleSelectFetchedModel(e: Event) {
    const val = (e.target as HTMLSelectElement).value;
    if (val) {
      openaiModel = val;
      syncOpenAiFieldsToStore();
    }
  }

  async function handleSaveOpenAiProfile() {
    const cleanUrl = openaiBaseUrl.trim() || 'https://api.openai.com/v1';
    const cleanKey = openaiApiKey.trim();
    const cleanModel = openaiModel.trim() || 'gpt-4o-mini';
    const cleanName = profileName.trim() || `API (${cleanModel})`;

    let targetId = selectedProfileId && selectedProfileId !== '__new__'
      ? selectedProfileId
      : `oai_${Date.now().toString(36)}`;

    const updatedProfile: OpenAiProfile = {
      id: targetId,
      name: cleanName,
      base_url: cleanUrl,
      api_key: cleanKey,
      model: cleanModel,
      available_models: fetchedModels
    };

    const existingIdx = savedOpenAiProfiles.findIndex(p => p.id === targetId);
    let nextList: OpenAiProfile[];
    if (existingIdx >= 0) {
      nextList = [...savedOpenAiProfiles];
      nextList[existingIdx] = updatedProfile;
    } else {
      nextList = [...savedOpenAiProfiles, updatedProfile];
    }

    selectedProfileId = targetId;
    profileName = cleanName;
    openaiBaseUrl = cleanUrl;
    openaiApiKey = cleanKey;
    openaiModel = cleanModel;

    await persistProfilesList(nextList);
    pipelineStore.updateConfig({
      text_engine: 'openai',
      openai_profile_id: targetId,
      openai_base_url: cleanUrl,
      openai_api_key: cleanKey,
      openai_model: cleanModel
    });

    saveFeedback = '✓ Сохранено в локальной памяти';
    setTimeout(() => {
      saveFeedback = '';
    }, 3000);
  }

  function handleStartNewProfile() {
    selectOpenAiProfile('__new__', false);
  }

  async function handleDeleteOpenAiProfile() {
    if (!selectedProfileId || selectedProfileId === '__new__') return;
    const nextList = savedOpenAiProfiles.filter(p => p.id !== selectedProfileId);
    await persistProfilesList(nextList);
    if (nextList.length > 0) {
      selectOpenAiProfile(nextList[0].id, true);
    } else {
      selectOpenAiProfile('__new__', true);
    }
    saveFeedback = 'Профиль удалён';
    setTimeout(() => {
      saveFeedback = '';
    }, 2500);
  }

  function applyOpenAiPreset(name: string, url: string, defaultModel: string) {
    profileName = name;
    openaiBaseUrl = url;
    openaiModel = defaultModel;
    fetchedModels = [];
    fetchModelsError = '';
    fetchModelsSuccess = '';
    syncOpenAiFieldsToStore();
  }

  onMount(() => {
    modelsStore.init();
    initOpenAiProfiles();
  });

  function handleIntensityChange(e: Event) {
    const value = parseFloat((e.target as HTMLInputElement).value);
    pipelineStore.updateConfig({ min_intensity_threshold: value });
  }

  function handleDurationChange(e: Event) {
    const value = parseInt((e.target as HTMLInputElement).value, 10);
    pipelineStore.updateConfig({ max_clip_duration_sec: value });
  }

  function handleMaxClipsChange(e: Event) {
    const value = parseInt((e.target as HTMLInputElement).value, 10);
    pipelineStore.updateConfig({ max_clips_count: value });
  }

  function handleVerticalChange(e: Event) {
    const value = (e.target as HTMLInputElement).checked;
    pipelineStore.updateConfig({ vertical_format: value });
  }

  function handleHighlightsChange(e: Event) {
    const value = (e.target as HTMLInputElement).checked;
    pipelineStore.updateConfig({ render_highlights: value });
  }

  function handleTalkVideoChange(e: Event) {
    const value = (e.target as HTMLInputElement).checked;
    pipelineStore.updateConfig({ render_talk_video: value });
  }

  function handleBumperToggle(e: Event) {
    const checked = (e.target as HTMLInputElement).checked;
    const currentCfg = $pipelineStore.config.bumper_config || {
      enabled: checked,
      active_preset_id: 'default',
      presets: []
    };
    pipelineStore.updateConfig({
      bumper_config: {
        ...currentCfg,
        enabled: checked
      }
    });
  }

  let loadedCharacters: import('../ipc').SubtitleCharacterInfo[] = [];

  $: if ($pipelineStore.config.custom_subtitles_path) {
    const path = $pipelineStore.config.custom_subtitles_path;
    import('../ipc').then(({ parseCustomSubtitles }) => {
      parseCustomSubtitles(path).then(res => {
        loadedCharacters = res.characters;
      }).catch(() => {
        loadedCharacters = [];
      });
    });
  } else {
    loadedCharacters = [];
  }

  function toggleCharacter(name: string) {
    let current = $pipelineStore.config.selected_characters || [];
    if (current.includes(name)) {
      current = current.filter(c => c !== name);
    } else {
      current = [...current, name];
    }
    pipelineStore.updateConfig({ selected_characters: current });
  }

  function selectAllCharacters() {
    const all = loadedCharacters.map(c => c.name);
    pipelineStore.updateConfig({ selected_characters: all });
  }

  function clearAllCharacters() {
    pipelineStore.updateConfig({ selected_characters: [] });
  }

  function handleAsrModelChange(e: Event) {
    const value = (e.target as HTMLSelectElement | HTMLInputElement).value;
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
    if (value === 'openai') {
      const activeProf = savedOpenAiProfiles.find(p => p.id === selectedProfileId) || savedOpenAiProfiles[0];
      if (activeProf) {
        selectedProfileId = activeProf.id;
        profileName = activeProf.name;
        openaiBaseUrl = activeProf.base_url;
        openaiApiKey = activeProf.api_key;
        openaiModel = activeProf.model;
        fetchedModels = activeProf.available_models || [];
        pipelineStore.updateConfig({
          text_engine: 'openai',
          openai_profile_id: activeProf.id,
          openai_base_url: activeProf.base_url,
          openai_api_key: activeProf.api_key,
          openai_model: activeProf.model
        });
        return;
      } else {
        pipelineStore.updateConfig({
          text_engine: 'openai',
          openai_base_url: openaiBaseUrl.trim() || 'https://api.openai.com/v1',
          openai_api_key: openaiApiKey.trim(),
          openai_model: openaiModel.trim() || 'gpt-4o-mini'
        });
        return;
      }
    }
    pipelineStore.updateConfig({ text_engine: value });
  }

  function handleTextModelChange(e: Event) {
    const value = (e.target as HTMLSelectElement | HTMLInputElement).value;
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
    let currentFragments: Fragment[] = [];
    fragmentsStore.subscribe(f => (currentFragments = f))();
    pipelineStore.exportAndRenderClips(currentFragments);
  }

  function toggleAdvanced() {
    showAdvanced = !showAdvanced;
  }

  function startProcess() {
    pipelineStore.startPipelineProcess();
  }

  $: whisperModels = $modelsStore.models.filter(m => m.category === 'whisper');
  $: llmModels = $modelsStore.models.filter(m => m.category === 'llm');
  $: visionModels = $modelsStore.models.filter(m => m.category === 'vision');
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

      <!-- Control 2: Dynamic Complete-Phrase Clip Duration -->
      <div class="flex flex-col gap-2">
        <div class="flex justify-between items-baseline">
          <span class="label-mono text-[#e2e2e4]">Длительность (по фразам)</span>
          <span class="font-mono text-[#5865f2] text-xs font-semibold">
            {Math.max(5, $pipelineStore.config.max_clip_duration_sec - 5)}–{$pipelineStore.config.max_clip_duration_sec + 5} сек (цель {$pipelineStore.config.max_clip_duration_sec}с)
          </span>
        </div>
        <input 
          type="range" 
          min="10" 
          max="55" 
          step="1" 
          value={$pipelineStore.config.max_clip_duration_sec}
          on:input={handleDurationChange}
          disabled={$pipelineStore.status === 'running'}
        />
        <div class="flex justify-between items-center text-[10px] text-[rgba(226,226,224,0.4)] font-mono">
          <span>10–20 с (Цельные фразы)</span>
          <span>50–60 сек</span>
        </div>
      </div>

      <!-- Control 3: Max Clips Count -->
      <div class="flex flex-col gap-2">
        <div class="flex justify-between items-baseline">
          <span class="label-mono text-[#e2e2e4]">Количество клипов</span>
          <span class="font-mono text-[#5865f2] text-xs font-semibold">
            {($pipelineStore.config.max_clips_count ?? 10) === 0 ? 'Авто (все яркие моменты)' : `до ${$pipelineStore.config.max_clips_count ?? 10} шт.`}
          </span>
        </div>
        <input 
          type="range" 
          min="0" 
          max="50" 
          step="1" 
          value={$pipelineStore.config.max_clips_count ?? 10}
          on:input={handleMaxClipsChange}
          disabled={$pipelineStore.status === 'running'}
        />
        <div class="flex justify-between items-center text-[10px] text-[rgba(226,226,224,0.4)] font-mono">
          <span>0 (Авто / без лимита)</span>
          <span>50 (Максимум)</span>
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
          <div class="text-[11px] text-[rgba(226,226,224,0.5)] mt-0.5 font-sans">AI-фокусировка по лицам (при снятой галочке — горизонтальные шортсы 16:9)</div>
        </div>
      </label>

      <!-- Checkbox: Highlights Assembly (16:9) -->
      <label class="flex items-center gap-3.5 p-3.5 border border-[rgba(226,226,224,0.1)] rounded-[4px] cursor-pointer hover:border-[rgba(226,226,224,0.2)] bg-[#0c0c0e]/60 transition-colors select-none">
        <input 
          type="checkbox" 
          checked={$pipelineStore.config.render_highlights ?? false}
          on:change={handleHighlightsChange}
          disabled={$pipelineStore.status === 'running'}
          class="w-4 h-4 rounded-[2px] bg-[#16161a] border-[rgba(226,226,224,0.3)] text-[#5865f2] focus:ring-0 focus:ring-offset-0 cursor-pointer accent-[#5865f2]"
        />
        <div>
          <div class="label-mono text-[#e2e2e4] normal-case tracking-normal text-xs font-medium">Сборка хайлайтов (16:9 YouTube)</div>
          <div class="text-[11px] text-[rgba(226,226,224,0.5)] mt-0.5 font-sans">Сборник лучших моментов в единый ролик с мягкими переходами (Crossfade 0.5с)</div>
        </div>
      </label>

      <!-- Checkbox: Talk Video Assembly -->
      <label class="flex items-center gap-3.5 p-3.5 border border-[rgba(226,226,224,0.1)] rounded-[4px] cursor-pointer hover:border-[rgba(226,226,224,0.2)] bg-[#0c0c0e]/60 transition-colors select-none">
        <input 
          type="checkbox" 
          checked={$pipelineStore.config.render_talk_video ?? false}
          on:change={handleTalkVideoChange}
          disabled={$pipelineStore.status === 'running'}
          class="w-4 h-4 rounded-[2px] bg-[#16161a] border-[rgba(226,226,224,0.3)] text-[#5865f2] focus:ring-0 focus:ring-offset-0 cursor-pointer accent-[#5865f2]"
        />
        <div>
          <div class="label-mono text-[#e2e2e4] normal-case tracking-normal text-xs font-medium">Монтаж разговорного видео / Демки</div>
          <div class="text-[11px] text-[rgba(226,226,224,0.5)] mt-0.5 font-sans">Чистовой подкаст/демка актера без пауз и оговорок с динамическим зумом на склейках</div>
        </div>
      </label>

      <!-- Checkbox 4: Preroll & Postroll (Bumpers) -->
      <div class="flex items-center justify-between p-3.5 border border-[rgba(226,226,224,0.1)] rounded-[4px] bg-[#0c0c0e]/60 transition-colors select-none">
        <label class="flex items-center gap-3.5 cursor-pointer flex-1">
          <input 
            type="checkbox" 
            checked={$pipelineStore.config.bumper_config?.enabled ?? false}
            on:change={handleBumperToggle}
            disabled={$pipelineStore.status === 'running'}
            class="w-4 h-4 rounded-[2px] bg-[#16161a] border-[rgba(226,226,224,0.3)] text-[#5865f2] focus:ring-0 focus:ring-offset-0 cursor-pointer accent-[#5865f2]"
          />
          <div>
            <div class="label-mono text-[#e2e2e4] normal-case tracking-normal text-xs font-medium">Преролл и постролл (Заставки)</div>
            <div class="text-[11px] text-[rgba(226,226,224,0.5)] mt-0.5 font-sans">Авто-прикрепление заставок перед и после каждого ролика (9:16 и 16:9)</div>
          </div>
        </label>
        <button
          type="button"
          on:click={() => (showBumperModal = true)}
          disabled={$pipelineStore.status === 'running'}
          class="px-3 py-1.5 bg-[#5865f2]/20 hover:bg-[#5865f2]/40 border border-[#5865f2]/50 text-[#e2e2e4] font-mono text-xs rounded transition-colors flex items-center gap-1.5 cursor-pointer"
        >
          <Settings class="w-3.5 h-3.5 text-[#5865f2]" />
          <span>Настроить</span>
        </button>
      </div>

      <!-- Character Filter Section (if custom subtitles are loaded) -->
      {#if $pipelineStore.config.custom_subtitles_path && loadedCharacters.length > 0}
        <div class="border border-[rgba(226,226,224,0.15)] bg-[#0c0c0e] p-4 rounded-[4px] flex flex-col gap-3">
          <div class="flex items-center justify-between">
            <div class="label-mono text-xs text-[#e2e2e4] normal-case font-medium flex items-center gap-2">
              <span>🎭</span>
              <span>Фильтр по персонажам / актерам</span>
            </div>
            <div class="flex items-center gap-2 text-[10px] font-mono">
              <button type="button" on:click={selectAllCharacters} class="text-[#5865f2] hover:underline cursor-pointer">Выбрать всех</button>
              <span class="text-[rgba(226,226,224,0.3)]">|</span>
              <button type="button" on:click={clearAllCharacters} class="text-[rgba(226,226,224,0.5)] hover:underline cursor-pointer">Сбросить</button>
            </div>
          </div>

          <p class="text-[11px] text-[rgba(226,226,224,0.5)] font-mono leading-relaxed">
            Отметьте персонажей для нарезки персонажных шортсов, хайлайтов или демок озвучки:
          </p>

          <div class="flex flex-wrap gap-2 max-h-[160px] overflow-y-auto pr-1">
            {#each loadedCharacters as char}
              {@const isSelected = ($pipelineStore.config.selected_characters || []).includes(char.name)}
              <button 
                type="button"
                on:click={() => toggleCharacter(char.name)}
                disabled={$pipelineStore.status === 'running'}
                class="px-3 py-1.5 rounded-[3px] text-xs font-mono font-medium border transition-colors cursor-pointer flex items-center gap-1.5 {
                  isSelected 
                    ? 'bg-[#5865f2] border-[#5865f2] text-white shadow-sm' 
                    : 'bg-[#16161a] border-[rgba(226,226,224,0.15)] text-[rgba(226,226,224,0.7)] hover:border-[rgba(226,226,224,0.3)] hover:text-[#e2e2e4]'
                }"
              >
                <span>{isSelected ? '✓' : '+'}</span>
                <span>{char.name}</span>
                <span class="text-[10px] opacity-75">({char.line_count})</span>
              </button>
            {/each}
          </div>
        </div>
      {/if}

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
            <span class="label-mono text-[#5865f2]">Менеджер весов (≤3B)</span>
            <button 
              type="button"
              on:click={onOpenModelManager}
              class="label-mono text-[#5865f2] hover:underline cursor-pointer font-bold"
            >
              [СКАЧАТЬ / УДАЛИТЬ МОДЕЛИ]
            </button>
          </div>

          <!-- ASR model -->
          <div class="flex flex-col gap-1">
            <label for="asr_model" class="label-mono">Модель распознавания речи (Whisper)</label>
            <select
              id="asr_model"
              value={$pipelineStore.config.asr_model_path}
              on:change={handleAsrModelChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            >
              {#each whisperModels as wm}
                <option value={joinRelativePath('models', wm.filename)}>
                  {wm.exists ? '✓ ' : ''}{wm.name} ({wm.size_mb} МБ){wm.exists ? ' [Скачана]' : ' [Не скачана]'}
                </option>
              {/each}
            </select>
          </div>

          <!-- NLP Text Engine -->
          <div class="flex flex-col gap-1">
            <label for="text_engine" class="label-mono">Движок анализа диалогов и хайлайтов</label>
            <select 
              id="text_engine"
              value={$pipelineStore.config.text_engine}
              on:change={handleTextEngineChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            >
              <option value="llama">Локальная ИИ-модель GGUF (llama-cli + авто-резерв NLP)</option>
              <option value="builtin">Быстрый эвристический NLP-алгоритм (без LLM)</option>
              <option value="ollama">Внешний Ollama сервер (Опционально)</option>
              <option value="openai">OpenAI API ключ (OpenAI-совместимый сервер)</option>
            </select>
          </div>

          {#if $pipelineStore.config.text_engine === 'llama'}
            <!-- Local GGUF NLP Model -->
            <div class="flex flex-col gap-1">
              <label for="text_model" class="label-mono">Модель анализа диалогов (LLM до 3B)</label>
              <select
                id="text_model"
                value={$pipelineStore.config.text_model_name}
                on:change={handleTextModelChange}
                disabled={$pipelineStore.status === 'running'}
                class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
              >
                {#each llmModels as lm}
                  <option value={lm.filename}>
                    {lm.exists ? '✓ ' : ''}{lm.name} ({lm.params}) {lm.exists ? '[Скачана]' : '[Не скачана]'}
                  </option>
                {/each}
              </select>
            </div>
          {:else if $pipelineStore.config.text_engine === 'ollama'}
            <!-- Ollama Server Settings -->
            <div class="p-3 bg-[#16161a]/70 border border-[rgba(226,226,224,0.12)] rounded-[3px] flex flex-col gap-2.5">
              <div class="flex flex-col gap-1">
                <label for="ollama_url" class="label-mono">Адрес сервера Ollama</label>
                <input
                  id="ollama_url"
                  type="text"
                  placeholder="http://localhost:11434"
                  value={$pipelineStore.config.ollama_url}
                  on:input={handleOllamaUrlChange}
                  disabled={$pipelineStore.status === 'running'}
                  class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
                />
              </div>
              <div class="flex flex-col gap-1">
                <label for="ollama_model" class="label-mono">Имя модели в Ollama</label>
                <input
                  id="ollama_model"
                  type="text"
                  placeholder="qwen2.5:3b"
                  value={$pipelineStore.config.text_model_name}
                  on:input={handleTextModelChange}
                  disabled={$pipelineStore.status === 'running'}
                  class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
                />
              </div>
            </div>
          {:else if $pipelineStore.config.text_engine === 'openai'}
            <!-- OpenAI-Compatible API Settings & Local Multi-Key Manager -->
            <div class="p-3 bg-[#16161a]/80 border border-[#5865f2]/35 rounded-[3px] flex flex-col gap-2.5">
              <div class="flex items-center justify-between">
                <span class="label-mono text-[#5865f2]">Настройки OpenAI-совместимого API</span>
                {#if saveFeedback}
                  <span class="font-mono text-[10px] text-[#23c55e]">{saveFeedback}</span>
                {/if}
              </div>

              <!-- Saved Profiles Selector -->
              <div class="flex flex-col gap-1">
                <div class="flex items-center justify-between">
                  <label for="openai_profile_select" class="label-mono">Сохранённые ключи / профили</label>
                  <div class="flex items-center gap-2">
                    <button
                      type="button"
                      on:click={handleStartNewProfile}
                      disabled={$pipelineStore.status === 'running'}
                      class="font-mono text-[10px] text-[#5865f2] hover:underline cursor-pointer"
                    >
                      [+ ДОБАВИТЬ ЕЩЁ КЛЮЧ]
                    </button>
                    {#if selectedProfileId && selectedProfileId !== '__new__' && savedOpenAiProfiles.length > 0}
                      <button
                        type="button"
                        on:click={handleDeleteOpenAiProfile}
                        disabled={$pipelineStore.status === 'running'}
                        class="font-mono text-[10px] text-[#ef4444] hover:underline cursor-pointer"
                      >
                        [УДАЛИТЬ]
                      </button>
                    {/if}
                  </div>
                </div>
                <select
                  id="openai_profile_select"
                  value={selectedProfileId || '__new__'}
                  on:change={handleProfileSelectChange}
                  disabled={$pipelineStore.status === 'running'}
                  class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.18)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
                >
                  {#if savedOpenAiProfiles.length === 0}
                    <option value="__new__">— Нет сохранённых ключей (создание нового) —</option>
                  {:else}
                    {#each savedOpenAiProfiles as prof}
                      <option value={prof.id}>
                        🔑 {prof.name} — {prof.model} ({prof.base_url.replace(/^https?:\/\//, '')})
                      </option>
                    {/each}
                    <option value="__new__">+ Создать новый профиль API-ключа...</option>
                  {/if}
                </select>
              </div>

              <!-- Quick Presets -->
              <div class="flex flex-wrap items-center gap-1.5">
                <span class="text-[10px] font-mono text-[rgba(226,226,224,0.45)]">Шаблоны:</span>
                <button
                  type="button"
                  on:click={() => applyOpenAiPreset('OpenAI', 'https://api.openai.com/v1', 'gpt-4o-mini')}
                  class="px-1.5 py-0.5 text-[10px] font-mono bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] hover:border-[#5865f2] text-[rgba(226,226,224,0.8)] rounded-[2px] cursor-pointer"
                >
                  OpenAI
                </button>
                <button
                  type="button"
                  on:click={() => applyOpenAiPreset('OpenRouter', 'https://openrouter.ai/api/v1', 'openai/gpt-4o-mini')}
                  class="px-1.5 py-0.5 text-[10px] font-mono bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] hover:border-[#5865f2] text-[rgba(226,226,224,0.8)] rounded-[2px] cursor-pointer"
                >
                  OpenRouter
                </button>
                <button
                  type="button"
                  on:click={() => applyOpenAiPreset('DeepSeek', 'https://api.deepseek.com/v1', 'deepseek-chat')}
                  class="px-1.5 py-0.5 text-[10px] font-mono bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] hover:border-[#5865f2] text-[rgba(226,226,224,0.8)] rounded-[2px] cursor-pointer"
                >
                  DeepSeek
                </button>
                <button
                  type="button"
                  on:click={() => applyOpenAiPreset('Groq', 'https://api.groq.com/openai/v1', 'llama-3.3-70b-versatile')}
                  class="px-1.5 py-0.5 text-[10px] font-mono bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] hover:border-[#5865f2] text-[rgba(226,226,224,0.8)] rounded-[2px] cursor-pointer"
                >
                  Groq
                </button>
                <button
                  type="button"
                  on:click={() => applyOpenAiPreset('LM Studio / Local', 'http://localhost:1234/v1', 'local-model')}
                  class="px-1.5 py-0.5 text-[10px] font-mono bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] hover:border-[#5865f2] text-[rgba(226,226,224,0.8)] rounded-[2px] cursor-pointer"
                >
                  Local / LM Studio
                </button>
              </div>

              <!-- Profile Name -->
              <div class="flex flex-col gap-1">
                <label for="openai_profile_name" class="label-mono">Название профиля (для списка)</label>
                <input
                  id="openai_profile_name"
                  type="text"
                  placeholder="Например: Мой OpenAI / OpenRouter / DeepSeek"
                  bind:value={profileName}
                  disabled={$pipelineStore.status === 'running'}
                  class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
                />
              </div>

              <!-- Server Base URL -->
              <div class="flex flex-col gap-1">
                <label for="openai_base_url" class="label-mono">Сервер API (Base URL)</label>
                <input
                  id="openai_base_url"
                  type="text"
                  placeholder="https://api.openai.com/v1"
                  bind:value={openaiBaseUrl}
                  on:input={syncOpenAiFieldsToStore}
                  disabled={$pipelineStore.status === 'running'}
                  class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
                />
              </div>

              <!-- API Key -->
              <div class="flex flex-col gap-1">
                <div class="flex items-center justify-between">
                  <label for="openai_api_key" class="label-mono">API-ключ (Bearer Token)</label>
                  <button
                    type="button"
                    on:click={() => (showApiKey = !showApiKey)}
                    class="font-mono text-[10px] text-[rgba(226,226,224,0.6)] hover:text-[#e2e2e4] cursor-pointer"
                  >
                    {showApiKey ? '[СКРЫТЬ]' : '[ПОКАЗАТЬ]'}
                  </button>
                </div>
                <input
                  id="openai_api_key"
                  type={showApiKey ? 'text' : 'password'}
                  placeholder="Введите API-ключ..."
                  bind:value={openaiApiKey}
                  on:input={syncOpenAiFieldsToStore}
                  disabled={$pipelineStore.status === 'running'}
                  class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
                />
              </div>

              <!-- Model Name & Server Model Fetcher -->
              <div class="flex flex-col gap-1.5">
                <div class="flex items-center justify-between gap-2">
                  <label for="openai_model" class="label-mono">Модель (Model ID)</label>
                  <button
                    type="button"
                    on:click={handleFetchModelsFromServer}
                    disabled={$pipelineStore.status === 'running' || isFetchingModels}
                    class="font-mono text-[10px] px-2 py-0.5 bg-[#5865f2]/20 hover:bg-[#5865f2]/35 border border-[#5865f2]/60 text-[#e2e2e4] rounded-[2px] cursor-pointer transition-colors disabled:opacity-40 flex items-center gap-1"
                  >
                    {#if isFetchingModels}
                      <span class="inline-block h-2.5 w-2.5 border border-white/30 border-t-white rounded-full animate-spin"></span>
                      <span>ЗАПРОС С СЕРВЕРА...</span>
                    {:else}
                      <span>↻ ЗАПРОСИТЬ С СЕРВЕРА</span>
                    {/if}
                  </button>
                </div>

                {#if fetchModelsSuccess}
                  <div class="text-[10px] font-mono text-[#23c55e]">{fetchModelsSuccess}</div>
                {/if}
                {#if fetchModelsError}
                  <div class="text-[10px] font-mono text-[#ef4444] break-words">{fetchModelsError}</div>
                {/if}

                {#if fetchedModels.length > 0}
                  <div class="p-2 bg-[#0c0c0e]/90 border border-[#5865f2]/30 rounded-[2px] flex flex-col gap-1.5">
                    <div class="flex items-center justify-between">
                      <span class="font-mono text-[10px] text-[rgba(226,226,224,0.7)]">
                        Выберите модель из списка сервера ({filteredFetchedModels.length} из {fetchedModels.length}):
                      </span>
                    </div>
                    {#if fetchedModels.length > 8}
                      <input
                        type="text"
                        placeholder="Поиск модели (например: gpt, llama, deepseek, qwen)..."
                        bind:value={modelSearchQuery}
                        disabled={$pipelineStore.status === 'running'}
                        class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2 py-1 text-[11px] text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
                      />
                    {/if}
                    <select
                      value={filteredFetchedModels.includes(openaiModel) ? openaiModel : ''}
                      on:change={handleSelectFetchedModel}
                      disabled={$pipelineStore.status === 'running'}
                      class="w-full bg-[#16161a] border border-[#5865f2]/50 rounded-[2px] px-2 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
                    >
                      {#if !filteredFetchedModels.includes(openaiModel)}
                        <option value="">— Выберите Model ID ({filteredFetchedModels.length} доступно) —</option>
                      {/if}
                      {#each filteredFetchedModels as mId}
                        <option value={mId}>{mId}</option>
                      {/each}
                    </select>
                  </div>
                {/if}

                <div class="flex gap-1.5">
                  <input
                    id="openai_model"
                    type="text"
                    placeholder="gpt-4o-mini, deepseek-chat, openai/gpt-4o-mini..."
                    bind:value={openaiModel}
                    on:input={syncOpenAiFieldsToStore}
                    disabled={$pipelineStore.status === 'running'}
                    class="flex-1 bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
                  />
                  <button
                    type="button"
                    on:click={handleFetchModelsFromServer}
                    disabled={$pipelineStore.status === 'running' || isFetchingModels}
                    title="Запросить список доступных Model ID с указанного сервера"
                    class="px-2.5 py-1.5 bg-[#0c0c0e] hover:bg-[#5865f2]/20 border border-[rgba(226,226,224,0.2)] hover:border-[#5865f2] text-[11px] font-mono text-[#e2e2e4] rounded-[2px] cursor-pointer transition-colors disabled:opacity-40 whitespace-nowrap"
                  >
                    {isFetchingModels ? '...' : 'Список ID'}
                  </button>
                </div>
              </div>

              <!-- Save Button -->
              <button
                type="button"
                on:click={handleSaveOpenAiProfile}
                disabled={$pipelineStore.status === 'running'}
                class="mt-1 w-full py-2 px-3 bg-[#5865f2] hover:bg-[#4752c4] text-white font-mono text-xs font-bold uppercase tracking-wider rounded-[2px] cursor-pointer transition-colors"
              >
                {selectedProfileId && selectedProfileId !== '__new__' && savedOpenAiProfiles.some(p => p.id === selectedProfileId)
                  ? 'Сохранить / обновить выбранный ключ'
                  : 'Сохранить ключ локально'}
              </button>
            </div>
          {/if}

          <!-- Vision model -->
          <div class="flex flex-col gap-1">
            <label for="vision_model" class="label-mono">Модель анализа кадров (Vision AI + mmproj)</label>
            <select
              id="vision_model"
              value={$pipelineStore.config.vision_model_path || ''}
              on:change={handleVisionModelChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            >
              <option value="">Авто / Двухфакторный анализ (FFmpeg сцены + аудио)</option>
              {#each visionModels as vm}
                <option value={joinRelativePath('models', vm.filename)}>
                  {vm.exists ? '✓ ' : ''}{vm.name} ({vm.params}){vm.exists ? ' [Скачана]' : ' [Не скачана]'}
                </option>
              {/each}
            </select>
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

<BumperModal show={showBumperModal} on:close={() => (showBumperModal = false)} />

