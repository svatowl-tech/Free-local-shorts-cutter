<!-- src/lib/components/ResultsList.svelte -->
<script lang="ts">
  import { pipelineStore } from '../stores/pipeline';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { FolderInput, FolderOpen, ArrowLeft, Play, Pause, X, Copy, Check } from '@lucide/svelte';

  const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;

  let activePreviewClip: string | null = null;
  let activePreviewSrc: string = '';
  let folderStatusMessage: string | null = null;
  let copiedFolderPath = false;

  $: defaultExportFolder = (() => {
    const firstClip = $pipelineStore.clips[0];
    if (firstClip) {
      const lastSlash = Math.max(firstClip.lastIndexOf('/'), firstClip.lastIndexOf('\\'));
      if (lastSlash > 0) {
        return firstClip.substring(0, lastSlash);
      }
    }
    return $pipelineStore.config.output_dir || 'Готовые клипы';
  })();

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
      <div class="label-mono text-[#23c55e] mb-1 font-bold">ОПЕРАЦИЯ УСПЕШНО ЗАВЕРШЕНА</div>
      <p class="text-xs text-[#e2e2e4]">Все фрагменты нарезаны и сохранены в портретном формате 9:16. Нажмите на любой ролик ниже для предпросмотра.</p>
    </div>
    <div class="flex items-center gap-2 shrink-0">
      <button
        type="button"
        on:click={(e) => revealInFolder($pipelineStore.clips[0] || defaultExportFolder, e)}
        class="btn py-2 px-3.5 bg-[#23c55e] hover:bg-[#1ea950] text-black font-mono text-xs font-bold rounded-[2px] flex items-center gap-2 cursor-pointer transition-colors shadow"
      >
        <FolderOpen class="h-4 w-4" />
        <span>Открыть папку с роликами</span>
      </button>
    </div>
  </div>

  <!-- Информационная панель директории сохранения и уведомление об открытии -->
  <div class="p-3.5 bg-[#0c0c0e] border border-[rgba(226,226,224,0.14)] rounded-[4px] flex flex-col sm:flex-row sm:items-center justify-between gap-3">
    <div class="min-w-0 flex-1">
      <div class="text-[10px] font-mono uppercase text-[rgba(226,226,224,0.45)]">
        Директория сохранения готовых клипов:
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
    {@const activeName = activePreviewClip.split(/[/\\]/).pop() || 'clip.mp4'}
    <div class="p-4 bg-[#0c0c0e] border border-[#5865f2]/60 rounded-[4px] flex flex-col gap-3 shadow-xl">
      <div class="flex items-center justify-between border-b border-[rgba(226,226,224,0.1)] pb-2.5">
        <div class="flex items-center gap-2 min-w-0">
          <span class="px-2 py-0.5 rounded bg-[#23c55e]/20 text-[#23c55e] font-mono text-[10px] font-bold uppercase">
            Превью 9:16
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

  <!-- Clips List -->
  <div class="space-y-3">
    <div class="label-mono text-[#e2e2e4]">
      Экспортированные клипы ({$pipelineStore.clips.length})
    </div>

    <div class="space-y-2">
      {#each $pipelineStore.clips as clip, index}
        {@const clipName = clip.split(/[/\\]/).pop() || `clip_${index + 1}.mp4`}
        {@const isPreviewing = activePreviewClip === clip}
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
          on:click={() => toggleClipPreview(clip)}
          class="p-4 bg-[#0c0c0e] border {isPreviewing ? 'border-[#23c55e] bg-[#141816]' : 'border-[rgba(226,226,224,0.1)] hover:border-[#5865f2]'} rounded-[4px] flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4 transition-colors cursor-pointer"
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
              <div class="flex items-center gap-2">
                <h4 class="text-xs font-bold text-[#e2e2e4] truncate font-mono" title={clipName}>
                  {clipName}
                </h4>
                <span class="text-[10px] font-mono px-1.5 py-0.5 rounded {isPreviewing ? 'bg-[#23c55e]/20 text-[#23c55e]' : 'bg-[#5865f2]/15 text-[#5865f2]'}">
                  {isPreviewing ? 'Воспроизводится' : 'Предпросмотр'}
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


