<script lang="ts">
  import { createEventDispatcher, onMount } from 'svelte';
  import { fragmentsStore, addFragmentRange, updateFragment, removeFragment } from '../stores/fragments';
  import type { Fragment } from '../stores/fragments';

  export let videoDuration: number = 0;
  export let currentTime: number = 0;

  const dispatch = createEventDispatcher();

  let timelineEl: HTMLElement;
  let timelineWidth: number = 0;

  $: scale = videoDuration > 0 && timelineWidth > 0 ? timelineWidth / videoDuration : 1;

  // Состояние выделения области мышкой (drag to select)
  let isSelecting = false;
  let selectionStartTime: number = 0;
  let selectionCurrentTime: number = 0;

  // Предпросмотр времени при наведении мыши (Hover scrubber)
  let hoverTime: number | null = null;

  // Состояние перемещения и изменения размера фрагментов
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

  function onTimelineMouseMoveHover(e: MouseEvent) {
    if (isSelecting || isDragging || isResizingLeft || isResizingRight) {
      hoverTime = null;
      return;
    }
    hoverTime = getMouseTime(e);
  }

  function onTimelineMouseLeave() {
    hoverTime = null;
  }

  // 1. Начало выделения области мышкой на шкале времени
  function onTimelineMouseDown(e: MouseEvent) {
    // Если кликнули на фрагмент или ручку ресайза — выделение новой области не запускаем
    const target = e.target as HTMLElement;
    if (target.closest('.fragment-box') || target.closest('.resize-handle')) {
      return;
    }

    if (videoDuration <= 0) return;

    isSelecting = true;
    const clickTime = getMouseTime(e);
    selectionStartTime = clickTime;
    selectionCurrentTime = clickTime;

    dispatch('seek', { time: clickTime });

    window.addEventListener('mousemove', onTimelineMouseMove);
    window.addEventListener('mouseup', onTimelineMouseUp);
  }

  function onTimelineMouseMove(e: MouseEvent) {
    if (!isSelecting) return;
    selectionCurrentTime = getMouseTime(e);
    // Плавно обновляем текущую позицию для удобства навигации
    dispatch('seek', { time: selectionCurrentTime });
  }

  function onTimelineMouseUp(e: MouseEvent) {
    if (!isSelecting) return;
    isSelecting = false;

    window.removeEventListener('mousemove', onTimelineMouseMove);
    window.removeEventListener('mouseup', onTimelineMouseUp);

    const s = Math.min(selectionStartTime, selectionCurrentTime);
    const end = Math.max(selectionStartTime, selectionCurrentTime);

    // Если пользователь протянул мышь хотя бы на 0.4 секунды — фиксируем новый клип
    if (end - s >= 0.4) {
      addFragmentRange(s, end);
    } else {
      // Иначе это был обычный одиночный клик — просто позиционируем курсор
      dispatch('seek', { time: s });
    }
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
    const ms = Math.floor((sec % 1) * 10);
    return `${min.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}.${ms}`;
  };

  // Шаг разметки времени для сетки
  $: gridInterval = videoDuration > 600 ? 60 : (videoDuration > 120 ? 30 : 10);
</script>

<div class="space-y-2 select-none">
  <!-- Верхняя строка подсказок и времени -->
  <div class="flex items-center justify-between text-xs font-mono">
    <div class="flex items-center gap-2">
      <span class="label-mono text-[#e2e2e4]">Разметка клипов (Reels 9:16)</span>
      <span class="text-[10px] text-[#5865f2] bg-[#5865f2]/10 border border-[#5865f2]/30 px-2 py-0.5 rounded-[2px]">
        💡 Зажмите ЛКМ и тяните для выделения области
      </span>
    </div>
    {#if videoDuration > 0}
      <span class="text-[#5865f2] font-semibold bg-[#16161a] border border-[rgba(226,226,224,0.1)] px-2 py-0.5 rounded-[2px]">
        {formatTime(currentTime)} / {formatTime(videoDuration)}
      </span>
    {/if}
  </div>

  <!-- Шкала времени с интерактивным выделением мышкой -->
  <div 
    bind:this={timelineEl}
    class="relative h-24 w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] rounded-[4px] overflow-hidden cursor-crosshair shadow-inner"
    on:mousedown={onTimelineMouseDown}
    on:mousemove={onTimelineMouseMoveHover}
    on:mouseleave={onTimelineMouseLeave}
    role="slider"
    tabindex="0"
    aria-valuenow={currentTime}
    aria-valuemin={0}
    aria-valuemax={videoDuration}
    on:keydown={(e) => {
      if (e.key === 'ArrowRight') dispatch('seek', { time: Math.min(videoDuration, currentTime + 1) });
      if (e.key === 'ArrowLeft') dispatch('seek', { time: Math.max(0, currentTime - 1) });
    }}
  >
    {#if videoDuration > 0}
      <!-- Линия предпросмотра при наведении курсора (Hover scrubber) -->
      {#if hoverTime !== null && !isSelecting}
        <div 
          class="absolute top-0 bottom-0 w-[1px] bg-white/50 pointer-events-none z-15"
          style="left: {hoverTime * scale}px"
        >
          <div class="absolute top-1 left-1.5 bg-[#1a1a24] border border-white/20 text-[#e2e2e4] font-mono text-[9px] px-1.5 py-0.5 rounded shadow whitespace-nowrap">
            {formatTime(hoverTime)}
          </div>
        </div>
      {/if}

      <!-- Временная сетка и секундные засечки -->
      {#each Array(Math.ceil(videoDuration / gridInterval) + 1) as _, i}
        {@const t = i * gridInterval}
        {#if t <= videoDuration}
          <div 
            class="absolute top-0 bottom-0 border-l border-[rgba(226,226,224,0.08)] pointer-events-none flex flex-col justify-between py-1"
            style="left: {t * scale}px;"
          >
            <span class="text-[9px] font-mono text-[rgba(226,226,224,0.4)] pl-1 select-none">
              {Math.floor(t / 60)}:{(t % 60).toString().padStart(2, '0')}
            </span>
            <div class="w-1 h-1.5 bg-[rgba(226,226,224,0.2)]"></div>
          </div>
        {/if}
      {/each}

      <!-- ДИНАМИЧЕСКИЙ ПРЯМОУГОЛЬНИК ВЫДЕЛЕНИЯ МЫШКОЙ (когда зажата кнопка мыши) -->
      {#if isSelecting}
        {@const selStart = Math.min(selectionStartTime, selectionCurrentTime)}
        {@const selEnd = Math.max(selectionStartTime, selectionCurrentTime)}
        {@const selWidth = (selEnd - selStart) * scale}
        <div 
          class="absolute top-1 bottom-1 bg-[#5865f2]/40 border-2 border-dashed border-[#5865f2] rounded-[3px] pointer-events-none z-30 shadow-[0_0_20px_rgba(88,101,242,0.6)] flex items-center justify-center transition-none"
          style="left: {selStart * scale}px; width: {Math.max(4, selWidth)}px;"
        >
          <div class="absolute -top-7 bg-[#16161a] border border-[#5865f2] text-white font-mono text-[10px] px-2 py-0.5 rounded shadow-lg whitespace-nowrap z-40">
            🎯 Reels: {formatTime(selStart)} — {formatTime(selEnd)} ({(selEnd - selStart).toFixed(1)}с)
          </div>
        </div>
      {/if}

      <!-- СПИСОК ВЫДЕЛЕННЫХ ФРАГМЕНТОВ (КЛИПОВ) -->
      {#each $fragmentsStore as frag, idx (frag.id)}
        {@const fragWidth = Math.max(18, (frag.end - frag.start) * scale)}
        {@const isPlaying = currentTime >= frag.start && currentTime <= frag.end}
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div 
          class="fragment-box absolute top-2 bottom-2 bg-gradient-to-r from-[#5865f2]/35 to-[#8b5cf6]/35 border-2 {isPlaying ? 'border-[#23c55e] shadow-[0_0_15px_rgba(35,197,94,0.5)]' : 'border-[#5865f2] hover:border-[#7c87f8]'} rounded-[3px] flex flex-col group select-none transition-colors z-20 overflow-hidden"
          style="left: {frag.start * scale}px; width: {fragWidth}px;"
          on:click={(e) => handleFragmentClick(frag, e)}
          on:mousedown={(e) => startDrag(frag, e)}
          role="button"
          tabindex="0"
          title="Клик: предпросмотр. Перетащите по шкале для смещения."
          on:keydown={(e) => e.key === 'Enter' && handleFragmentClick(frag, null as any)}
        >
          <!-- Кнопка удаления фрагмента -->
          <button 
            class="absolute top-1 right-1 w-4 h-4 bg-red-600 hover:bg-red-500 text-white rounded-[2px] flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity text-[10px] font-mono z-30 shadow"
            aria-label="Удалить фрагмент"
            title="Удалить этот клип"
            on:click={(e) => handleRemoveFragment(frag.id, e)}
          >
            ✕
          </button>

          <!-- Шапка клипа: номер и время -->
          <div class="w-full flex items-center justify-between text-[10px] text-[#e2e2e4] font-mono px-1.5 pt-0.5 pointer-events-none font-bold">
            <span class="bg-[#5865f2] text-white px-1 py-0.2 rounded text-[9px]">#{idx + 1}</span>
            <span class="text-[9px] text-[#e2e2e4] opacity-90 truncate ml-1">{formatTime(frag.start)}</span>
            <span class="text-[9px] text-[#23c55e] ml-auto">{(frag.end - frag.start).toFixed(0)}с</span>
          </div>

          <!-- Центральная область перемещения клипа -->
          <div class="flex-1 cursor-grab active:cursor-grabbing w-full flex items-center justify-center">
            <span class="text-[9px] font-mono text-white/50 group-hover:text-white/80 transition-colors">9:16 Reels</span>
          </div>

          <!-- Левая ручка изменения размера (Start Resize Handle) -->
          <div 
            class="resize-handle absolute top-0 bottom-0 left-0 w-3 cursor-ew-resize bg-white/5 hover:bg-[#5865f2] flex items-center justify-center transition-colors z-30"
            on:mousedown={(e) => startResizeLeft(frag, e)}
            on:click|stopPropagation
            role="separator"
            tabindex="0"
            title="Потяните для изменения начала клипа"
          >
            <div class="w-0.5 h-4 bg-white/60 rounded"></div>
          </div>

          <!-- Правая ручка изменения размера (End Resize Handle) -->
          <div 
            class="resize-handle absolute top-0 bottom-0 right-0 w-3 cursor-ew-resize bg-white/5 hover:bg-[#5865f2] flex items-center justify-center transition-colors z-30"
            on:mousedown={(e) => startResizeRight(frag, e)}
            on:click|stopPropagation
            role="separator"
            tabindex="0"
            title="Потяните для изменения конца клипа"
          >
            <div class="w-0.5 h-4 bg-white/60 rounded"></div>
          </div>
        </div>
      {/each}

      <!-- Текущий маркер воспроизведения (Playhead) -->
      <div 
        class="absolute top-0 bottom-0 w-0.5 bg-[#e2e2e4] pointer-events-none z-35 shadow-[0_0_8px_white]"
        style="left: {currentTime * scale}px"
      >
        <div class="absolute -top-1 -left-1.5 w-3.5 h-3.5 bg-[#e2e2e4] rotate-45 border border-black/50"></div>
      </div>
    {:else}
      <div class="flex h-full items-center justify-center text-xs text-[rgba(226,226,224,0.4)] font-mono">
        Загрузите видеофайл для интерактивной разметки Reels
      </div>
    {/if}
  </div>
</div>

