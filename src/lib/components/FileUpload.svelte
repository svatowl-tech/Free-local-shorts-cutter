<!-- src/lib/components/FileUpload.svelte -->
<script lang="ts">
  import { pipelineStore } from '../stores/pipeline';
  import { HardDrive, Trash2 } from '@lucide/svelte';

  let dragOver = false;
  let fileInput: HTMLInputElement;

  function formatBytes(bytes: number, decimals = 2) {
    if (bytes === 0) return '0 Байт';
    const k = 1024;
    const dm = decimals < 0 ? 0 : decimals;
    const sizes = ['Байт', 'КБ', 'МБ', 'ГБ', 'ТБ'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
  }

  function handleFileSelected(event: Event) {
    const target = event.target as HTMLInputElement;
    const files = target.files;
    if (files && files.length > 0) {
      const file = files[0];
      pipelineStore.setVideo({
        name: file.name,
        path: file.webkitRelativePath || `/local/user/videos/${file.name}`,
        size: file.size,
        duration: 180
      });
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
            extensions: ['mp4', 'avi', 'mkv', 'mov', 'webm']
          }]
        });

        if (selected && typeof selected === 'string') {
          const fileName = selected.split(/[/\\]/).pop() || 'selected_video.mp4';
          pipelineStore.setVideo({
            name: fileName,
            path: selected,
            size: 345220000,
            duration: 180
          });
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
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) {
      const file = files[0];
      if (file.type.startsWith('video/') || /\.(mp4|mov|mkv|avi|webm)$/i.test(file.name)) {
        pipelineStore.setVideo({
          name: file.name,
          path: file.webkitRelativePath || `/drag-drop/videos/${file.name}`,
          size: file.size,
          duration: 180
        });
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
      <p class="text-xs text-[rgba(226,226,224,0.5)] max-w-[280px] leading-relaxed">
        Поддерживаются MP4, MOV, MKV, AVI, WEBM. Обработка локально.
      </p>
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
            <span>• Длительность: ~3 мин</span>
          </div>
          <p class="text-[10px] text-[rgba(226,226,224,0.4)] truncate font-mono mt-1.5 p-1 bg-[#16161a] border border-[rgba(226,226,224,0.08)] rounded-[2px]">
            {$pipelineStore.video.path}
          </p>
        </div>
      </div>

      <button 
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
