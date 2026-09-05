<!-- src/lib/components/ResultsList.svelte -->
<script lang="ts">
  import { pipelineStore } from '../stores/pipeline';
  import { CheckCircle2, FolderInput, ExternalLink, ArrowLeft, MoreVertical, Play, Share2 } from '@lucide/svelte';

  // Функция открытия папки с клипами (в Tauri)
  async function revealInFolder(filePath: string) {
    try {
      const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;
      if (isTauri) {
        // Динамический импорт Tauri shell
        const { open } = await import('@tauri-apps/plugin-shell');
        
        // В зависимости от ОС открываем родительскую директорию
        const parentDir = filePath.substring(0, filePath.lastIndexOf('/')) || '.';
        await open(parentDir);
      } else {
        alert(`[Симулятор] Открываем директорию файла: ${filePath}`);
      }
    } catch (err) {
      console.error('Не удалось открыть локальную директорию:', err);
    }
  }

  function handleReset() {
    pipelineStore.reset();
  }
</script>

<div class="bg-slate-900/60 border border-slate-800 rounded-2xl p-5 shadow-xl backdrop-blur-md flex flex-col gap-5">
  <!-- Хедер успешного завершения -->
  <div class="flex items-center gap-3 bg-emerald-950/20 border border-emerald-900/30 rounded-xl p-4">
    <div class="p-2.5 bg-emerald-500/20 text-emerald-400 rounded-xl">
      <CheckCircle2 class="h-6 w-6" />
    </div>
    <div>
      <h2 class="text-sm font-bold text-emerald-400 font-mono uppercase tracking-wider">Пайплайн успешно завершен</h2>
      <p class="text-[11px] text-slate-400 mt-0.5">Все фрагменты успешно нарезаны и экспортированы в вертикальном формате с авто-кропом.</p>
    </div>
  </div>

  <!-- Список сгенерированных клипов -->
  <div class="space-y-3">
    <h3 class="text-[10px] font-bold font-mono text-slate-400 uppercase tracking-widest pl-1">
      Экспортированные клипы ({$pipelineStore.clips.length})
    </h3>

    <div class="space-y-2.5">
      {#each $pipelineStore.clips as clip, index}
        {@const clipName = clip.split(/[/\\]/).pop() || `clip_${index + 1}.mp4`}
        <div class="bg-slate-950/80 border border-slate-850 rounded-xl p-3.5 flex items-center justify-between gap-4 hover:border-indigo-500/30 transition-all duration-300">
          <div class="flex items-center gap-3 min-w-0">
            <div class="relative group cursor-pointer">
              <!-- Иконка видеоплеера с кнопкой воспроизведения -->
              <div class="h-10 w-10 bg-slate-900 border border-slate-800 rounded-lg flex items-center justify-center text-indigo-400 group-hover:scale-105 transition-transform">
                <Play class="h-4 w-4 fill-indigo-400" />
              </div>
            </div>
            
            <div class="min-w-0">
              <h4 class="text-xs font-bold text-slate-200 truncate font-mono" title={clipName}>
                {clipName}
              </h4>
              <p class="text-[10px] text-slate-500 font-mono mt-0.5 max-w-md truncate">
                Путь: {clip}
              </p>
            </div>
          </div>

          <div class="flex items-center gap-2">
            <!-- Кнопка Открыть в папке -->
            <button 
              on:click={() => revealInFolder(clip)}
              class="px-2.5 py-1.5 cursor-pointer border border-slate-800 hover:border-indigo-900/50 hover:bg-indigo-950/20 text-[11px] font-medium font-mono text-slate-400 hover:text-indigo-300 rounded-lg transition-colors flex items-center gap-1.5"
              title="Открыть содержащую папку в проводнике"
            >
              <FolderInput class="h-3.5 w-3.5" />
              <span class="hidden sm:inline">Открыть в папке</span>
            </button>
          </div>
        </div>
      {/each}
    </div>
  </div>

  <!-- Кнопки управления -->
  <div class="flex gap-3 mt-2">
    <button 
      on:click={handleReset}
      class="flex-1 cursor-pointer bg-slate-950 hover:bg-slate-925 text-slate-300 border border-slate-850 hover:border-slate-805 font-semibold py-2.5 px-4 rounded-xl transition duration-200 text-xs tracking-wider flex items-center justify-center gap-1.5"
    >
      <ArrowLeft class="h-3.5 w-3.5" />
      ОБРАБОТАТЬ ЕЩЕ ОДНО ВИДЕО
    </button>
  </div>
</div>
