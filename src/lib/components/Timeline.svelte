<script lang="ts">
  import { createEventDispatcher, onMount } from 'svelte';
  import { fragmentsStore, addFragment, updateFragment, removeFragment } from '../stores/fragments';
  import type { Fragment } from '../stores/fragments';

  export let videoDuration: number = 0; // seconds
  export let currentTime: number = 0; // seconds

  const dispatch = createEventDispatcher();

  let timelineEl: HTMLElement;
  let timelineWidth: number = 0;

  // React to resize and calculate scale
  $: scale = videoDuration > 0 && timelineWidth > 0 ? timelineWidth / videoDuration : 1;

  // Interaction states
  let isDragging = false;
  let isResizingLeft = false;
  let isResizingRight = false;
  let activeFragmentId: string | null = null;

  // Mouse tracking state
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
    
    // Check if clicked exactly on a fragment is handled by child elements' stopPropagation, 
    // so if this fires, it means we clicked on empty space.
    // Automatically adding a new fragment at seek position
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

  // --- MOUSE DOWN HANDLERS ---
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

  // --- MOUSE MOVE AND UP ---
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
  <div class="flex items-center justify-between text-xs font-mono text-slate-400">
    <span>Таймлайн фрагментов</span>
    {#if videoDuration > 0}
      <span>{formatTime(currentTime)} / {formatTime(videoDuration)}</span>
    {/if}
  </div>

  <div 
    bind:this={timelineEl}
    class="relative h-24 w-full bg-slate-900 border border-slate-800 rounded overflow-hidden select-none cursor-pointer"
    on:click={handleTimelineClick}
    role="slider"
    tabindex="0"
    aria-valuenow={currentTime}
    aria-valuemin={0}
    aria-valuemax={videoDuration}
    on:keydown={(e) => {
      if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
        const delta = e.key === 'ArrowRight' ? 1 : -1;
        // In a real app this would alter currentTime, but we dispatch a seek event just in case
        // Custom events here need the same format as handled in the parent component
      }
    }}
  >
    {#if videoDuration > 0}
      <!-- Дробления шкалы времени (подсказки) -->
      {#each Array(Math.ceil(videoDuration / 10)) as _, i}
        <div class="absolute top-0 bottom-0 border-l border-slate-800/50 pointer-events-none" style="left: {i * 10 * scale}px;"></div>
      {/each}

      <!-- Фрагменты -->
      {#each $fragmentsStore as frag (frag.id)}
        <div 
          class="absolute top-2 bottom-2 bg-indigo-500/20 border border-indigo-500/50 rounded flex flex-col group"
          style="left: {frag.start * scale}px; width: {(frag.end - frag.start) * scale}px;"
          on:click={(e) => handleFragmentClick(frag, e)}
          on:mousedown={(e) => startDrag(frag, e)}
          role="button"
          tabindex="0"
          on:keydown={(e) => e.key === 'Enter' && handleFragmentClick(frag, null as any)}
        >
          <!-- Delete button -->
          <button 
            class="absolute -top-2 -right-2 w-5 h-5 bg-red-500 hover:bg-red-400 text-white rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity text-[10px] z-10 shadow-lg"
            on:click={(e) => handleRemoveFragment(frag.id, e)}
          >
            ✕
          </button>

          <!-- Label -->
          <div class="w-full text-center text-[10px] text-indigo-300 font-mono pointer-events-none pt-1 overflow-hidden truncate px-1">
            {formatTime(frag.start)} - {formatTime(frag.end)}
          </div>

          <!-- Drag content -->
          <div class="flex-grow cursor-grab active:cursor-grabbing w-full h-full"></div>

          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <!-- Left Resize Handle -->
          <div 
            class="absolute top-0 bottom-0 left-0 w-2 cursor-w-resize bg-indigo-400/0 hover:bg-indigo-400/50 transition-colors z-10"
            on:mousedown={(e) => startResizeLeft(frag, e)}
            on:click|stopPropagation
            role="separator"
            tabindex="0"
          ></div>

          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <!-- Right Resize Handle -->
          <div 
            class="absolute top-0 bottom-0 right-0 w-2 cursor-e-resize bg-indigo-400/0 hover:bg-indigo-400/50 transition-colors z-10"
            on:mousedown={(e) => startResizeRight(frag, e)}
            on:click|stopPropagation
            role="separator"
            tabindex="0"
          ></div>
        </div>
      {/each}

      <!-- Линия текущего времени -->
      <div 
        class="absolute top-0 bottom-0 w-0.5 bg-red-500 pointer-events-none shadow-[0_0_8px_rgba(239,68,68,0.8)] z-20"
        style="left: {currentTime * scale}px"
      >
        <div class="absolute -top-1 -left-1.5 w-3.5 h-3.5 rounded-full bg-red-500"></div>
      </div>
    {:else}
      <div class="flex h-full items-center justify-center text-xs text-slate-500">
        Выберите видео для начала редактирования
      </div>
    {/if}
  </div>
</div>
