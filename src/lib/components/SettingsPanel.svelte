<!-- src/lib/components/SettingsPanel.svelte -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { pipelineStore } from '../stores/pipeline';
  import { fragmentsStore, type Fragment } from '../stores/fragments';
  import { modelsStore } from '../stores/models';
  import { getOpenAiProfiles, saveOpenAiProfiles, fetchOpenAiModels, parseCustomSubtitles, saveBumperConfig, type OpenAiProfile, type SubtitleCharacterInfo } from '../ipc';
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

  function createFallbackBumperItem(title: string, subtitle: string) {
    return {
      enabled: true,
      mode: 'generate' as const,
      custom_video_path: null,
      custom_audio_path: null,
      custom_audio_duration: null,
      sync_duration_to_audio: true,
      audio_volume: 1.0,
      audio_fade_in: true,
      audio_fade_out: true,
      title,
      subtitle,
      duration_sec: 2.5,
      bg_mode: 'blur_frame' as const,
      bg_video_path: null,
      bg_image_path: null,
      bg_color_primary: '#0f172a',
      bg_color_secondary: '#312e81',
      bg_blur: 12,
      bg_dim: 0.35,
      bg_scale: 1.1,
      bg_saturation: 1.0,
      bg_contrast: 1.0,
      bg_vignette: true,
      bg_video_speed: 1.0,
      logo_image_path: null,
      logo_scale: 0.45,
      logo_x_percent: 50,
      logo_y_percent: 36,
      logo_opacity: 1.0,
      logo_radius: 0,
      logo_shadow: true,
      title_color: '#FFFFFF',
      title_size: 36,
      subtitle_size: 18,
      subtitle_color: '#cbd5e1',
      outline_color: '#000000',
      outline_width: 2,
      plate_style: 'glass' as const,
      plate_color: '#000000',
      plate_opacity: 0.55,
      title_x_percent: 50,
      title_y_percent: 74,
      animation: 'fade_zoom' as const,
      transition: 'crossfade' as const,
      overlay_png_base64: null
    };
  }

  function handleBumperToggle(e: Event) {
    const checked = (e.target as HTMLInputElement).checked;
    const existing = $pipelineStore.config.bumper_config;
    const hasPresets = existing && Array.isArray(existing.presets) && existing.presets.length > 0;
    const currentCfg = hasPresets
      ? existing
      : {
          enabled: checked,
          active_preset_id: 'default',
          presets: [
            {
              id: 'default',
              name: 'Студия озвучки (По умолчанию)',
              v916: {
                preroll: createFallbackBumperItem('Студия озвучки', 'Озвучено специально для вас'),
                postroll: createFallbackBumperItem('Подпишись на канал', 'Ставь лайк и пиши комментарий!')
              },
              v169: {
                preroll: createFallbackBumperItem('Студия озвучки', 'Приятного просмотра'),
                postroll: createFallbackBumperItem('Спасибо за просмотр!', 'Подписывайтесь на наши соцсети')
              },
              created_at: Date.now()
            }
          ]
        };
    const updated = {
      ...currentCfg,
      enabled: checked
    };
    pipelineStore.updateConfig({
      bumper_config: updated
    });
    if (isTauriRuntime()) {
      saveBumperConfig(updated).catch(() => {});
    }
  }

  let loadedCharacters: SubtitleCharacterInfo[] = [];

  $: if ($pipelineStore.customSubtitles && $pipelineStore.customSubtitles.characters.length > 0) {
    loadedCharacters = $pipelineStore.customSubtitles.characters;
  } else if ($pipelineStore.config.custom_subtitles_path) {
    const path = $pipelineStore.config.custom_subtitles_path;
    parseCustomSubtitles(path).then(res => {
      loadedCharacters = res.characters;
    }).catch(() => {
      loadedCharacters = [];
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

  function handleVerifierModelChange(e: Event) {
    const value = (e.target as HTMLSelectElement | HTMLInputElement).value;
    pipelineStore.updateConfig({ verifier_model_name: value });
  }

  function handleSmmModelChange(e: Event) {
    const value = (e.target as HTMLSelectElement | HTMLInputElement).value;
    pipelineStore.updateConfig({ smm_model_name: value });
  }

  function handleOllamaUrlChange(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    pipelineStore.updateConfig({ ollama_url: value });
  }

  const GENERAL_TOPIC_PRESETS = [
    { label: 'Юмор и панчлайны', prompt: 'Юмор, сарказм, смешные панчлайны и абсурдные ситуации' },
    { label: 'Деньги и цифры', prompt: 'Деньги, цифры, бизнес-кейсы, доходы и провалы' },
    { label: 'Споры и интрига', prompt: 'Острые споры, конфликты, неожиданные противоречия и интрига' },
    { label: 'Советы и лайфхаки', prompt: 'Практические советы, правила, инсайты и пошаговые выводы' }
  ];

  const ANIME_TOPIC_PRESETS = [
    { label: 'Шутки', prompt: 'Аниме: шутки, комедийные сценки, подколы и нелепые реакции' },
    { label: 'Экшен', prompt: 'Аниме: экшен, напряженные бои, атаки, техники и боевые крики' },
    { label: 'Драма', prompt: 'Аниме: сильная драма, клятвы, слезы, признания и эмоциональный накал' },
    { label: 'Повороты', prompt: 'Аниме: неожиданные сюжетные повороты, тайны, предательства и раскрытия' }
  ];

  function handleAiPromptInput(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    pipelineStore.updateConfig({ ai_prompt: value });
  }

  function toggleTopicPreset(promptText: string) {
    const current = ($pipelineStore.config.ai_prompt || '').trim();
    const next = current === promptText ? '' : promptText;
    pipelineStore.updateConfig({ ai_prompt: next });
  }

  function clearAiPrompt() {
    pipelineStore.updateConfig({ ai_prompt: '' });
  }

  function handleDistributeEvenlyChange(e: Event) {
    const checked = (e.target as HTMLInputElement).checked;
    pipelineStore.updateConfig({ distribute_evenly: checked });
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
  $: installedLlmModels = llmModels.filter(m => m.exists);
  $: uninstalledLlmModels = llmModels.filter(m => !m.exists);
  $: visionModels = $modelsStore.models.filter(m => m.category === 'vision');
</script>

<div class="panel p-6 sm:p-8 flex flex-col justify-between h-full">
  <div>
    <!-- Panel Header -->
    <div class="panel-title flex items-center justify-between pb-4 mb-6 border-b border-[rgba(226,226,224,0.1)]">
      <span>Настройки нарезки</span>
    </div>

    <!-- Controls Stack -->
    <div class="flex flex-col gap-6">
      
      <!-- Control 1: Emotion Intensity -->
      <div class="flex flex-col gap-2">
        <div class="flex justify-between items-baseline">
          <span class="label-mono text-[#e2e2e4]">Порог отбора моментов</span>
          <span class="font-mono text-[#5865f2] text-xs font-semibold tabular-nums">
            {Math.round($pipelineStore.config.min_intensity_threshold * 100)}%
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
        <div class="flex justify-between items-center text-[10px] text-[rgba(226,226,224,0.45)] font-mono">
          <span>Больше клипов</span>
          <span>Только лучшие пики</span>
        </div>
      </div>

      <!-- Control 2: Dynamic Complete-Phrase Clip Duration -->
      <div class="flex flex-col gap-2">
        <div class="flex justify-between items-baseline">
          <span class="label-mono text-[#e2e2e4]">Целевая длительность</span>
          <span class="font-mono text-[#5865f2] text-xs font-semibold tabular-nums">
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
        <div class="flex justify-between items-center text-[10px] text-[rgba(226,226,224,0.45)] font-mono">
          <span>10–20 сек</span>
          <span>50–60 сек</span>
        </div>
      </div>

      <!-- Control 3: Max Clips Count -->
      <div class="flex flex-col gap-2">
        <div class="flex justify-between items-baseline">
          <span class="label-mono text-[#e2e2e4]">Количество клипов</span>
          <span class="font-mono text-[#5865f2] text-xs font-semibold tabular-nums">
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
        <div class="flex justify-between items-center text-[10px] text-[rgba(226,226,224,0.45)] font-mono">
          <span>Без лимита</span>
          <span>до 50 шт.</span>
        </div>
      </div>

      <!-- Checkbox 1: Vertical Format (9:16) -->
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
          <div class="text-[11px] text-[rgba(226,226,224,0.5)] mt-0.5 font-sans">Автоматический фокус на лицах для Shorts, Reels и TikTok (без галочки — 16:9)</div>
        </div>
      </label>

      <!-- Checkbox 2: Highlights Assembly (16:9) -->
      <label class="flex items-center gap-3.5 p-3.5 border border-[rgba(226,226,224,0.1)] rounded-[4px] cursor-pointer hover:border-[rgba(226,226,224,0.2)] bg-[#0c0c0e]/60 transition-colors select-none">
        <input 
          type="checkbox" 
          checked={$pipelineStore.config.render_highlights ?? false}
          on:change={handleHighlightsChange}
          disabled={$pipelineStore.status === 'running'}
          class="w-4 h-4 rounded-[2px] bg-[#16161a] border-[rgba(226,226,224,0.3)] text-[#5865f2] focus:ring-0 focus:ring-offset-0 cursor-pointer accent-[#5865f2]"
        />
        <div>
          <div class="label-mono text-[#e2e2e4] normal-case tracking-normal text-xs font-medium">Сборник лучших моментов (Хайлайты)</div>
          <div class="text-[11px] text-[rgba(226,226,224,0.5)] mt-0.5 font-sans">Склеивает лучшие сцены с субтитрами и плавными переходами в один ролик 16:9</div>
        </div>
      </label>

      <!-- Checkbox 3: Talk Video Assembly -->
      <label class="flex items-center gap-3.5 p-3.5 border border-[rgba(226,226,224,0.1)] rounded-[4px] cursor-pointer hover:border-[rgba(226,226,224,0.2)] bg-[#0c0c0e]/60 transition-colors select-none">
        <input 
            type="checkbox" 
            checked={$pipelineStore.config.render_talk_video ?? false}
            on:change={handleTalkVideoChange}
            disabled={$pipelineStore.status === 'running'}
            class="w-4 h-4 rounded-[2px] bg-[#16161a] border-[rgba(226,226,224,0.3)] text-[#5865f2] focus:ring-0 focus:ring-offset-0 cursor-pointer accent-[#5865f2]"
        />
        <div>
          <div class="label-mono text-[#e2e2e4] normal-case tracking-normal text-xs font-medium">Разговорный монтаж (без пауз)</div>
          <div class="text-[11px] text-[rgba(226,226,224,0.5)] mt-0.5 font-sans">Удаляет тишину и заминки, добавляет субтитры и легкую смену крупности плана</div>
        </div>
      </label>

      <!-- Checkbox 4: Preroll & Postroll (Bumpers) -->
      <div class="flex items-center justify-between p-3.5 border border-[rgba(226,226,224,0.1)] rounded-[4px] bg-[#0c0c0e]/60 transition-colors select-none gap-2">
        <label class="flex items-center gap-3.5 cursor-pointer flex-1 min-w-0">
          <input 
            type="checkbox" 
            checked={$pipelineStore.config.bumper_config?.enabled ?? false}
            on:change={handleBumperToggle}
            disabled={$pipelineStore.status === 'running'}
            class="w-4 h-4 rounded-[2px] bg-[#16161a] border-[rgba(226,226,224,0.3)] text-[#5865f2] focus:ring-0 focus:ring-offset-0 cursor-pointer accent-[#5865f2] shrink-0"
          />
          <div class="min-w-0">
            <div class="label-mono text-[#e2e2e4] normal-case tracking-normal text-xs font-medium">Заставки (Преролл и Постролл)</div>
            <div class="text-[11px] text-[rgba(226,226,224,0.5)] mt-0.5 font-sans">Добавляет ваши заставки в начале и конце каждого готового ролика</div>
          </div>
        </label>
        <button
          type="button"
          on:click={() => (showBumperModal = true)}
          disabled={$pipelineStore.status === 'running'}
          class="px-3 py-1.5 bg-[#5865f2]/20 hover:bg-[#5865f2]/40 border border-[#5865f2]/50 text-[#e2e2e4] font-mono text-xs rounded transition-colors flex items-center gap-1.5 cursor-pointer whitespace-nowrap shrink-0"
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

          <p class="text-[11px] text-[rgba(226,226,224,0.5)] font-sans leading-relaxed">
            Отметьте нужных персонажей — программа нарежет моменты с их участием и сохранит короткие ответные реплики собеседников:
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

      <!-- AI Focus & Genre Presets -->
      <div class="flex flex-col gap-2.5 p-3.5 border border-[rgba(226,226,224,0.1)] rounded-[4px] bg-[#0c0c0e]/60">
        <div class="flex items-center justify-between gap-2">
          <label for="ai_prompt_input" class="label-mono text-[#e2e2e4]">Фокус и тема для ИИ (необязательно)</label>
          {#if ($pipelineStore.config.ai_prompt || '').trim().length > 0}
            <button
              type="button"
              on:click={clearAiPrompt}
              disabled={$pipelineStore.status === 'running'}
              class="font-mono text-[10px] text-[rgba(226,226,224,0.6)] hover:text-[#e2e2e4] cursor-pointer disabled:opacity-40 whitespace-nowrap"
            >
              Сбросить
            </button>
          {/if}
        </div>

        <input
          id="ai_prompt_input"
          type="text"
          value={$pipelineStore.config.ai_prompt || ''}
          on:input={handleAiPromptInput}
          disabled={$pipelineStore.status === 'running'}
          placeholder="Например: провалы в бизнесе или эпичные фразы перед боем..."
          class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-2 text-xs text-[#e2e2e4] font-sans placeholder:text-[rgba(226,226,224,0.35)] focus:border-[#5865f2] focus:outline-none"
        />

        <div class="flex flex-col gap-2 pt-0.5">
          <div class="flex flex-wrap items-center gap-1.5">
            <span class="font-mono text-[10px] text-[rgba(226,226,224,0.45)] mr-0.5">Общие:</span>
            {#each GENERAL_TOPIC_PRESETS as preset}
              {@const isActive = ($pipelineStore.config.ai_prompt || '').trim() === preset.prompt}
              <button
                type="button"
                on:click={() => toggleTopicPreset(preset.prompt)}
                disabled={$pipelineStore.status === 'running'}
                class="px-2 py-1 text-[10px] font-mono rounded-[2px] border transition-colors cursor-pointer whitespace-nowrap disabled:opacity-40 {
                  isActive
                    ? 'bg-[#5865f2] border-[#5865f2] text-white'
                    : 'bg-[#16161a] border-[rgba(226,226,224,0.15)] text-[rgba(226,226,224,0.75)] hover:border-[#5865f2]/60 hover:text-[#e2e2e4]'
                }"
              >
                {preset.label}
              </button>
            {/each}
          </div>

          <div class="flex flex-wrap items-center gap-1.5 pt-1.5 border-t border-[rgba(226,226,224,0.08)]">
            <span class="font-mono text-[10px] text-[rgba(226,226,224,0.45)] mr-0.5">Аниме:</span>
            {#each ANIME_TOPIC_PRESETS as preset}
              {@const isActive = ($pipelineStore.config.ai_prompt || '').trim() === preset.prompt}
              <button
                type="button"
                on:click={() => toggleTopicPreset(preset.prompt)}
                disabled={$pipelineStore.status === 'running'}
                class="px-2 py-1 text-[10px] font-mono rounded-[2px] border transition-colors cursor-pointer whitespace-nowrap disabled:opacity-40 {
                  isActive
                    ? 'bg-[#5865f2] border-[#5865f2] text-white'
                    : 'bg-[#16161a] border-[rgba(226,226,224,0.15)] text-[rgba(226,226,224,0.75)] hover:border-[#5865f2]/60 hover:text-[#e2e2e4]'
                }"
              >
                {preset.label}
              </button>
            {/each}
          </div>
        </div>
      </div>

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
            class="font-mono py-2.5 px-2 border-none uppercase text-xs font-bold tracking-wider cursor-pointer transition-colors rounded-[2px] whitespace-nowrap {
              $pipelineStore.config.mode === 'confirm' 
                ? 'bg-[#5865f2] text-white' 
                : 'bg-[rgba(226,226,224,0.1)] text-[rgba(226,226,224,0.6)] hover:text-[#e2e2e4]'
            }"
          >
            С проверкой
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
        <span>Дополнительные настройки</span>
        {#if showAdvanced}
          <ChevronUp class="h-3.5 w-3.5 text-[#5865f2]" />
        {:else}
          <ChevronDown class="h-3.5 w-3.5 text-[#5865f2]" />
        {/if}
      </button>

      {#if showAdvanced}
        <div class="mt-3.5 p-4 bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] rounded-[4px] flex flex-col gap-3">
          <div class="flex items-center justify-between">
            <span class="label-mono text-[#5865f2]">Локальные модели</span>
            <button 
              type="button"
              on:click={onOpenModelManager}
              class="label-mono text-[#5865f2] hover:underline cursor-pointer font-bold whitespace-nowrap"
            >
              Управление моделями
            </button>
          </div>

          <!-- ASR model -->
          <div class="flex flex-col gap-1">
            <label for="asr_model" class="label-mono">Точность распознавания речи</label>
            <select
              id="asr_model"
              value={$pipelineStore.config.asr_model_path}
              on:change={handleAsrModelChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            >
              {#each whisperModels as wm}
                <option value={joinRelativePath('models', wm.filename)}>
                  {wm.exists ? '✓ ' : ''}{wm.name} ({wm.size_mb} МБ){wm.exists ? ' (Установлена)' : ' (Не скачана)'}
                </option>
              {/each}
            </select>
          </div>

          <!-- NLP Text Engine -->
          <div class="flex flex-col gap-1">
            <label for="text_engine" class="label-mono">Способ анализа и подбора сцен</label>
            <select 
              id="text_engine"
              value={$pipelineStore.config.text_engine}
              on:change={handleTextEngineChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            >
              <option value="llama">Локальная нейросеть на ПК (рекомендуется)</option>
              <option value="builtin">Быстрый встроенный анализ (без нейросети)</option>
              <option value="ollama">Локальный сервер Ollama</option>
              <option value="openai">Внешний API-сервис (OpenAI / OpenRouter / DeepSeek)</option>
            </select>
          </div>

          {#if $pipelineStore.config.text_engine === 'llama'}
            <!-- 3-Stage Local AI Pipeline Configuration -->
            <div class="p-3 bg-[#16161a]/75 border border-[rgba(226,226,224,0.14)] rounded-[3px] flex flex-col gap-3">
              <div class="flex items-center justify-between">
                <span class="label-mono text-[#e2e2e4] normal-case text-xs font-semibold">Этапы работы нейросети</span>
                <span class="font-mono text-[10px] text-[#23c55e] tabular-nums">
                  Установлено: {installedLlmModels.length}/{llmModels.length}
                </span>
              </div>

              <!-- Stage 1: Initial replica scoring -->
              <div class="flex flex-col gap-1">
                <label for="text_model_stage1" class="label-mono text-[10px] text-[#9aa4ff]">
                  1. Отбор ярких сцен и эмоций
                </label>
                <select
                  id="text_model_stage1"
                  value={$pipelineStore.config.text_model_name}
                  on:change={handleTextModelChange}
                  disabled={$pipelineStore.status === 'running'}
                  class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.18)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
                >
                  {#if installedLlmModels.length > 0}
                    <optgroup label="Установленные модели">
                      {#each installedLlmModels as lm}
                        <option value={lm.filename}>
                          ✓ {lm.name} ({lm.params})
                        </option>
                      {/each}
                    </optgroup>
                    {#if uninstalledLlmModels.length > 0}
                      <optgroup label="Доступны для загрузки">
                        {#each uninstalledLlmModels as lm}
                          <option value={lm.filename}>
                            {lm.name} ({lm.params}) (Не скачана)
                          </option>
                        {/each}
                      </optgroup>
                    {/if}
                  {:else}
                    {#each llmModels as lm}
                      <option value={lm.filename}>
                        {lm.exists ? '✓ ' : ''}{lm.name} ({lm.params}){lm.exists ? ' (Установлена)' : ' (Не скачана)'}
                      </option>
                    {/each}
                  {/if}
                </select>
              </div>

              <!-- Stage 2: Final 2nd verification & repair loop -->
              <div class="flex flex-col gap-1">
                <label for="text_model_stage2" class="label-mono text-[10px] text-emerald-400">
                  2. Проверка смысла и целостности клипов
                </label>
                <select
                  id="text_model_stage2"
                  value={$pipelineStore.config.verifier_model_name || $pipelineStore.config.text_model_name}
                  on:change={handleVerifierModelChange}
                  disabled={$pipelineStore.status === 'running'}
                  class="w-full bg-[#0c0c0e] border border-emerald-500/30 rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-emerald-400 focus:outline-none"
                >
                  {#if installedLlmModels.length > 0}
                    <optgroup label="Установленные модели">
                      {#each installedLlmModels as lm}
                        <option value={lm.filename}>
                          ✓ {lm.name} ({lm.params})
                        </option>
                      {/each}
                    </optgroup>
                    {#if uninstalledLlmModels.length > 0}
                      <optgroup label="Доступны для загрузки">
                        {#each uninstalledLlmModels as lm}
                          <option value={lm.filename}>
                            {lm.name} ({lm.params}) (Не скачана)
                          </option>
                        {/each}
                      </optgroup>
                    {/if}
                  {:else}
                    {#each llmModels as lm}
                      <option value={lm.filename}>
                        {lm.exists ? '✓ ' : ''}{lm.name} ({lm.params}){lm.exists ? ' (Установлена)' : ' (Не скачана)'}
                      </option>
                    {/each}
                  {/if}
                </select>
              </div>

              <!-- Stage 3: Final SMM Copywriter -->
              <div class="flex flex-col gap-1">
                <label for="text_model_stage3" class="label-mono text-[10px] text-amber-300">
                  3. Названия, описания и хештеги для публикации
                </label>
                <select
                  id="text_model_stage3"
                  value={$pipelineStore.config.smm_model_name || $pipelineStore.config.verifier_model_name || $pipelineStore.config.text_model_name}
                  on:change={handleSmmModelChange}
                  disabled={$pipelineStore.status === 'running'}
                  class="w-full bg-[#0c0c0e] border border-amber-500/35 rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-amber-400 focus:outline-none"
                >
                  {#if installedLlmModels.length > 0}
                    <optgroup label="Установленные модели">
                      {#each installedLlmModels as lm}
                        <option value={lm.filename}>
                          ✓ {lm.name} ({lm.params})
                        </option>
                      {/each}
                    </optgroup>
                    {#if uninstalledLlmModels.length > 0}
                      <optgroup label="Доступны для загрузки">
                        {#each uninstalledLlmModels as lm}
                          <option value={lm.filename}>
                            {lm.name} ({lm.params}) (Не скачана)
                          </option>
                        {/each}
                      </optgroup>
                    {/if}
                  {:else}
                    {#each llmModels as lm}
                      <option value={lm.filename}>
                        {lm.exists ? '✓ ' : ''}{lm.name} ({lm.params}){lm.exists ? ' (Установлена)' : ' (Не скачана)'}
                      </option>
                    {/each}
                  {/if}
                </select>
              </div>

              <div class="text-[11px] font-sans text-[rgba(226,226,224,0.6)] leading-relaxed bg-[#0c0c0e]/80 border border-[rgba(226,226,224,0.1)] rounded-[2px] px-2.5 py-2 space-y-0.5">
                <div>• <span class="text-[#9aa4ff]">Отбор:</span> находит самые эмоциональные и динамичные сцены.</div>
                <div>• <span class="text-emerald-400">Проверка:</span> следит, чтобы фразы не обрывались и момент имел смысл.</div>
                <div>• <span class="text-amber-300">Описания:</span> подготавливает готовый заголовок, текст с иронией и хештеги рядом с видео.</div>
              </div>
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
                <label for="ollama_model" class="label-mono">1. Модель отбора сцен</label>
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
              <div class="flex flex-col gap-1">
                <label for="ollama_verifier_model" class="label-mono text-emerald-400">2. Модель проверки смысла</label>
                <input
                  id="ollama_verifier_model"
                  type="text"
                  placeholder="По умолчанию = модель 1"
                  value={$pipelineStore.config.verifier_model_name || $pipelineStore.config.text_model_name}
                  on:input={handleVerifierModelChange}
                  disabled={$pipelineStore.status === 'running'}
                  class="w-full bg-[#0c0c0e] border border-emerald-500/30 rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-emerald-400 focus:outline-none"
                />
              </div>
              <div class="flex flex-col gap-1">
                <label for="ollama_smm_model" class="label-mono text-amber-300">3. Модель заголовков и описаний</label>
                <input
                  id="ollama_smm_model"
                  type="text"
                  placeholder="По умолчанию = модель 2"
                  value={$pipelineStore.config.smm_model_name || $pipelineStore.config.verifier_model_name || $pipelineStore.config.text_model_name}
                  on:input={handleSmmModelChange}
                  disabled={$pipelineStore.status === 'running'}
                  class="w-full bg-[#0c0c0e] border border-amber-500/35 rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-amber-400 focus:outline-none"
                />
              </div>
            </div>
          {:else if $pipelineStore.config.text_engine === 'openai'}
            <!-- OpenAI-Compatible API Settings & Local Multi-Key Manager -->
            <div class="p-3 bg-[#16161a]/80 border border-[#5865f2]/35 rounded-[3px] flex flex-col gap-2.5">
              <div class="flex items-center justify-between">
                <span class="label-mono text-[#5865f2]">Подключение внешнего API</span>
                {#if saveFeedback}
                  <span class="font-mono text-[10px] text-[#23c55e]">{saveFeedback}</span>
                {/if}
              </div>

              <!-- Saved Profiles Selector -->
              <div class="flex flex-col gap-1">
                <div class="flex items-center justify-between">
                  <label for="openai_profile_select" class="label-mono">Сохранённые профили</label>
                  <div class="flex items-center gap-2">
                    <button
                      type="button"
                      on:click={handleStartNewProfile}
                      disabled={$pipelineStore.status === 'running'}
                      class="font-mono text-[10px] text-[#5865f2] hover:underline cursor-pointer"
                    >
                      + Добавить профиль
                    </button>
                    {#if selectedProfileId && selectedProfileId !== '__new__' && savedOpenAiProfiles.length > 0}
                      <button
                        type="button"
                        on:click={handleDeleteOpenAiProfile}
                        disabled={$pipelineStore.status === 'running'}
                        class="font-mono text-[10px] text-[#ef4444] hover:underline cursor-pointer"
                      >
                        Удалить
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
                  <label for="openai_api_key" class="label-mono">API-ключ</label>
                  <button
                    type="button"
                    on:click={() => (showApiKey = !showApiKey)}
                    class="font-mono text-[10px] text-[rgba(226,226,224,0.6)] hover:text-[#e2e2e4] cursor-pointer"
                  >
                    {showApiKey ? 'Скрыть' : 'Показать'}
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
                  <label for="openai_model" class="label-mono">Модель</label>
                  <button
                    type="button"
                    on:click={handleFetchModelsFromServer}
                    disabled={$pipelineStore.status === 'running' || isFetchingModels}
                    class="font-mono text-[10px] px-2 py-0.5 bg-[#5865f2]/20 hover:bg-[#5865f2]/35 border border-[#5865f2]/60 text-[#e2e2e4] rounded-[2px] cursor-pointer transition-colors disabled:opacity-40 flex items-center gap-1 whitespace-nowrap"
                  >
                    {#if isFetchingModels}
                      <span class="inline-block h-2.5 w-2.5 border border-white/30 border-t-white rounded-full animate-spin"></span>
                      <span>Обновление...</span>
                    {:else}
                      <span>Обновить список</span>
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
                    {isFetchingModels ? '...' : 'Список'}
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
                  ? 'Обновить профиль'
                  : 'Сохранить профиль'}
              </button>
            </div>
          {/if}

          <!-- Vision model -->
          <div class="flex flex-col gap-1">
            <label for="vision_model" class="label-mono">Визуальный анализ кадров (опционально)</label>
            <select
              id="vision_model"
              value={$pipelineStore.config.vision_model_path || ''}
              on:change={handleVisionModelChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            >
              <option value="">Автоматический (по движению и звуку)</option>
              {#each visionModels as vm}
                <option value={joinRelativePath('models', vm.filename)}>
                  {vm.exists ? '✓ ' : ''}{vm.name} ({vm.params}){vm.exists ? ' (Установлена)' : ' (Не скачана)'}
                </option>
              {/each}
            </select>
          </div>

          <!-- Output dir -->
          <div class="flex flex-col gap-1">
            <label for="output_dir" class="label-mono">Папка для готовых видео</label>
            <input 
              id="output_dir"
              type="text" 
              value={$pipelineStore.config.output_dir}
              on:input={handleOutputDirChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.15)] rounded-[2px] px-2.5 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] focus:outline-none"
            />
          </div>

          <!-- Uniform Timeline Distribution Toggle -->
          <label class="flex items-start gap-3 p-3 border border-[rgba(226,226,224,0.12)] rounded-[3px] cursor-pointer hover:border-[rgba(226,226,224,0.22)] bg-[#16161a]/70 transition-colors select-none">
            <input
              type="checkbox"
              checked={$pipelineStore.config.distribute_evenly ?? true}
              on:change={handleDistributeEvenlyChange}
              disabled={$pipelineStore.status === 'running'}
              class="w-4 h-4 mt-0.5 rounded-[2px] bg-[#0c0c0e] border-[rgba(226,226,224,0.3)] text-[#5865f2] focus:ring-0 focus:ring-offset-0 cursor-pointer accent-[#5865f2] shrink-0"
            />
            <div>
              <div class="label-mono text-[#e2e2e4] normal-case tracking-normal text-xs font-medium">Равномерно по всему видео</div>
              <div class="text-[11px] text-[rgba(226,226,224,0.5)] mt-0.5 font-sans leading-snug">
                Распределяет клипы по всему хронометражу длинных роликов, подкастов и серий (отключите для строгого топа лучших сцен)
              </div>
            </div>
          </label>
        </div>
      {/if}
    </div>
  </div>

  <!-- Action Buttons Stack -->
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
        <span>Начать нарезку</span>
      {/if}
    </button>

    <button 
      on:click={exportClips}
      disabled={!$pipelineStore.video || $pipelineStore.status === 'running' || ($pipelineStore.config.mode === 'auto' && $pipelineStore.status !== 'completed')}
      class="font-mono p-4 border-none uppercase text-xs font-bold tracking-[0.1em] cursor-pointer transition-opacity rounded-[2px] bg-[#23c55e] text-white disabled:opacity-30 disabled:cursor-not-allowed flex items-center justify-center gap-2"
    >
      <span>Сохранить клипы</span>
    </button>
  </div>
</div>

<BumperModal show={showBumperModal} on:close={() => (showBumperModal = false)} />

