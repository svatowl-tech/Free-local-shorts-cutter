<script lang="ts">
  import { createEventDispatcher, onMount } from 'svelte';
  import { fragmentsStore, addFragment, updateFragment, removeFragment } from '../stores/fragments';
  import type { Fragment } from '../stores/fragments';

  export let videoDuration: number = 0;
  export let currentTime: number = 0;

  const dispatch = createEventDispatcher();

  let timelineEl: HTMLElement;
  let timelineWidth: number = 0;

  $: scale = videoDuration > 0 && timelineWidth > 0 ? timelineWidth / videoDuration : 1;

  let isDragging = false;
  let isResizingLeft = false;
  let isResizingRight = false;
  let activeFragmentId: string | null = null;

  let dragStartX: number = 0;
  let initialFragStart: number = 0;
  let initialFragEnd: number = 0;

  onMount(() => {
    const observer = new ResizeObserver((entries) => {
      for (let entry of entries) {
        timelineWidth = entry.contentRect.width;
      }
    });
    if (timelineEl) {
      observer.observe(timelineEl);
      timelineWidth = timelineEl.clientWidth;
    }
    return () => observer.disconnect();
  });

  function getMouseTime(e: MouseEvent): number {
    if (!timelineEl) return 0;
    const rect = timelineEl.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const time = x / scale;
    return Math.max(0, Math.min(videoDuration, time));
  }

  function handleTimelineClick(e: MouseEvent) {
    if (isDragging || isResizingLeft || isResizingRight) return;
    
    const time = getMouseTime(e);
    dispatch('seek', { time });
    
    const start = time;
    const end = Math.min(videoDuration, time + 5);
    addFragment(start, end - start);
  }

  function handleFragmentClick(frag: Fragment, e: MouseEvent) {
    e?.stopPropagation();
    if (isDragging || isResizingLeft || isResizingRight) return;
    dispatch('seek', { time: frag.start });
  }

  function handleRemoveFragment(id: string, e: MouseEvent) {
    e?.stopPropagation();
    removeFragment(id);
  }

  function startDrag(frag: Fragment, e: MouseEvent) {
    e.stopPropagation();
    isDragging = true;
    activeFragmentId = frag.id;
    dragStartX = e.clientX;
    initialFragStart = frag.start;
    initialFragEnd = frag.end;

    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
  }

  function startResizeLeft(frag: Fragment, e: MouseEvent) {
    e.stopPropagation();
    isResizingLeft = true;
    activeFragmentId = frag.id;
    dragStartX = e.clientX;
    initialFragStart = frag.start;
    initialFragEnd = frag.end;

    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
  }

  function startResizeRight(frag: Fragment, e: MouseEvent) {
    e.stopPropagation();
    isResizingRight = true;
    activeFragmentId = frag.id;
    dragStartX = e.clientX;
    initialFragStart = frag.start;
    initialFragEnd = frag.end;

    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
  }

  function onMouseMove(e: MouseEvent) {
    if (!activeFragmentId) return;

    const deltaX = e.clientX - dragStartX;
    const deltaTime = deltaX / scale;
    const fragment = $fragmentsStore.find(f => f.id === activeFragmentId);
    if (!fragment) return;

    if (isDragging) {
      const duration = initialFragEnd - initialFragStart;
      let newStart = initialFragStart + deltaTime;
      let newEnd = initialFragEnd + deltaTime;

      if (newStart < 0) {
        newStart = 0;
        newEnd = duration;
      }
      if (newEnd > videoDuration) {
        newEnd = videoDuration;
        newStart = Math.max(0, videoDuration - duration);
      }
      updateFragment(activeFragmentId, { start: newStart, end: newEnd });
    } else if (isResizingLeft) {
      let newStart = Math.max(0, Math.min(initialFragStart + deltaTime, initialFragEnd - 0.5));
      updateFragment(activeFragmentId, { start: newStart });
    } else if (isResizingRight) {
      let newEnd = Math.min(videoDuration, Math.max(initialFragEnd + deltaTime, initialFragStart + 0.5));
      updateFragment(activeFragmentId, { end: newEnd });
    }
  }

  function onMouseUp() {
    isDragging = false;
    isResizingLeft = false;
    isResizingRight = false;
    activeFragmentId = null;

    window.removeEventListener('mousemove', onMouseMove);
    window.removeEventListener('mouseup', onMouseUp);
  }

  const formatTime = (sec: number) => {
    const min = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${min}:${s.toString().padStart(2, '0')}`;
  };
</script>

<div class="space-y-2">
  <div class="flex items-center justify-between text-xs font-mono">
    <span class="label-mono text-[#e2e2e4]">Таймлайн фрагментов</span>
    {#if videoDuration > 0}
      <span class="text-[#5865f2] font-semibold">{formatTime(currentTime)} / {formatTime(videoDuration)}</span>
    {/if}
  </div>

  <div 
    bind:this={timelineEl}
    class="relative h-20 w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.1)] rounded-[2px] overflow-hidden select-none cursor-pointer"
    on:click={handleTimelineClick}
    role="slider"
    tabindex="0"
    aria-valuenow={currentTime}
    aria-valuemin={0}
    aria-valuemax={videoDuration}
    on:keydown={(e) => {
      if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
        // Accessibility placeholder
      }
    }}
  >
    {#if videoDuration > 0}
      <!-- Grid subdivisions -->
      {#each Array(Math.ceil(videoDuration / 10)) as _, i}
        <div class="absolute top-0 bottom-0 border-l border-[rgba(226,226,224,0.05)] pointer-events-none" style="left: {i * 10 * scale}px;"></div>
      {/each}

      <!-- Fragments -->
      {#each $fragmentsStore as frag (frag.id)}
        <div 
          class="absolute top-2 bottom-2 bg-[#5865f2]/20 border border-[#5865f2] rounded-[2px] flex flex-col group transition-colors"
          style="left: {frag.start * scale}px; width: {(frag.end - frag.start) * scale}px;"
          on:click={(e) => handleFragmentClick(frag, e)}
          on:mousedown={(e) => startDrag(frag, e)}
          role="button"
          tabindex="0"
          on:keydown={(e) => e.key === 'Enter' && handleFragmentClick(frag, null as any)}
        >
          <!-- Delete button -->
          <button 
            class="absolute -top-1 -right-1 w-4 h-4 bg-red-500 hover:bg-red-400 text-white rounded-none flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity text-[9px] font-mono z-10"
            aria-label="Удалить фрагмент"
            title="Удалить фрагмент"
            on:click={(e) => handleRemoveFragment(frag.id, e)}
          >
            ✕
          </button>

          <!-- Label -->
          <div class="w-full text-center text-[9px] text-[#e2e2e4] font-mono pointer-events-none pt-0.5 overflow-hidden truncate px-1 font-bold">
            {formatTime(frag.start)} - {formatTime(frag.end)}
          </div>

          <!-- Drag content -->
          <div class="flex-grow cursor-grab active:cursor-grabbing w-full h-full"></div>

          <!-- Left Resize Handle -->
          <div 
            class="absolute top-0 bottom-0 left-0 w-2 cursor-w-resize bg-[#5865f2]/0 hover:bg-[#5865f2] transition-colors z-10"
            on:mousedown={(e) => startResizeLeft(frag, e)}
            on:click|stopPropagation
            role="separator"
            tabindex="0"
          ></div>

          <!-- Right Resize Handle -->
          <div 
            class="absolute top-0 bottom-0 right-0 w-2 cursor-e-resize bg-[#5865f2]/0 hover:bg-[#5865f2] transition-colors z-10"
            on:mousedown={(e) => startResizeRight(frag, e)}
            on:click|stopPropagation
            role="separator"
            tabindex="0"
          ></div>
        </div>
      {/each}

      <!-- Current Time Playhead -->
      <div 
        class="absolute top-0 bottom-0 w-0.5 bg-[#5865f2] pointer-events-none z-20"
        style="left: {currentTime * scale}px"
      >
        <div class="absolute -top-0.5 -left-1 w-2.5 h-2.5 bg-[#5865f2]"></div>
      </div>
    {:else}
      <div class="flex h-full items-center justify-center text-xs text-[rgba(226,226,224,0.4)] font-mono">
        Выберите видео для начала редактирования
      </div>
    {/if}
  </div>
</div>
