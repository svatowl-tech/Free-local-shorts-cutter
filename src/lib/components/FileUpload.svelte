<!-- src/lib/components/FileUpload.svelte -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { pipelineStore } from '../stores/pipeline';
  import {
    getVideoMetadata,
    parseCustomSubtitles,
    parseCustomSubtitlesContent
  } from '../ipc';
  import { isTauriRuntime, getFileName, formatErrorMessage } from '../utils/path';
  import { HardDrive, Trash2, Users, Search, Sparkles } from '@lucide/svelte';

  interface FileWithNativePath extends File {
    path?: string;
  }

  interface TauriDragDropPayload {
    type?: string;
    paths?: string[];
  }

  let dragOver = false;
  let fileInput: HTMLInputElement;
  let subtitlesInput: HTMLInputElement;
  let isParsingSubtitles = false;
  let characterSearchQuery = '';
  let unlistenDragDrop: (() => void) | null = null;

  $: customSubtitlesParseResult = $pipelineStore.customSubtitles;
  $: filteredCharacters = (customSubtitlesParseResult?.characters || []).filter((c) =>
    characterSearchQuery.trim()
      ? c.name.toLowerCase().includes(characterSearchQuery.trim().toLowerCase()) ||
        (c.sample_text || '').toLowerCase().includes(characterSearchQuery.trim().toLowerCase())
      : true
  );

  function formatBytes(bytes: number, decimals = 2) {
    if (!bytes || bytes === 0) return 'Размер определяется...';
    const k = 1024;
    const dm = decimals < 0 ? 0 : decimals;
    const sizes = ['Байт', 'КБ', 'МБ', 'ГБ', 'ТБ'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
  }

  function formatDurationSec(sec?: number): string {
    if (!sec || sec <= 0) return '0с';
    if (sec < 60) return `${Math.round(sec)}с`;
    const m = Math.floor(sec / 60);
    const s = Math.round(sec % 60);
    return `${m}м ${s}с`;
  }

  async function selectPathFromTauri(selectedPath: string) {
    const fileName = getFileName(selectedPath, 'selected_video.mp4');

    pipelineStore.setVideo({
      name: fileName,
      path: selectedPath,
      size: 0,
      duration: 0
    });

    try {
      const meta = await getVideoMetadata(selectedPath);
      if (meta) {
        pipelineStore.updateVideoMetadata({
          size: meta.size || 0,
          duration: meta.duration || 0
        });
      }
    } catch (e: unknown) {
      console.warn('FFmpeg метаданные пропущены:', formatErrorMessage(e));
    }
  }

  onMount(async () => {
    if (isTauriRuntime()) {
      try {
        const { getCurrentWebview } = await import('@tauri-apps/api/webview');
        unlistenDragDrop = await getCurrentWebview().onDragDropEvent(async (event) => {
          const payload = event.payload as TauriDragDropPayload;
          if (payload.type === 'over' || payload.type === 'enter') {
            dragOver = true;
          } else if (payload.type === 'leave' || payload.type === 'cancel') {
            dragOver = false;
          } else if (payload.type === 'drop' && Array.isArray(payload.paths) && payload.paths.length > 0) {
            dragOver = false;
            const droppedPath = payload.paths[0];
            if (/\.(mp4|mov|mkv|avi|webm|ts|m4v)$/i.test(droppedPath)) {
              await selectPathFromTauri(droppedPath);
            } else if (/\.(ass|ssa|srt|vtt|sub)$/i.test(droppedPath)) {
              await processSubtitlePath(droppedPath);
            } else {
              pipelineStore.updateError('Поддерживаются видео (MP4, MKV, MOV, AVI, WEBM) и субтитры (ASS, SSA, SRT, VTT, SUB)');
            }
          }
        });
      } catch (e: unknown) {
        console.warn('Не удалось подписаться на нативный Tauri drag-drop:', formatErrorMessage(e));
      }
    }
  });

  onDestroy(() => {
    if (unlistenDragDrop) {
      unlistenDragDrop();
      unlistenDragDrop = null;
    }
  });

  function handleFileSelected(event: Event) {
    const target = event.target as HTMLInputElement;
    const files = target.files;
    if (files && files.length > 0) {
      const file = files[0] as FileWithNativePath;
      const objectUrl = URL.createObjectURL(file);
      const nativePath = file.path;
      const filePath = nativePath || file.name;

      pipelineStore.setVideo({
        name: file.name,
        path: filePath,
        size: file.size,
        duration: 0,
        objectUrl
      });

      if (nativePath) {
        getVideoMetadata(nativePath)
          .then((meta) => {
            if (meta && meta.duration > 0) {
              pipelineStore.updateVideoMetadata({
                duration: meta.duration,
                size: meta.size || file.size
              });
            }
          })
          .catch((err: unknown) => {
            console.warn('Не удалось получить метаданные видео:', formatErrorMessage(err));
          });
      }
    }
  }

  async function chooseVideoTauri() {
    try {
      if (isTauriRuntime()) {
        const { open } = await import('@tauri-apps/plugin-dialog');
        const selected = await open({
          multiple: false,
          filters: [
            {
              name: 'Video',
              extensions: ['mp4', 'avi', 'mkv', 'mov', 'webm', 'ts', 'm4v']
            }
          ]
        });

        if (selected && typeof selected === 'string') {
          await selectPathFromTauri(selected);
        }
      } else {
        fileInput?.click();
      }
    } catch (err: unknown) {
      console.error('Ошибка выбора файла Tauri, переключаемся на стандартный диалог:', formatErrorMessage(err));
      fileInput?.click();
    }
  }

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    dragOver = true;
  }

  function handleDragLeave() {
    dragOver = false;
  }

  async function handleDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    const isTauri = isTauriRuntime();
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) {
      const file = files[0] as FileWithNativePath;
      const nativePath = file.path;
      if (isTauri && !nativePath) {
        return;
      }
      if (/\.(ass|ssa|srt|vtt|sub)$/i.test(file.name)) {
        if (nativePath && isTauri) {
          await processSubtitlePath(nativePath);
        } else {
          const text = await file.text();
          await processSubtitleContent(file.name, text);
        }
        return;
      }
      if (file.type.startsWith('video/') || /\.(mp4|mov|mkv|avi|webm|ts|m4v)$/i.test(file.name)) {
        const objectUrl = URL.createObjectURL(file);
        const filePath = nativePath || file.name;
        pipelineStore.setVideo({
          name: file.name,
          path: filePath,
          size: file.size,
          duration: 0,
          objectUrl
        });

        if (nativePath) {
          getVideoMetadata(nativePath)
            .then((meta) => {
              if (meta && meta.duration > 0) {
                pipelineStore.updateVideoMetadata({
                  duration: meta.duration,
                  size: meta.size || file.size
                });
              }
            })
            .catch((err: unknown) => {
              console.warn('Не удалось получить метаданные видео после drop:', formatErrorMessage(err));
            });
        }
      } else {
        pipelineStore.updateError('Пожалуйста, перетащите видеофайл (MP4, MKV, MOV) или файл субтитров (ASS, SRT, VTT)');
      }
    }
  }

  function clearSelected(e: MouseEvent) {
    e.stopPropagation();
    pipelineStore.setVideo(null);
    clearSubtitles();
  }

  async function chooseSubtitlesTauri() {
    try {
      if (isTauriRuntime()) {
        const { open } = await import('@tauri-apps/plugin-dialog');
        const selected = await open({
          multiple: false,
          filters: [
            {
              name: 'Subtitles (ASS, SSA, SRT, VTT, SUB)',
              extensions: ['ass', 'ssa', 'srt', 'vtt', 'sub']
            }
          ]
        });

        if (selected && typeof selected === 'string') {
          await processSubtitlePath(selected);
        }
      } else {
        subtitlesInput?.click();
      }
    } catch (err: unknown) {
      console.error('Ошибка выбора субтитров:', formatErrorMessage(err));
      subtitlesInput?.click();
    }
  }

  async function handleSubtitleFileSelected(event: Event) {
    const target = event.target as HTMLInputElement;
    const files = target.files;
    if (files && files.length > 0) {
      const file = files[0] as FileWithNativePath;
      const nativePath = file.path;
      if (nativePath && isTauriRuntime()) {
        await processSubtitlePath(nativePath);
      } else {
        const text = await file.text();
        await processSubtitleContent(file.name, text);
      }
      target.value = '';
    }
  }

  async function processSubtitlePath(path: string) {
    isParsingSubtitles = true;
    try {
      const res = await parseCustomSubtitles(path);
      pipelineStore.setCustomSubtitles(res);
      const charNames = res.characters.map((c) => c.name);
      pipelineStore.updateConfig({
        custom_subtitles_path: res.file_path || path,
        selected_characters: charNames
      });
      pipelineStore.addLog(
        `✓ Подгружены готовые субтитры: '${getFileName(path, 'subtitles.ass')}'. Найдено реплик: ${res.total_lines}, Персонажей: ${res.characters.length}`
      );
    } catch (err: unknown) {
      pipelineStore.updateError(`Ошибка подгрузки субтитров: ${formatErrorMessage(err)}`);
    } finally {
      isParsingSubtitles = false;
    }
  }

  async function processSubtitleContent(fileName: string, content: string) {
    isParsingSubtitles = true;
    try {
      const res = await parseCustomSubtitlesContent(fileName, content);
      pipelineStore.setCustomSubtitles(res);
      const charNames = res.characters.map((c) => c.name);
      pipelineStore.updateConfig({
        custom_subtitles_path: res.file_path || fileName,
        selected_characters: charNames
      });
      pipelineStore.addLog(
        `✓ Подгружены готовые субтитры: '${fileName}'. Найдено реплик: ${res.total_lines}, Персонажей: ${res.characters.length}`
      );
    } catch (err: unknown) {
      pipelineStore.updateError(`Ошибка разбора субтитров: ${formatErrorMessage(err)}`);
    } finally {
      isParsingSubtitles = false;
    }
  }

  function clearSubtitles() {
    characterSearchQuery = '';
    pipelineStore.setCustomSubtitles(null);
    pipelineStore.updateConfig({
      custom_subtitles_path: null,
      selected_characters: []
    });
  }

  function toggleCharacter(name: string) {
    let current = $pipelineStore.config.selected_characters || [];
    if (current.includes(name)) {
      current = current.filter((c) => c !== name);
    } else {
      current = [...current, name];
    }
    pipelineStore.updateConfig({ selected_characters: current });
  }

  function selectAllCharacters() {
    const all = (customSubtitlesParseResult?.characters || []).map((c) => c.name);
    pipelineStore.updateConfig({ selected_characters: all });
  }

  function clearAllCharacters() {
    pipelineStore.updateConfig({ selected_characters: [] });
  }

  function activateCharacterDemoPreset(charName: string) {
    pipelineStore.updateConfig({
      selected_characters: [charName],
      render_talk_video: true
    });
    pipelineStore.addLog(`🎙 Выбран режим сборки персонажной демки для "${charName}" (Ток-видео + Шортсы)`);
  }
</script>

<div class="panel p-6 sm:p-8 flex-1 flex flex-col">
  <div class="panel-title flex items-center gap-2 mb-4">
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <path d="m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5"/>
      <rect x="2" y="6" width="14" height="12" rx="2"/>
    </svg>
    <span>Выбор видеоматериала</span>
  </div>

  <input 
    type="file" 
    accept="video/*" 
    class="hidden" 
    style="display: none;"
    bind:this={fileInput} 
    on:change={handleFileSelected}
  />

  <input 
    type="file" 
    accept=".ass,.ssa,.srt,.vtt,.sub" 
    class="hidden" 
    style="display: none;"
    bind:this={subtitlesInput} 
    on:change={handleSubtitleFileSelected}
  />

  {#if !$pipelineStore.video}
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <div 
      on:dragover={handleDragOver}
      on:dragleave={handleDragLeave}
      on:drop={handleDrop}
      on:click={chooseVideoTauri}
      class="flex-1 min-h-[240px] border-[1.5px] border-dashed rounded-[4px] p-8 flex flex-col items-center justify-center text-center transition-all duration-200 cursor-pointer select-none {
        dragOver 
          ? 'border-[#5865f2] bg-[#5865f2]/5' 
          : 'border-[rgba(226,226,224,0.15)] hover:border-[#5865f2] hover:bg-[#1a1a20]'
      }"
    >
      <div class="mb-4 text-[#e2e2e4] opacity-50 transition-opacity group-hover:opacity-80">
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M12 13v8M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242M8 17l4-4 4 4"/>
        </svg>
      </div>
      <h2 class="text-base font-semibold tracking-tight text-[#e2e2e4] mb-1.5 uppercase font-syne">ПЕРЕТАЩИТЕ ВИДЕО СЮДА</h2>
      <p class="text-xs text-[rgba(226,226,224,0.5)] max-w-[280px] leading-relaxed mb-4">
        Поддерживаются MP4, MOV, MKV, AVI, WEBM. Обработка локально.
      </p>

      <div class="label-mono px-3 py-1.5 bg-[#16161a] border border-[rgba(226,226,224,0.1)] rounded text-[rgba(226,226,224,0.7)] text-xs flex items-center gap-1.5">
        <span>📁</span>
        <span>Нажмите или перетащите файл для монтажа</span>
      </div>
    </div>
  {:else}
    <!-- Selected video card -->
    <div class="border border-[rgba(226,226,224,0.15)] bg-[#0c0c0e] p-5 rounded-[4px] flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
      <div class="flex items-start gap-4 flex-1 min-w-0">
        <div class="w-10 h-10 bg-[#5865f2] rounded-[4px] flex items-center justify-center text-white shrink-0">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="2" y="6" width="14" height="12" rx="2"/>
            <path d="m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5"/>
          </svg>
        </div>
        <div class="flex-1 min-w-0">
          <div class="label-mono mb-0.5">ВЫБРАН ФАЙЛ</div>
          <h3 class="text-sm font-semibold text-[#e2e2e4] truncate font-mono mb-1" title={$pipelineStore.video.name}>
            {$pipelineStore.video.name}
          </h3>
          <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-[rgba(226,226,224,0.5)] font-mono tabular-nums">
            <span class="flex items-center gap-1">
              <HardDrive class="h-3 w-3" />
              {formatBytes($pipelineStore.video.size)}
            </span>
            {#if $pipelineStore.video.duration && $pipelineStore.video.duration > 0}
              <span>• Длительность: {Math.round($pipelineStore.video.duration)} сек</span>
            {/if}
          </div>
          <p class="text-[10px] text-[rgba(226,226,224,0.4)] truncate font-mono mt-1.5 p-1 bg-[#16161a] border border-[rgba(226,226,224,0.08)] rounded-[2px]">
            {$pipelineStore.video.path}
          </p>
        </div>
      </div>

      <button 
        type="button"
        on:click={clearSelected}
        class="btn border border-[rgba(226,226,224,0.15)] bg-transparent text-[rgba(226,226,224,0.6)] hover:text-red-400 hover:border-red-500/40 p-2.5 rounded-[4px] transition-colors cursor-pointer self-stretch sm:self-auto flex items-center justify-center gap-1.5"
        title="Сбросить выбор"
      >
        <Trash2 class="h-4 w-4" />
        <span class="text-[10px] sm:hidden">УДАЛИТЬ</span>
      </button>
    </div>

    <!-- Subtitle Upload & Character Selection Section right under video -->
    <div class="mt-4 border border-[rgba(226,226,224,0.12)] bg-[#0c0c0e]/90 p-4 rounded-[4px] flex flex-col gap-3">
      <div class="flex items-center justify-between gap-3">
        <div class="flex items-center gap-2">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="text-[#5865f2]">
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
            <path d="M8 10h8"/>
            <path d="M8 14h6"/>
          </svg>
          <span class="label-mono text-xs text-[#e2e2e4] font-medium normal-case">
            Загрузить субтитры (ASS / SSA / SRT / VTT / SUB)
          </span>
        </div>

        {#if $pipelineStore.config.custom_subtitles_path}
          <div class="flex items-center gap-2">
            <button 
              type="button"
              on:click={chooseSubtitlesTauri}
              disabled={$pipelineStore.status === 'running'}
              class="text-[10px] text-[#5865f2] hover:underline font-mono cursor-pointer"
            >
              Заменить файл
            </button>
            <span class="text-[rgba(226,226,224,0.25)]">|</span>
            <button 
              type="button"
              on:click={clearSubtitles}
              disabled={$pipelineStore.status === 'running'}
              class="text-[10px] text-red-400 hover:underline font-mono cursor-pointer"
            >
              Сбросить субтитры
            </button>
          </div>
        {/if}
      </div>

      {#if $pipelineStore.config.custom_subtitles_path}
        <div class="flex items-center justify-between bg-[#16161a] p-2.5 rounded-[3px] border border-[rgba(226,226,224,0.1)]">
          <div class="flex items-center gap-2 min-w-0">
            <span class="text-emerald-400 font-mono text-xs">✓</span>
            <div class="truncate text-xs font-mono text-[#e2e2e4]" title={$pipelineStore.config.custom_subtitles_path}>
              {getFileName($pipelineStore.config.custom_subtitles_path, 'subtitles.ass')}
            </div>
          </div>
          {#if customSubtitlesParseResult}
            <div class="text-[11px] text-[rgba(226,226,224,0.6)] font-mono tabular-nums shrink-0 ml-2">
              {customSubtitlesParseResult.total_lines} реплик · {customSubtitlesParseResult.characters.length} перс.
            </div>
          {/if}
        </div>
        <p class="text-[11px] text-emerald-400/90 font-mono">
          ⚡ Готовые субтитры подключены — сканирование речи через Whisper пропущено, таймкоды и персонажи загружены напрямую.
        </p>

        {#if customSubtitlesParseResult && customSubtitlesParseResult.characters.length > 0}
          <div class="mt-1 pt-3 border-t border-[rgba(226,226,224,0.1)] flex flex-col gap-2.5">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <div class="flex items-center gap-1.5 text-xs font-mono text-[#e2e2e4] font-semibold">
                <Users class="w-3.5 h-3.5 text-[#5865f2]" />
                <span>Персонажи / актёры в субтитрах ({($pipelineStore.config.selected_characters || []).length} из {customSubtitlesParseResult.characters.length})</span>
              </div>
              <div class="flex items-center gap-2 text-[10px] font-mono">
                <button
                  type="button"
                  on:click={selectAllCharacters}
                  disabled={$pipelineStore.status === 'running'}
                  class="text-[#5865f2] hover:underline cursor-pointer"
                >
                  Выбрать всех
                </button>
                <span class="text-[rgba(226,226,224,0.3)]">|</span>
                <button
                  type="button"
                  on:click={clearAllCharacters}
                  disabled={$pipelineStore.status === 'running'}
                  class="text-[rgba(226,226,224,0.55)] hover:underline cursor-pointer"
                >
                  Сбросить
                </button>
              </div>
            </div>

            {#if customSubtitlesParseResult.characters.length > 5}
              <div class="relative">
                <Search class="w-3.5 h-3.5 text-[rgba(226,226,224,0.4)] absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
                <input
                  type="text"
                  placeholder="Поиск персонажа или фразы..."
                  bind:value={characterSearchQuery}
                  class="w-full bg-[#16161a] border border-[rgba(226,226,224,0.14)] rounded-[3px] pl-8 pr-3 py-1.5 text-xs text-[#e2e2e4] font-mono focus:border-[#5865f2] outline-none"
                />
              </div>
            {/if}

            <div class="grid grid-cols-1 sm:grid-cols-2 gap-1.5 max-h-[190px] overflow-y-auto pr-1">
              {#each filteredCharacters as char}
                {@const isSelected = ($pipelineStore.config.selected_characters || []).includes(char.name)}
                <div
                  class="flex items-center justify-between gap-2 p-2 rounded-[3px] border transition-colors {
                    isSelected
                      ? 'bg-[#5865f2]/15 border-[#5865f2]/60 text-white'
                      : 'bg-[#16161a] border-[rgba(226,226,224,0.1)] text-[rgba(226,226,224,0.65)] hover:border-[rgba(226,226,224,0.25)]'
                  }"
                >
                  <button
                    type="button"
                    on:click={() => toggleCharacter(char.name)}
                    disabled={$pipelineStore.status === 'running'}
                    class="flex items-center gap-2 min-w-0 flex-1 text-left cursor-pointer"
                    title={char.sample_text ? `Пример реплики: «${char.sample_text}»` : char.name}
                  >
                    <span class="w-4 h-4 rounded-[2px] flex items-center justify-center text-[10px] font-mono shrink-0 {isSelected ? 'bg-[#5865f2] text-white' : 'bg-[#0c0c0e] border border-[rgba(226,226,224,0.2)] text-transparent'}">
                      ✓
                    </span>
                    <div class="min-w-0 flex-1">
                      <div class="text-xs font-mono font-medium truncate text-[#e2e2e4]">{char.name}</div>
                      <div class="text-[10px] font-mono text-[rgba(226,226,224,0.45)] tabular-nums truncate">
                        {char.line_count} репл. · ~{formatDurationSec(char.total_duration_sec)}
                      </div>
                    </div>
                  </button>

                  <button
                    type="button"
                    on:click={() => activateCharacterDemoPreset(char.name)}
                    disabled={$pipelineStore.status === 'running'}
                    class="px-2 py-1 rounded-[2px] bg-[#0c0c0e] hover:bg-[#5865f2] border border-[rgba(226,226,224,0.15)] hover:border-[#5865f2] text-[10px] font-mono text-[rgba(226,226,224,0.8)] hover:text-white shrink-0 transition-colors cursor-pointer flex items-center gap-1"
                    title="Выбрать только этого персонажа и включить сборку персонажной демки (Ток-видео)"
                  >
                    <Sparkles class="w-2.5 h-2.5" />
                    <span>Демка</span>
                  </button>
                </div>
              {/each}
            </div>

            <div class="text-[10px] text-[rgba(226,226,224,0.45)] font-mono leading-relaxed">
              💡 При выборе конкретных персонажей автоматически захватываются короткие вопросы собеседников (≤2.5с перед репликой) для сохранения живого контекста диалога.
            </div>
          </div>
        {/if}
      {:else}
        <button 
          type="button"
          on:click={chooseSubtitlesTauri}
          disabled={isParsingSubtitles || $pipelineStore.status === 'running'}
          class="btn border border-[#5865f2]/40 bg-[#5865f2]/10 hover:bg-[#5865f2]/20 text-[#e2e2e4] py-2.5 px-4 rounded-[4px] text-xs font-mono font-medium flex items-center justify-center gap-2 cursor-pointer transition-colors"
        >
          <span>💬</span>
          <span>{isParsingSubtitles ? 'Разбор файла субтитров и персонажей...' : 'Загрузить субтитры (ASS / SSA / SRT / VTT)'}</span>
        </button>
        <p class="text-[10px] text-[rgba(226,226,224,0.45)] font-mono">
          Позволяет мгновенно пропустить распознавание Whisper и собрать шортсы, хайлайты или актёрскую демку по конкретным персонажам из файла субтитров.
        </p>
      {/if}
    </div>
  {/if}
</div>
