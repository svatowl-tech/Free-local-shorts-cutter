<!-- src/lib/components/FileUpload.svelte -->
<script lang="ts">
  import { pipelineStore } from '../stores/pipeline';
  import { Video, UploadCloud, FileVideo, HardDrive, Trash2 } from '@lucide/svelte';

  let dragOver = false;
  let fileInput: HTMLInputElement;

  // Функция форматирования байтов
  function formatBytes(bytes: number, decimals = 2) {
    if (bytes === 0) return '0 Байт';
    const k = 1024;
    const dm = decimals < 0 ? 0 : decimals;
    const sizes = ['Байт', 'КБ', 'МБ', 'ГБ', 'ТБ'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
  }

  // Обработка выбора файла через HTML5 Input (браузер / резервный вариант)
  function handleFileSelected(event: Event) {
    const target = event.target as HTMLInputElement;
    const files = target.files;
    if (files && files.length > 0) {
      const file = files[0];
      pipelineStore.setVideo({
        name: file.name,
        path: file.webkitRelativePath || `/local/user/videos/${file.name}`,
        size: file.size,
        duration: 180 // по умолчанию для демонстрации
      });
    }
  }

  // Вызов нативного Tauri диалога (для десктопа)
  async function chooseVideoTauri() {
    try {
      const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;
      if (isTauri) {
        // Динамический импорт для предотвращения ошибок сборки в обычном браузере
        const { open } = await import('@tauri-apps/plugin-dialog');
        const selected = await open({
          multiple: false,
          filters: [{
            name: 'Video',
            extensions: ['mp4', 'avi', 'mkv', 'mov', 'webm']
          }]
        });

        if (selected && typeof selected === 'string') {
          // Имитируем получение размера и имени для нативного файла
          const fileName = selected.split(/[/\\]/).pop() || 'selected_video.mp4';
          pipelineStore.setVideo({
            name: fileName,
            path: selected,
            size: 345220000, // Специфический размер для демонстрации
            duration: 180
          });
        }
      } else {
        // Клик на скрытый инпут, если в браузере
        fileInput.click();
      }
    } catch (err) {
      console.error('Ошибка выбора файла Tauri, переключаемся на стандартный диалог:', err);
      fileInput.click();
    }
  }

  // Drag and drop handlers
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
      if (file.type.startsWith('video/')) {
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

  function clearSelected() {
    pipelineStore.setVideo(null);
  }
</script>

<div class="bg-slate-900/60 border border-slate-800 rounded-2xl p-5 shadow-xl backdrop-blur-md">
  <div class="flex items-center gap-2 mb-4">
    <Video class="h-5 w-5 text-indigo-400" />
    <h2 class="text-sm font-bold tracking-widest text-slate-200 uppercase font-mono">Выбор видеоматериала</h2>
  </div>

  <input 
    type="file" 
    accept="video/*" 
    class="hidden" 
    bind:this={fileInput} 
    on:change={handleFileSelected}
  />

  {#if !$pipelineStore.video}
    <!-- Область загрузки (Drag and Drop) -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <div 
      on:dragover={handleDragOver}
      on:dragleave={handleDragLeave}
      on:drop={handleDrop}
      on:click={chooseVideoTauri}
      class="group border-2 border-dashed rounded-xl p-8 text-center cursor-pointer transition-all duration-300 flex flex-col items-center justify-center min-h-[220px] {
        dragOver 
          ? 'border-indigo-400 bg-indigo-950/20 shadow-[0_0_15px_rgba(99,102,241,0.15)]' 
          : 'border-slate-800 hover:border-indigo-500/50 hover:bg-slate-950/40'
      }"
    >
      <div class="p-4 bg-slate-950 border border-slate-800 rounded-2xl mb-4 group-hover:scale-105 transition-transform duration-300">
        <UploadCloud class="h-8 w-8 text-indigo-400 group-hover:text-indigo-300" />
      </div>
      <p class="text-xs font-semibold text-slate-300 mb-1.5 font-mono">
        ПЕРЕТАЩИТЕ ВИДЕО СЮДА ИЛИ НАЖМИТЕ
      </p>
      <p class="text-[11px] text-slate-500 max-w-xs leading-relaxed font-sans">
        Поддерживаются MP4, MOV, MKV, AVI, WEBM. Файл обрабатывается локально без отправки на сервер.
      </p>
    </div>
  {:else}
    <!-- Панель выбранного видео -->
    <div class="bg-slate-950/80 border border-slate-800/80 rounded-xl p-4 flex flex-col md:flex-row items-start md:items-center justify-between gap-4">
      <div class="flex items-start gap-3.5 flex-1 min-w-0">
        <div class="p-3 bg-indigo-950/40 border border-indigo-900/40 rounded-xl text-indigo-400">
          <FileVideo class="h-6 w-6" />
        </div>
        <div class="flex-1 min-w-0">
          <h3 class="text-xs font-bold text-slate-200 truncate font-mono mb-1 leading-tight" title={$pipelineStore.video.name}>
            {$pipelineStore.video.name}
          </h3>
          <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-slate-500 font-mono">
            <span class="flex items-center gap-1">
              <HardDrive class="h-3 w-3" />
              {formatBytes($pipelineStore.video.size)}
            </span>
            <span>• Длительность: ~3 мин</span>
          </div>
          <p class="text-[10px] text-slate-600 truncate font-mono mt-1 w-full bg-slate-950/40 px-1.5 py-0.5 rounded border border-slate-900">
            Путь: {$pipelineStore.video.path}
          </p>
        </div>
      </div>

      <button 
        on:click={clearSelected}
        class="p-2 border border-slate-800 text-slate-400 hover:text-red-400 hover:border-red-950 rounded-xl transition-all hover:bg-red-950/10 cursor-pointer self-stretch md:self-auto flex items-center justify-center"
        title="Сбросить выбор"
      >
        <Trash2 class="h-4 w-4" />
      </button>
    </div>
  {/if}
</div>
