<!-- src/lib/components/ResultsList.svelte -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { pipelineStore } from '../stores/pipeline';
  import { fragmentsStore } from '../stores/fragments';
  import {
    getProjectSmmMetadata,
    regenerateClipSmm,
    buildDemoSmmItem,
    type SmmClipMetadata
  } from '../ipc';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { isTauriRuntime, getFileName, getParentDir, formatErrorMessage } from '../utils/path';
  import { FolderInput, FolderOpen, ArrowLeft, Play, Pause, X, Copy, Check, RefreshCw, FileText } from '@lucide/svelte';

  const isTauri = isTauriRuntime();

  let activePreviewClip: string | null = null;
  let activePreviewSrc: string = '';
  let folderStatusMessage: string | null = null;
  let copiedFolderPath = false;
  let copiedAllSmm = false;
  let copiedFieldMap: Record<string, boolean> = {};
  let regeneratingMap: Record<string, boolean> = {};

  onMount(async () => {
    if (isTauri && $pipelineStore.video && $pipelineStore.smmMetadata.length === 0) {
      try {
        const diskSmm = await getProjectSmmMetadata($pipelineStore.video.path, $pipelineStore.config);
        if (Array.isArray(diskSmm) && diskSmm.length > 0) {
          for (const item of diskSmm) {
            pipelineStore.updateSmmItem(item);
          }
        }
      } catch (e) {
        console.warn('Не удалось подгрузить SMM-метаданные проекта:', e);
      }
    }
  });

  $: defaultExportFolder = (() => {
    const firstClip = $pipelineStore.clips[0];
    if (firstClip) {
      const parent = getParentDir(firstClip);
      if (parent) {
        return parent;
      }
    }
    return $pipelineStore.config.output_dir || 'Готовые клипы';
  })();

  function getSmmForClip(clipPath: string, index: number): SmmClipMetadata {
    const normalizedTarget = clipPath.replace(/\\/g, '/');
    const targetName = getFileName(clipPath, `clip_${index + 1}.mp4`);
    const found = $pipelineStore.smmMetadata.find(
      m =>
        m.clip_path === clipPath ||
        m.clip_path.replace(/\\/g, '/') === normalizedTarget ||
        m.clip_filename === targetName
    );
    if (found) return found;

    const frag = $fragmentsStore[index] || { start: index * 15, end: (index + 1) * 15 };
    const modelName =
      $pipelineStore.config.smm_model_name ||
      $pipelineStore.config.verifier_model_name ||
      $pipelineStore.config.text_model_name;
    const videoTitle = $pipelineStore.video?.name || 'Видео';
    return buildDemoSmmItem(clipPath, videoTitle, index, frag.start, frag.end, modelName);
  }

  function formatFullSmmText(smm: SmmClipMetadata): string {
    return `${smm.title}\n\n${smm.description}\n\n${smm.hashtags}`;
  }

  async function copyTextWithFeedback(key: string, text: string, e?: MouseEvent) {
    e?.stopPropagation();
    if (typeof navigator !== 'undefined' && navigator.clipboard) {
      await navigator.clipboard.writeText(text);
      copiedFieldMap = { ...copiedFieldMap, [key]: true };
      setTimeout(() => {
        copiedFieldMap = { ...copiedFieldMap, [key]: false };
      }, 2000);
    }
  }

  async function copyAllClipsSmm() {
    if (typeof navigator === 'undefined' || !navigator.clipboard) return;
    const blocks = $pipelineStore.clips.map((clip, idx) => {
      const smm = getSmmForClip(clip, idx);
      return `=== [${smm.clip_type.toUpperCase()}] ${smm.clip_filename} ===\nНазвание: ${smm.title}\n\nОписание:\n${smm.description}\n\nХештеги:\n${smm.hashtags}\nФайл .txt: ${smm.txt_file_path}`;
    });
    await navigator.clipboard.writeText(blocks.join('\n\n----------------------------------------\n\n'));
    copiedAllSmm = true;
    setTimeout(() => {
      copiedAllSmm = false;
    }, 2200);
  }

  async function handleRegenerateSmm(clipPath: string, index: number, e?: MouseEvent) {
    e?.stopPropagation();
    if (regeneratingMap[clipPath]) return;
    regeneratingMap = { ...regeneratingMap, [clipPath]: true };
    try {
      const frag = $fragmentsStore[index] || { start: index * 15, end: (index + 1) * 15 };
      const videoPath = $pipelineStore.video?.path || $pipelineStore.video?.name || 'Видео';
      const updated = await regenerateClipSmm(
        videoPath,
        clipPath,
        index,
        frag.start,
        frag.end,
        $pipelineStore.config
      );
      pipelineStore.updateSmmItem(updated);
      pipelineStore.addLog(
        `✍️ SMM-описание обновлено и перезаписано в ${getFileName(updated.txt_file_path, 'clip.txt')}`,
        'LLM',
        'info'
      );
    } catch (err: unknown) {
      console.error('Ошибка перегенерации SMM:', formatErrorMessage(err));
    } finally {
      regeneratingMap = { ...regeneratingMap, [clipPath]: false };
    }
  }

  function handleSmmFieldEdit(
    clipPath: string,
    index: number,
    field: 'title' | 'description' | 'hashtags',
    value: string
  ) {
    const current = getSmmForClip(clipPath, index);
    pipelineStore.updateSmmItem({
      ...current,
      [field]: value
    });
  }

  function toggleClipPreview(clipPath: string) {
    if (activePreviewClip === clipPath) {
      activePreviewClip = null;
      activePreviewSrc = '';
      return;
    }
    activePreviewClip = clipPath;
    if (isTauri) {
      try {
        activePreviewSrc = convertFileSrc(clipPath);
      } catch (err) {
        console.error('Не удалось сконвертировать путь для предпросмотра клипа:', err);
        activePreviewSrc = clipPath;
      }
    } else {
      activePreviewSrc = clipPath;
    }
  }

  async function revealInFolder(targetPath: string, e?: MouseEvent) {
    e?.stopPropagation();
    const resolvedDir = await pipelineStore.openFolder(targetPath);
    folderStatusMessage = isTauri
      ? `Папка открыта в проводнике: ${resolvedDir}`
      : `Путь к папке готовых роликов: ${resolvedDir}`;
  }

  async function copyExportFolderPath(e?: MouseEvent) {
    e?.stopPropagation();
    if (typeof navigator !== 'undefined' && navigator.clipboard) {
      await navigator.clipboard.writeText(defaultExportFolder);
      copiedFolderPath = true;
      setTimeout(() => {
        copiedFolderPath = false;
      }, 2000);
    }
  }

  function getClipCategoryInfo(clipPath: string): { label: string; badgeClass: string } {
    const normalized = clipPath.replace(/\\/g, '/');
    if (normalized.includes('/Хайлайты/') || normalized.includes('Хайлайты_')) {
      return {
        label: 'Хайлайты 16:9',
        badgeClass: 'bg-amber-500/20 text-amber-300 border border-amber-500/40'
      };
    }
    if (normalized.includes('/Ток-видео/') || normalized.includes('Разговорное_видео') || normalized.includes('Демка_') || normalized.includes('Ток_видео')) {
      return {
        label: normalized.includes('Демка_') ? 'Демка 16:9' : 'Ток-видео 16:9',
        badgeClass: 'bg-emerald-500/20 text-emerald-300 border border-emerald-500/40'
      };
    }
    return {
      label: $pipelineStore.config.vertical_format ? 'Шортс 9:16' : 'Шортс 16:9',
      badgeClass: 'bg-[#5865f2]/20 text-[#9aa4ff] border border-[#5865f2]/40'
    };
  }

  function handleReset() {
    activePreviewClip = null;
    activePreviewSrc = '';
    folderStatusMessage = null;
    pipelineStore.reset();
  }
</script>

<div class="panel p-6 sm:p-8 flex flex-col gap-6">
  <!-- Status Banner -->
  <div class="p-4 bg-[#23c55e]/10 border-l-[3px] border-[#23c55e] rounded-[2px] flex flex-col sm:flex-row sm:items-center justify-between gap-3">
    <div>
      <div class="label-mono text-[#23c55e] mb-1 font-bold">ОПЕРАЦИЯ И SMM-КОПИРАЙТИНГ УСПЕШНО ЗАВЕРШЕНЫ</div>
      <p class="text-xs text-[#e2e2e4]">
        Все ролики и готовые текстовые файлы описаний (<span class="font-mono text-amber-300">.txt</span> с названием, ироничным описанием и хештегами) разложены по папкам (<span class="font-mono">Шортсы</span>, <span class="font-mono">Хайлайты</span>, <span class="font-mono">Ток-видео</span>).
      </p>
    </div>
    <div class="flex flex-wrap items-center gap-2 shrink-0">
      <button
        type="button"
        on:click={copyAllClipsSmm}
        class="btn py-2 px-3 border border-amber-500/50 bg-amber-500/15 hover:bg-amber-500/30 text-amber-200 font-mono text-xs font-bold rounded-[2px] flex items-center gap-1.5 cursor-pointer transition-colors"
        title="Скопировать названия, описания и хештеги сразу всех роликов"
      >
        {#if copiedAllSmm}
          <Check class="h-4 w-4 text-[#23c55e]" />
          <span class="text-[#23c55e]">Все SMM-тексты скопированы!</span>
        {:else}
          <Copy class="h-4 w-4 text-amber-300" />
          <span>Скопировать SMM всех роликов</span>
        {/if}
      </button>

      <button
        type="button"
        on:click={(e) => revealInFolder($pipelineStore.clips[0] || defaultExportFolder, e)}
        class="btn py-2 px-3.5 bg-[#23c55e] hover:bg-[#1ea950] text-black font-mono text-xs font-bold rounded-[2px] flex items-center gap-2 cursor-pointer transition-colors shadow"
      >
        <FolderOpen class="h-4 w-4" />
        <span>Открыть папку с роликами и .txt</span>
      </button>
    </div>
  </div>

  <!-- Информационная панель директории сохранения и уведомление об открытии -->
  <div class="p-3.5 bg-[#0c0c0e] border border-[rgba(226,226,224,0.14)] rounded-[4px] flex flex-col sm:flex-row sm:items-center justify-between gap-3">
    <div class="min-w-0 flex-1">
      <div class="text-[10px] font-mono uppercase text-[rgba(226,226,224,0.45)]">
        Директория сохранения готовых клипов и .txt файлов SMM:
      </div>
      <div class="text-xs font-mono text-[#e2e2e4] truncate mt-0.5 select-all" title={defaultExportFolder}>
        {defaultExportFolder}
      </div>
      {#if folderStatusMessage}
        <div class="text-[11px] font-mono text-[#23c55e] mt-1">
          ✓ {folderStatusMessage}
        </div>
      {/if}
    </div>

    <div class="flex items-center gap-2 shrink-0">
      <button
        type="button"
        on:click={copyExportFolderPath}
        class="btn py-1.5 px-3 border border-[rgba(226,226,224,0.15)] bg-[#16161a] hover:bg-[#202026] text-xs font-mono text-[#e2e2e4] rounded-[2px] flex items-center gap-1.5 cursor-pointer transition-colors"
        title="Скопировать полный путь к папке в буфер обмена"
      >
        {#if copiedFolderPath}
          <Check class="h-3.5 w-3.5 text-[#23c55e]" />
          <span class="text-[#23c55e]">Путь скопирован</span>
        {:else}
          <Copy class="h-3.5 w-3.5 text-[#5865f2]" />
          <span>Скопировать путь</span>
        {/if}
      </button>

      <button
        type="button"
        on:click={(e) => revealInFolder($pipelineStore.clips[0] || defaultExportFolder, e)}
        class="btn py-1.5 px-3 border border-[#5865f2]/40 bg-[#5865f2]/15 hover:bg-[#5865f2] text-xs font-mono text-[#e2e2e4] hover:text-white rounded-[2px] flex items-center gap-1.5 cursor-pointer transition-colors"
      >
        <FolderInput class="h-3.5 w-3.5" />
        <span>Открыть папку</span>
      </button>
    </div>
  </div>

  <!-- Встроенный плеер предпросмотра выбранного готового ролика -->
  {#if activePreviewClip && activePreviewSrc}
    {@const activeName = getFileName(activePreviewClip, 'clip.mp4')}
    {@const activeCategory = getClipCategoryInfo(activePreviewClip)}
    <div class="p-4 bg-[#0c0c0e] border border-[#5865f2]/60 rounded-[4px] flex flex-col gap-3 shadow-xl">
      <div class="flex items-center justify-between border-b border-[rgba(226,226,224,0.1)] pb-2.5">
        <div class="flex items-center gap-2 min-w-0">
          <span class="px-2 py-0.5 rounded font-mono text-[10px] font-bold uppercase {activeCategory.badgeClass}">
            Превью • {activeCategory.label}
          </span>
          <span class="font-mono text-xs font-bold text-[#e2e2e4] truncate" title={activeName}>
            {activeName}
          </span>
        </div>
        <button
          type="button"
          on:click={() => {
            activePreviewClip = null;
            activePreviewSrc = '';
          }}
          class="p-1 rounded bg-[#16161a] hover:bg-red-500/20 text-[rgba(226,226,224,0.6)] hover:text-red-400 transition-colors"
          title="Закрыть предпросмотр"
        >
          <X class="w-4 h-4" />
        </button>
      </div>

      <div class="w-full max-h-[480px] bg-black rounded-[3px] overflow-hidden flex items-center justify-center">
        <!-- svelte-ignore a11y-media-has-caption -->
        <video
          src={activePreviewSrc}
          controls
          autoplay
          class="max-h-[460px] w-auto object-contain"
        >
          Ваш браузер не поддерживает воспроизведение видео.
        </video>
      </div>
    </div>
  {/if}

  <!-- Clips & SMM Cards List -->
  <div class="space-y-3">
    <div class="flex items-center justify-between">
      <div class="label-mono text-[#e2e2e4]">
        Экспортированные ролики и готовые SMM-описания ({$pipelineStore.clips.length})
      </div>
      <div class="text-[11px] font-mono text-amber-300/90">
        ✍️ Название + Описание с иронией + Хештеги (.txt в папке с видео)
      </div>
    </div>

    <div class="space-y-3.5">
      {#each $pipelineStore.clips as clip, index}
        {@const clipName = getFileName(clip, `clip_${index + 1}.mp4`)}
        {@const isPreviewing = activePreviewClip === clip}
        {@const clipCat = getClipCategoryInfo(clip)}
        {@const smm = getSmmForClip(clip, index)}
        {@const txtFileName = getFileName(smm.txt_file_path, clipName.replace(/\.mp4$/i, '.txt'))}
        {@const isRegenerating = !!regeneratingMap[clip]}

        <div
          class="bg-[#0c0c0e] border {isPreviewing ? 'border-[#23c55e] bg-[#141816]' : 'border-[rgba(226,226,224,0.12)]'} rounded-[4px] overflow-hidden transition-colors"
        >
          <!-- Top Row: Video Info & Preview Controls -->
          <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
          <div
            on:click={() => toggleClipPreview(clip)}
            class="p-4 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4 cursor-pointer hover:bg-[#16161a]/60 transition-colors"
            title="Нажмите для воспроизведения предпросмотра ролика"
          >
            <div class="flex items-center gap-3.5 min-w-0">
              <div class="w-10 h-10 {isPreviewing ? 'bg-[#23c55e] text-black' : 'bg-[#16161a] text-[#5865f2]'} border border-[rgba(226,226,224,0.1)] rounded-[2px] flex items-center justify-center shrink-0 transition-colors">
                {#if isPreviewing}
                  <Pause class="h-4 w-4" />
                {:else}
                  <Play class="h-4 w-4 fill-current" />
                {/if}
              </div>

              <div class="min-w-0">
                <div class="flex items-center gap-2 flex-wrap">
                  <h4 class="text-xs font-bold text-[#e2e2e4] truncate font-mono" title={clipName}>
                    {clipName}
                  </h4>
                  <span class="text-[10px] font-mono px-1.5 py-0.5 rounded {clipCat.badgeClass}">
                    {clipCat.label}
                  </span>
                  {#if $fragmentsStore[index]?.repaired || ($fragmentsStore[index]?.title && $fragmentsStore[index].title.includes('Доработан'))}
                    <span
                      class="text-[10px] font-mono px-1.5 py-0.5 rounded bg-emerald-500/20 text-emerald-300 border border-emerald-500/40 font-bold"
                      title={$fragmentsStore[index]?.repair_note || 'Клип автоматически доработан на 2-м круге ревизии ИИ'}
                    >
                      ✓ Доработан (2 круг)
                    </span>
                  {/if}
                  <span
                    class="text-[10px] font-mono px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-300 border border-amber-500/30 flex items-center gap-1"
                    title={`Текстовый файл с описанием сохранен рядом с видео: ${smm.txt_file_path}`}
                  >
                    <FileText class="w-3 h-3" />
                    <span>{txtFileName}</span>
                  </span>
                </div>
                <p class="text-[10px] text-[rgba(226,226,224,0.4)] font-mono mt-0.5 truncate">
                  {clip}
                </p>
              </div>
            </div>

            <div class="flex items-center gap-2 shrink-0">
              <button
                type="button"
                on:click|stopPropagation={() => toggleClipPreview(clip)}
                class="btn py-2 px-3 border {isPreviewing ? 'border-[#23c55e] bg-[#23c55e]/15 text-[#23c55e]' : 'border-[#5865f2]/40 bg-[#5865f2]/10 hover:bg-[#5865f2] text-[#e2e2e4] hover:text-white'} text-xs font-mono rounded-[2px] flex items-center gap-1.5 cursor-pointer transition-colors"
              >
                {#if isPreviewing}
                  <Pause class="h-3.5 w-3.5" />
                  <span>Скрыть превью</span>
                {:else}
                  <Play class="h-3.5 w-3.5" />
                  <span>Смотреть</span>
                {/if}
              </button>

              <button
                type="button"
                on:click={(e) => revealInFolder(clip, e)}
                class="btn py-2 px-3 border border-[rgba(226,226,224,0.15)] bg-transparent hover:bg-[#5865f2]/10 hover:border-[#5865f2] text-xs font-mono text-[rgba(226,226,224,0.7)] hover:text-[#e2e2e4] rounded-[2px] flex items-center gap-2 cursor-pointer transition-colors"
              >
                <FolderInput class="h-3.5 w-3.5 text-[#5865f2]" />
                <span>Открыть папку</span>
              </button>
            </div>
          </div>

          <!-- SMM Package Block: Title, Ironic Description, Hashtags -->
          <div class="p-4 bg-[#121216] border-t border-[rgba(226,226,224,0.09)] flex flex-col gap-3">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <div class="flex items-center gap-2 flex-wrap">
                <span class="font-mono text-[11px] font-bold text-amber-300">
                  ✍️ SMM-описание для выкладки
                </span>
                <span class="font-mono text-[10px] px-1.5 py-0.5 rounded bg-[#1c1c22] border border-[rgba(226,226,224,0.12)] text-[rgba(226,226,224,0.65)]">
                  Модель: {smm.model_used}
                </span>
              </div>

              <div class="flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  on:click={(e) => handleRegenerateSmm(clip, index, e)}
                  disabled={isRegenerating}
                  class="px-2.5 py-1 bg-[#1c1c22] hover:bg-[#272730] border border-[rgba(226,226,224,0.18)] text-[11px] font-mono text-[#e2e2e4] rounded-[2px] flex items-center gap-1.5 cursor-pointer transition-colors disabled:opacity-40"
                  title="Сгенерировать новый вариант названия, ироничной шутки и хештегов и обновить .txt файл"
                >
                  <RefreshCw class="w-3 h-3 text-amber-300 {isRegenerating ? 'animate-spin' : ''}" />
                  <span>{isRegenerating ? 'Генерация...' : 'Другой вариант шутки / SMM'}</span>
                </button>

                <button
                  type="button"
                  on:click={(e) => copyTextWithFeedback(`all_${index}`, formatFullSmmText(smm), e)}
                  class="px-3 py-1 bg-amber-500/20 hover:bg-amber-500/35 border border-amber-500/50 text-amber-200 font-mono text-[11px] font-bold rounded-[2px] flex items-center gap-1.5 cursor-pointer transition-colors"
                  title="Скопировать сразу Название + Описание + Хештеги для вставки при публикации"
                >
                  {#if copiedFieldMap[`all_${index}`]}
                    <Check class="w-3.5 h-3.5 text-[#23c55e]" />
                    <span class="text-[#23c55e]">Скопировано всё!</span>
                  {:else}
                    <Copy class="w-3.5 h-3.5 text-amber-300" />
                    <span>Скопировать всё для выкладки</span>
                  {/if}
                </button>
              </div>
            </div>

            <!-- 1. Title -->
            <div class="flex flex-col gap-1">
              <div class="flex items-center justify-between">
                <span class="font-mono text-[10px] uppercase text-[rgba(226,226,224,0.5)]">
                  1. Название ролика
                </span>
                <button
                  type="button"
                  on:click={(e) => copyTextWithFeedback(`title_${index}`, smm.title, e)}
                  class="font-mono text-[10px] text-[#9aa4ff] hover:underline flex items-center gap-1 cursor-pointer"
                >
                  {#if copiedFieldMap[`title_${index}`]}
                    <span class="text-[#23c55e]">✓ Название скопировано</span>
                  {:else}
                    <span>[Копировать название]</span>
                  {/if}
                </button>
              </div>
              <input
                type="text"
                value={smm.title}
                on:input={(e) => handleSmmFieldEdit(clip, index, 'title', (e.target as HTMLInputElement).value)}
                class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.14)] focus:border-amber-400 rounded-[2px] px-2.5 py-1.5 text-xs font-mono font-semibold text-[#e2e2e4] focus:outline-none"
              />
            </div>

            <!-- 2. Description (with context & irony) -->
            <div class="flex flex-col gap-1">
              <div class="flex items-center justify-between">
                <span class="font-mono text-[10px] uppercase text-[rgba(226,226,224,0.5)]">
                  2. Описание (исходник + суть внутри + контекст + ироничная шутка)
                </span>
                <button
                  type="button"
                  on:click={(e) => copyTextWithFeedback(`desc_${index}`, smm.description, e)}
                  class="font-mono text-[10px] text-[#9aa4ff] hover:underline flex items-center gap-1 cursor-pointer"
                >
                  {#if copiedFieldMap[`desc_${index}`]}
                    <span class="text-[#23c55e]">✓ Описание скопировано</span>
                  {:else}
                    <span>[Копировать описание]</span>
                  {/if}
                </button>
              </div>
              <textarea
                rows="3"
                value={smm.description}
                on:input={(e) => handleSmmFieldEdit(clip, index, 'description', (e.target as HTMLTextAreaElement).value)}
                class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.14)] focus:border-amber-400 rounded-[2px] px-2.5 py-1.5 text-xs font-mono text-[#e2e2e4] leading-relaxed resize-y focus:outline-none"
              ></textarea>
            </div>

            <!-- 3. Hashtags -->
            <div class="flex flex-col gap-1">
              <div class="flex items-center justify-between">
                <span class="font-mono text-[10px] uppercase text-[rgba(226,226,224,0.5)]">
                  3. Хештеги
                </span>
                <button
                  type="button"
                  on:click={(e) => copyTextWithFeedback(`tags_${index}`, smm.hashtags, e)}
                  class="font-mono text-[10px] text-[#9aa4ff] hover:underline flex items-center gap-1 cursor-pointer"
                >
                  {#if copiedFieldMap[`tags_${index}`]}
                    <span class="text-[#23c55e]">✓ Хештеги скопированы</span>
                  {:else}
                    <span>[Копировать хештеги]</span>
                  {/if}
                </button>
              </div>
              <input
                type="text"
                value={smm.hashtags}
                on:input={(e) => handleSmmFieldEdit(clip, index, 'hashtags', (e.target as HTMLInputElement).value)}
                class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.14)] focus:border-amber-400 rounded-[2px] px-2.5 py-1.5 text-xs font-mono text-amber-300 focus:outline-none"
              />
            </div>

            <!-- Saved .txt path footer -->
            <div class="text-[10px] font-mono text-[rgba(226,226,224,0.45)] flex items-center justify-between pt-1 border-t border-[rgba(226,226,224,0.06)]">
              <span class="truncate" title={smm.txt_file_path}>
                📄 Экспортировано в текстовый файл: <span class="text-[rgba(226,226,224,0.75)]">{smm.txt_file_path}</span>
              </span>
            </div>
          </div>
        </div>
      {/each}
    </div>
  </div>

  <!-- Log Actions on Complete -->
  <div class="p-3 bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] rounded-[4px] flex flex-wrap items-center justify-between gap-3">
    <div class="text-xs font-mono text-[rgba(226,226,224,0.6)]">
      Журнал выполнения ({$pipelineStore.logs.length} записей)
    </div>
    <div class="flex items-center gap-2">
      <button 
        type="button"
        on:click={() => pipelineStore.copyLogsToClipboard()}
        class="btn py-1.5 px-3 border border-[rgba(226,226,224,0.15)] bg-[#16161a] hover:bg-[#202026] text-xs font-mono text-[#e2e2e4] rounded-[2px] flex items-center gap-1.5 cursor-pointer"
      >
        <span>📋</span>
        <span>Скопировать логи</span>
      </button>
      <button 
        type="button"
        on:click={() => pipelineStore.downloadLogsAsFile()}
        class="btn py-1.5 px-3 border border-[rgba(226,226,224,0.15)] bg-[#16161a] hover:bg-[#202026] text-xs font-mono text-[#e2e2e4] rounded-[2px] flex items-center gap-1.5 cursor-pointer"
      >
        <span>💾</span>
        <span>Сохранить в файл</span>
      </button>
    </div>
  </div>

  <!-- Reset Button -->
  <button 
    on:click={handleReset}
    class="font-mono p-4 border-none uppercase text-xs font-bold tracking-[0.1em] cursor-pointer transition-opacity rounded-[2px] bg-[#5865f2] text-white flex items-center justify-center gap-2"
  >
    <ArrowLeft class="h-4 w-4" />
    <span>Обработать еще одно видео</span>
  </button>
</div>
