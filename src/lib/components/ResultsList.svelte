<!-- src/lib/components/ResultsList.svelte -->
<script lang="ts">
  import { pipelineStore } from '../stores/pipeline';
  import { FolderInput, ArrowLeft, Play } from '@lucide/svelte';

  async function revealInFolder(filePath: string) {
    try {
      const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;
      if (isTauri) {
        const { open } = await import('@tauri-apps/plugin-shell');
        const parentDir = filePath.substring(0, filePath.lastIndexOf('/')) || filePath.substring(0, filePath.lastIndexOf('\\')) || '.';
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

<div class="panel p-6 sm:p-8 flex flex-col gap-6">
  <!-- Status Banner -->
  <div class="p-4 bg-[#23c55e]/10 border-l-[3px] border-[#23c55e] rounded-[2px] flex items-center justify-between">
    <div>
      <div class="label-mono text-[#23c55e] mb-1 font-bold">ОПЕРАЦИЯ УСПЕШНО ЗАВЕРШЕНА</div>
      <p class="text-xs text-[#e2e2e4]">Все фрагменты нарезаны и сохранены в портретном формате 9:16.</p>
    </div>
    <span class="label-mono font-mono text-[#23c55e] bg-[#23c55e]/20 px-2 py-1 rounded-[2px]">
      100% SUCCESS
    </span>
  </div>

  <!-- Clips List -->
  <div class="space-y-3">
    <div class="label-mono text-[#e2e2e4]">
      Экспортированные клипы ({$pipelineStore.clips.length})
    </div>

    <div class="space-y-2">
      {#each $pipelineStore.clips as clip, index}
        {@const clipName = clip.split(/[/\\]/).pop() || `clip_${index + 1}.mp4`}
        <div class="p-4 bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] rounded-[4px] flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4 hover:border-[#5865f2] transition-colors">
          <div class="flex items-center gap-3.5 min-w-0">
            <div class="w-10 h-10 bg-[#16161a] border border-[rgba(226,226,224,0.1)] rounded-[2px] flex items-center justify-center text-[#5865f2] shrink-0">
              <Play class="h-4 w-4 fill-[#5865f2]" />
            </div>
            
            <div class="min-w-0">
              <h4 class="text-xs font-bold text-[#e2e2e4] truncate font-mono" title={clipName}>
                {clipName}
              </h4>
              <p class="text-[10px] text-[rgba(226,226,224,0.4)] font-mono mt-0.5 truncate">
                {clip}
              </p>
            </div>
          </div>

          <button 
            on:click={() => revealInFolder(clip)}
            class="btn py-2 px-3 border border-[rgba(226,226,224,0.15)] bg-transparent hover:bg-[#5865f2]/10 hover:border-[#5865f2] text-xs font-mono text-[rgba(226,226,224,0.7)] hover:text-[#e2e2e4] rounded-[2px] flex items-center gap-2 cursor-pointer transition-colors"
          >
            <FolderInput class="h-3.5 w-3.5 text-[#5865f2]" />
            <span>Открыть папку</span>
          </button>
        </div>
      {/each}
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
