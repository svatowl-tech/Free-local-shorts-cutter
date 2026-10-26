<!-- src/lib/components/FileUpload.svelte -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { pipelineStore } from '../stores/pipeline';
  import { getVideoMetadata } from '../ipc';
  import { HardDrive, Trash2 } from '@lucide/svelte';

  let dragOver = false;
  let fileInput: HTMLInputElement;
  let unlistenDragDrop: (() => void) | null = null;

  function formatBytes(bytes: number, decimals = 2) {
    if (!bytes || bytes === 0) return 'Размер определяется...';
    const k = 1024;
    const dm = decimals < 0 ? 0 : decimals;
    const sizes = ['Байт', 'КБ', 'МБ', 'ГБ', 'ТБ'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
  }

  async function selectPathFromTauri(selectedPath: string) {
    const fileName = selectedPath.split(/[/\\]/).pop() || 'selected_video.mp4';

    // Мгновенно устанавливаем выбранный файл в стор (без блокировки UI)
    pipelineStore.setVideo({
      name: fileName,
      path: selectedPath,
      size: 0,
      duration: 0
    });

    // Асинхронно запрашиваем размер и длительность через FFmpeg
    try {
      const meta = await getVideoMetadata(selectedPath);
      if (meta) {
        pipelineStore.updateVideoMetadata({
          size: meta.size || 0,
          duration: meta.duration || 0
        });
      }
    } catch (e) {
      console.warn('FFmpeg метаданные пропущены:', e);
    }
  }

  onMount(async () => {
    const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;
    if (isTauri) {
      try {
        const { getCurrentWebview } = await import('@tauri-apps/api/webview');
        unlistenDragDrop = await getCurrentWebview().onDragDropEvent(async (event) => {
          const payload = event.payload as any;
          if (payload.type === 'over' || payload.type === 'enter') {
            dragOver = true;
          } else if (payload.type === 'leave' || payload.type === 'cancel') {
            dragOver = false;
          } else if (payload.type === 'drop' && Array.isArray(payload.paths) && payload.paths.length > 0) {
            dragOver = false;
            const droppedPath = payload.paths[0];
            if (/\.(mp4|mov|mkv|avi|webm|ts|m4v)$/i.test(droppedPath)) {
              await selectPathFromTauri(droppedPath);
            } else {
              pipelineStore.updateError('Пожалуйста, перетащите видеофайл поддерживаемого формата (MP4, MKV, MOV, AVI, WEBM)');
            }
          }
        });
      } catch (e) {
        console.warn('Не удалось подписаться на нативный Tauri drag-drop:', e);
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
      const file = files[0];
      const objectUrl = URL.createObjectURL(file);
      const nativePath = (file as any).path;
      const filePath = nativePath || file.name;

      pipelineStore.setVideo({
        name: file.name,
        path: filePath,
        size: file.size,
        duration: 0,
        objectUrl
      });

      if (nativePath) {
        getVideoMetadata(nativePath).then(meta => {
          if (meta && meta.duration > 0) {
            pipelineStore.updateVideoMetadata({
              duration: meta.duration,
              size: meta.size || file.size
            });
          }
        }).catch(() => {});
      }
    }
  }

  async function chooseVideoTauri() {
    try {
      const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;
      if (isTauri) {
        const { open } = await import('@tauri-apps/plugin-dialog');
        const selected = await open({
          multiple: false,
          filters: [{
            name: 'Video',
            extensions: ['mp4', 'avi', 'mkv', 'mov', 'webm', 'ts', 'm4v']
          }]
        });

        if (selected && typeof selected === 'string') {
          await selectPathFromTauri(selected);
        }
      } else {
        fileInput?.click();
      }
    } catch (err) {
      console.error('Ошибка выбора файла Tauri, переключаемся на стандартный диалог:', err);
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

  function handleDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) {
      const file = files[0];
      const nativePath = (file as any).path;
      // В Tauri v2 путь приходит через onDragDropEvent. Если его нет в DOM File, не перезаписываем полный путь коротким именем
      if (isTauri && !nativePath) {
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
          getVideoMetadata(nativePath).then(meta => {
            if (meta && meta.duration > 0) {
              pipelineStore.updateVideoMetadata({
                duration: meta.duration,
                size: meta.size || file.size
              });
            }
          }).catch(() => {});
        }
      } else {
        pipelineStore.updateError('Пожалуйста, перетащите видеофайл проверенных форматов (MP4, MKV, MOV)');
      }
    }
  }

  function clearSelected(e: MouseEvent) {
    e.stopPropagation();
    pipelineStore.setVideo(null);
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

  {#if !$pipelineStore.video}
    <!-- Dropzone according to Variation 2 -->
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
          <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-[rgba(226,226,224,0.5)] font-mono">
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
  {/if}
</div>
