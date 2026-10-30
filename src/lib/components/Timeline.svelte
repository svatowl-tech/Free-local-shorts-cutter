<script lang="ts">
  import { createEventDispatcher, onMount, tick } from 'svelte';
  import { fragmentsStore, addFragmentRange, updateFragment, removeFragment } from '../stores/fragments';
  import type { Fragment } from '../stores/fragments';
  import { ZoomIn, ZoomOut, Maximize2, Play, Pause, Crosshair } from '@lucide/svelte';

  export let videoDuration: number = 0;
  export let currentTime: number = 0;
  export let activePlayingFragmentId: string | null = null;
  export let isVideoPlaying: boolean = false;

  const dispatch = createEventDispatcher();

  let viewportEl: HTMLElement;
  let timelineEl: HTMLElement;
  let viewportWidth: number = 0;

  // Масштабирование таймлайна (1x — весь ролик, до 30x — покадровая/субсекундная точность)
  const MIN_ZOOM = 1;
  const MAX_ZOOM = 30;
  const ZOOM_PRESETS = [1, 2, 5, 10, 20];

  let zoomLevel: number = 1;
  let autoFollowPlayhead: boolean = true;

  $: trackWidth = Math.max(viewportWidth, Math.round(viewportWidth * zoomLevel));
  $: scale = videoDuration > 0 && trackWidth > 0 ? trackWidth / videoDuration : 1;

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
  let hasMovedDuringDrag = false;

  let dragStartX: number = 0;
  let initialFragStart: number = 0;
  let initialFragEnd: number = 0;

  onMount(() => {
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        viewportWidth = entry.contentRect.width;
      }
    });
    if (viewportEl) {
      observer.observe(viewportEl);
      viewportWidth = viewportEl.clientWidth;
    }
    return () => observer.disconnect();
  });

  // Автопрокрутка масштабированного таймлайна за курсором воспроизведения
  $: if (
    autoFollowPlayhead &&
    isVideoPlaying &&
    viewportEl &&
    zoomLevel > 1 &&
    !isSelecting &&
    !isDragging &&
    !isResizingLeft &&
    !isResizingRight
  ) {
    const playheadX = currentTime * scale;
    const viewLeft = viewportEl.scrollLeft;
    const viewRight = viewLeft + viewportWidth;
    const margin = viewportWidth * 0.15;

    if (playheadX > viewRight - margin || playheadX < viewLeft + margin * 0.5) {
      viewportEl.scrollLeft = Math.max(0, playheadX - viewportWidth * 0.25);
    }
  }

  export async function setZoom(newZoom: number, anchorTime?: number, anchorClientX?: number) {
    const clamped = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, Math.round(newZoom * 100) / 100));
    if (!viewportEl || videoDuration <= 0) {
      zoomLevel = clamped;
      return;
    }

    const focusTime = anchorTime !== undefined ? anchorTime : currentTime;
    const viewportRect = viewportEl.getBoundingClientRect();
    const offsetInViewport =
      anchorClientX !== undefined
        ? Math.max(0, Math.min(viewportWidth, anchorClientX - viewportRect.left))
        : viewportWidth / 2;

    zoomLevel = clamped;
    await tick();

    if (zoomLevel <= 1) {
      viewportEl.scrollLeft = 0;
    } else {
      const newTrackWidth = Math.max(viewportWidth, viewportWidth * zoomLevel);
      const newScale = newTrackWidth / videoDuration;
      const targetScrollLeft = focusTime * newScale - offsetInViewport;
      viewportEl.scrollLeft = Math.max(0, Math.min(newTrackWidth - viewportWidth, targetScrollLeft));
    }
  }

  export async function zoomToFragment(frag: Fragment, e?: MouseEvent) {
    e?.stopPropagation();
    if (videoDuration <= 0) return;
    const fragDuration = Math.max(1, frag.end - frag.start);
    const idealZoom = Math.min(MAX_ZOOM, Math.max(2, (videoDuration / fragDuration) * 0.55));
    const centerTime = (frag.start + frag.end) / 2;
    await setZoom(idealZoom, centerTime);
  }

  function handleWheel(e: WheelEvent) {
    if (videoDuration <= 0) return;
    // Если зажат Shift или идет горизонтальный свайп тачпада — стандартный горизонтальный скролл
    if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
      return;
    }
    e.preventDefault();
    const mouseTime = getMouseTime(e);
    const factor = e.deltaY < 0 ? 1.25 : 0.8;
    setZoom(zoomLevel * factor, mouseTime, e.clientX);
  }

  function getMouseTime(e: MouseEvent): number {
    if (!timelineEl || scale <= 0) return 0;
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
    if (e.button !== 0) return;
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
    // Автопрокрутка при протяжке выделения к краю видимой области
    if (viewportEl && zoomLevel > 1) {
      const rect = viewportEl.getBoundingClientRect();
      if (e.clientX > rect.right - 30) {
        viewportEl.scrollLeft += 15;
      } else if (e.clientX < rect.left + 30) {
        viewportEl.scrollLeft -= 15;
      }
    }
    dispatch('seek', { time: selectionCurrentTime });
  }

  function onTimelineMouseUp() {
    if (!isSelecting) return;
    isSelecting = false;

    window.removeEventListener('mousemove', onTimelineMouseMove);
    window.removeEventListener('mouseup', onTimelineMouseUp);

    const s = Math.min(selectionStartTime, selectionCurrentTime);
    const end = Math.max(selectionStartTime, selectionCurrentTime);

    // Если пользователь протянул мышь хотя бы на 0.4 секунды — фиксируем новый клип
    if (end - s >= 0.4) {
      addFragmentRange(Math.round(s * 20) / 20, Math.round(end * 20) / 20);
    } else {
      // Иначе это был обычный одиночный клик — просто позиционируем курсор
      dispatch('seek', { time: s });
    }
  }

  function handleFragmentClick(frag: Fragment, e: MouseEvent) {
    e?.stopPropagation();
    if (hasMovedDuringDrag || isResizingLeft || isResizingRight) return;
    // При клике по ролику на таймлайне запускаем его предпросмотр (воспроизведение от старта до конца)
    dispatch('previewFragment', { fragment: frag });
  }

  function handleRemoveFragment(id: string, e: MouseEvent) {
    e?.stopPropagation();
    removeFragment(id);
  }

  function startDrag(frag: Fragment, e: MouseEvent) {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    if (target.closest('button') || target.closest('.resize-handle')) return;

    e.stopPropagation();
    isDragging = true;
    hasMovedDuringDrag = false;
    activeFragmentId = frag.id;
    dragStartX = e.clientX;
    initialFragStart = frag.start;
    initialFragEnd = frag.end;

    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
  }

  function startResizeLeft(frag: Fragment, e: MouseEvent) {
    if (e.button !== 0) return;
    e.stopPropagation();
    isResizingLeft = true;
    hasMovedDuringDrag = false;
    activeFragmentId = frag.id;
    dragStartX = e.clientX;
    initialFragStart = frag.start;
    initialFragEnd = frag.end;

    dispatch('seek', { time: frag.start });

    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
  }

  function startResizeRight(frag: Fragment, e: MouseEvent) {
    if (e.button !== 0) return;
    e.stopPropagation();
    isResizingRight = true;
    hasMovedDuringDrag = false;
    activeFragmentId = frag.id;
    dragStartX = e.clientX;
    initialFragStart = frag.start;
    initialFragEnd = frag.end;

    dispatch('seek', { time: frag.end });

    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
  }

  function onMouseMove(e: MouseEvent) {
    if (!activeFragmentId) return;

    const deltaX = e.clientX - dragStartX;
    if (Math.abs(deltaX) > 4) {
      hasMovedDuringDrag = true;
    }

    const deltaTime = deltaX / scale;
    const fragment = $fragmentsStore.find((f) => f.id === activeFragmentId);
    if (!fragment) return;

    if (isDragging) {
      if (!hasMovedDuringDrag) return;
      const fragDuration = initialFragEnd - initialFragStart;
      let newStart = initialFragStart + deltaTime;
      let newEnd = initialFragEnd + deltaTime;

      if (newStart < 0) {
        newStart = 0;
        newEnd = fragDuration;
      }
      if (newEnd > videoDuration) {
        newEnd = videoDuration;
        newStart = Math.max(0, videoDuration - fragDuration);
      }
      newStart = Math.round(newStart * 20) / 20;
      newEnd = Math.round(newEnd * 20) / 20;
      updateFragment(activeFragmentId, { start: newStart, end: newEnd });
      dispatch('seek', { time: newStart });
    } else if (isResizingLeft) {
      let newStart = Math.max(0, Math.min(initialFragStart + deltaTime, initialFragEnd - 0.5));
      newStart = Math.round(newStart * 20) / 20;
      updateFragment(activeFragmentId, { start: newStart });
      // Покадровый предпросмотр сдвигаемой левой границы
      dispatch('seek', { time: newStart });
    } else if (isResizingRight) {
      let newEnd = Math.min(videoDuration, Math.max(initialFragEnd + deltaTime, initialFragStart + 0.5));
      newEnd = Math.round(newEnd * 20) / 20;
      updateFragment(activeFragmentId, { end: newEnd });
      // Покадровый предпросмотр сдвигаемой правой границы
      dispatch('seek', { time: newEnd });
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

  const formatRulerTick = (sec: number, step: number) => {
    const min = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    if (step < 1) {
      const ms = Math.round((sec % 1) * 10);
      return `${min}:${s.toString().padStart(2, '0')}.${ms}`;
    }
    return `${min}:${s.toString().padStart(2, '0')}`;
  };

  // Адаптивный шаг сетки времени в зависимости от масштаба (чтобы деления были каждые ~85px)
  $: gridInterval = (() => {
    if (videoDuration <= 0 || scale <= 0) return 10;
    const targetSec = 85 / scale;
    const niceSteps = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600];
    for (const step of niceSteps) {
      if (step >= targetSec) return step;
    }
    return 600;
  })();

  $: tickCount = videoDuration > 0 ? Math.min(800, Math.ceil(videoDuration / gridInterval) + 1) : 0;
</script>

<div class="space-y-2 select-none">
  <!-- Верхняя панель: подсказки, управление масштабом (Zoom) и текущее время -->
  <div class="flex flex-wrap items-center justify-between gap-2 text-xs font-mono bg-[#0e0e12] border border-[rgba(226,226,224,0.1)] px-3 py-2 rounded-[4px]">
    <div class="flex flex-wrap items-center gap-2">
      <span class="label-mono text-[#e2e2e4]">Шкала времени</span>
      <span class="text-[10px] text-[#5865f2] bg-[#5865f2]/10 border border-[#5865f2]/30 px-2 py-0.5 rounded-[2px]">
        💡 Клик по клипу — превью • Колесо мыши — масштаб • Края — точная подгонка
      </span>
    </div>

    <!-- Панель масштабирования таймлайна -->
    <div class="flex flex-wrap items-center gap-1.5">
      <button
        type="button"
        on:click={() => setZoom(zoomLevel / 1.4)}
        disabled={zoomLevel <= MIN_ZOOM || videoDuration <= 0}
        class="p-1 rounded-[2px] bg-[#16161a] hover:bg-[#25252e] disabled:opacity-40 text-[#e2e2e4] border border-[rgba(226,226,224,0.12)] transition-colors"
        title="Отдалить таймлайн"
      >
        <ZoomOut class="w-3.5 h-3.5" />
      </button>

      <!-- Ползунок плавного масштабирования -->
      <input
        type="range"
        min={MIN_ZOOM}
        max={MAX_ZOOM}
        step="0.25"
        value={zoomLevel}
        disabled={videoDuration <= 0}
        on:input={(e) => setZoom(parseFloat((e.currentTarget as HTMLInputElement).value))}
        class="w-20 sm:w-28 accent-[#5865f2] cursor-pointer h-1.5 bg-[#1a1a22] rounded-lg"
        title="Плавное масштабирование шкалы времени"
      />

      <button
        type="button"
        on:click={() => setZoom(zoomLevel * 1.4)}
        disabled={zoomLevel >= MAX_ZOOM || videoDuration <= 0}
        class="p-1 rounded-[2px] bg-[#16161a] hover:bg-[#25252e] disabled:opacity-40 text-[#e2e2e4] border border-[rgba(226,226,224,0.12)] transition-colors"
        title="Приблизить таймлайн для точной настройки границ"
      >
        <ZoomIn class="w-3.5 h-3.5" />
      </button>

      <!-- Быстрые пресеты масштаба -->
      <div class="hidden md:flex items-center gap-1 ml-1">
        {#each ZOOM_PRESETS as preset}
          <button
            type="button"
            disabled={videoDuration <= 0}
            on:click={() => setZoom(preset)}
            class="px-1.5 py-0.5 rounded-[2px] text-[10px] font-mono border transition-colors {Math.abs(zoomLevel - preset) < 0.2
              ? 'bg-[#5865f2] text-white border-[#5865f2] font-bold'
              : 'bg-[#16161a] text-[rgba(226,226,224,0.65)] border-[rgba(226,226,224,0.1)] hover:text-white'}"
            title="Масштаб {preset}x"
          >
            {preset}x
          </button>
        {/each}
      </div>

      <!-- Кнопка центрирования на текущем курсоре -->
      {#if zoomLevel > 1}
        <button
          type="button"
          on:click={() => setZoom(zoomLevel, currentTime)}
          class="px-1.5 py-0.5 rounded-[2px] text-[10px] font-mono bg-[#16161a] hover:bg-[#25252e] text-[#23c55e] border border-[#23c55e]/30 flex items-center gap-1 transition-colors"
          title="Сфокусировать видимую область на текущем времени"
        >
          <Crosshair class="w-3 h-3" />
          <span class="hidden sm:inline">К курсору</span>
        </button>

        <button
          type="button"
          on:click={() => setZoom(1)}
          class="px-1.5 py-0.5 rounded-[2px] text-[10px] font-mono bg-[#16161a] hover:bg-[#25252e] text-[rgba(226,226,224,0.7)] border border-[rgba(226,226,224,0.12)] flex items-center gap-1 transition-colors"
          title="Вписать весь таймлайн в экран (1x)"
        >
          <Maximize2 class="w-3 h-3" />
          <span>Вписать</span>
        </button>
      {/if}

      {#if videoDuration > 0}
        <span class="text-[#5865f2] font-semibold bg-[#16161a] border border-[rgba(226,226,224,0.1)] px-2 py-0.5 rounded-[2px] ml-1">
          {formatTime(currentTime)} / {formatTime(videoDuration)}
        </span>
      {/if}
    </div>
  </div>

  <!-- Горизонтально прокручиваемая область просмотра таймлайна -->
  <div
    bind:this={viewportEl}
    on:wheel={handleWheel}
    class="relative w-full overflow-x-auto overflow-y-hidden bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] rounded-[4px] shadow-inner"
  >
    <!-- Масштабируемая дорожка времени -->
    <div
      bind:this={timelineEl}
      style="width: {trackWidth > 0 ? `${trackWidth}px` : '100%'};"
      class="relative h-28 cursor-crosshair select-none"
      on:mousedown={onTimelineMouseDown}
      on:mousemove={onTimelineMouseMoveHover}
      on:mouseleave={onTimelineMouseLeave}
      role="slider"
      tabindex="0"
      aria-valuenow={currentTime}
      aria-valuemin={0}
      aria-valuemax={videoDuration}
      on:keydown={(e) => {
        const step = zoomLevel >= 10 ? 0.1 : zoomLevel >= 4 ? 0.5 : 1;
        if (e.key === 'ArrowRight') dispatch('seek', { time: Math.min(videoDuration, currentTime + step) });
        if (e.key === 'ArrowLeft') dispatch('seek', { time: Math.max(0, currentTime - step) });
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

        <!-- Временная линейка: основные деления и мелкие засечки для точной подгонки -->
        {#each Array(tickCount) as _, i}
          {@const t = Math.round(i * gridInterval * 100) / 100}
          {#if t <= videoDuration}
            <div
              class="absolute top-0 bottom-0 border-l border-[rgba(226,226,224,0.1)] pointer-events-none flex flex-col justify-between py-1"
              style="left: {t * scale}px;"
            >
              <span class="text-[9px] font-mono text-[rgba(226,226,224,0.45)] pl-1 select-none whitespace-nowrap">
                {formatRulerTick(t, gridInterval)}
              </span>
              <div class="w-1 h-2 bg-[rgba(226,226,224,0.25)]"></div>
            </div>

            <!-- Промежуточные мелкие деления (5 под-шагов внутри каждого интервала) -->
            {#each [1, 2, 3, 4] as sub}
              {@const subTime = t + (gridInterval / 5) * sub}
              {#if subTime < videoDuration}
                <div
                  class="absolute bottom-0 h-1.5 border-l border-[rgba(226,226,224,0.12)] pointer-events-none"
                  style="left: {subTime * scale}px;"
                ></div>
                <div
                  class="absolute top-0 h-1 border-l border-[rgba(226,226,224,0.08)] pointer-events-none"
                  style="left: {subTime * scale}px;"
                ></div>
              {/if}
            {/each}
          {/if}
        {/each}

        <!-- ДИНАМИЧЕСКИЙ ПРЯМОУГОЛЬНИК ВЫДЕЛЕНИЯ МЫШКОЙ (когда зажата кнопка мыши) -->
        {#if isSelecting}
          {@const selStart = Math.min(selectionStartTime, selectionCurrentTime)}
          {@const selEnd = Math.max(selectionStartTime, selectionCurrentTime)}
          {@const selWidth = (selEnd - selStart) * scale}
          <div
            class="absolute top-5 bottom-1.5 bg-[#5865f2]/40 border-2 border-dashed border-[#5865f2] rounded-[3px] pointer-events-none z-30 shadow-[0_0_20px_rgba(88,101,242,0.6)] flex items-center justify-center transition-none"
            style="left: {selStart * scale}px; width: {Math.max(4, selWidth)}px;"
          >
            <div class="absolute -top-5 bg-[#16161a] border border-[#5865f2] text-white font-mono text-[10px] px-2 py-0.5 rounded shadow-lg whitespace-nowrap z-40">
              🎯 {formatTime(selStart)} — {formatTime(selEnd)} ({(selEnd - selStart).toFixed(1)}с)
            </div>
          </div>
        {/if}

        <!-- СПИСОК ВЫДЕЛЕННЫХ ФРАГМЕНТОВ (КЛИПОВ) -->
        {#each $fragmentsStore as frag, idx (frag.id)}
          {@const fragWidth = Math.max(24, (frag.end - frag.start) * scale)}
          {@const isActivePreview = activePlayingFragmentId === frag.id && isVideoPlaying}
          {@const isCursorInside = currentTime >= frag.start && currentTime <= frag.end}
          {@const isBeingEdited = activeFragmentId === frag.id && (isResizingLeft || isResizingRight || isDragging)}

          <!-- Всплывающий индикатор точных границ при перетаскивании или ресайзе -->
          {#if isBeingEdited}
            <div
              class="absolute top-0.5 bg-[#121218] border border-[#23c55e] text-[#e2e2e4] font-mono text-[10px] px-2 py-0.5 rounded shadow-xl whitespace-nowrap z-50 pointer-events-none flex items-center gap-1.5"
              style="left: {Math.max(4, frag.start * scale)}px;"
            >
              <span class="text-[#23c55e] font-bold">Клип #{idx + 1}:</span>
              <span>{formatTime(frag.start)} → {formatTime(frag.end)}</span>
              <span class="text-[#5865f2] font-bold">({(frag.end - frag.start).toFixed(1)}с)</span>
            </div>
          {/if}

          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div
            class="fragment-box absolute top-5 bottom-1.5 {isActivePreview
              ? 'bg-gradient-to-r from-[#23c55e]/35 to-[#16a34a]/35 border-[#23c55e] shadow-[0_0_16px_rgba(35,197,94,0.55)]'
              : isCursorInside
                ? 'bg-gradient-to-r from-[#5865f2]/40 to-[#8b5cf6]/40 border-[#23c55e]/80 shadow-[0_0_10px_rgba(35,197,94,0.3)]'
                : 'bg-gradient-to-r from-[#5865f2]/30 to-[#8b5cf6]/30 border-[#5865f2] hover:border-[#7c87f8]'} border-2 rounded-[3px] flex flex-col justify-between group select-none transition-colors z-20 overflow-hidden cursor-pointer"
            style="left: {frag.start * scale}px; width: {fragWidth}px;"
            on:click={(e) => handleFragmentClick(frag, e)}
            on:dblclick={(e) => zoomToFragment(frag, e)}
            on:mousedown={(e) => startDrag(frag, e)}
            role="button"
            tabindex="0"
            title="Клик: воспроизвести клип (превью) • Двойной клик: приблизить масштаб к клипу • Потяните края для точной настройки границ"
            on:keydown={(e) => e.key === 'Enter' && handleFragmentClick(frag, null as any)}
          >
            <!-- Верхняя строка клипа: номер, время и кнопки управления -->
            <div class="w-full flex items-center justify-between text-[10px] text-[#e2e2e4] font-mono px-2 pt-1 font-bold gap-1">
              <div class="flex items-center gap-1 min-w-0 pointer-events-none">
                <span class="{isActivePreview ? 'bg-[#23c55e] text-black' : 'bg-[#5865f2] text-white'} px-1 py-0.2 rounded text-[9px] shrink-0">
                  #{idx + 1}
                </span>
                {#if fragWidth > 95}
                  <span class="text-[9px] text-[#e2e2e4] opacity-95 truncate">
                    {formatTime(frag.start)}–{formatTime(frag.end)}
                  </span>
                {/if}
              </div>

              <div class="flex items-center gap-1 shrink-0 z-30">
                {#if fragWidth > 55}
                  <span class="text-[9px] text-[#23c55e] bg-black/40 px-1 rounded pointer-events-none">
                    {(frag.end - frag.start).toFixed(1)}с
                  </span>
                {/if}

                <!-- Кнопка быстрого зума к этому клипу -->
                <button
                  type="button"
                  class="w-4 h-4 bg-[#16161a]/90 hover:bg-[#5865f2] text-white rounded-[2px] flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity shadow"
                  title="Приблизить таймлайн к этому клипу"
                  on:click={(e) => zoomToFragment(frag, e)}
                >
                  <ZoomIn class="w-2.5 h-2.5" />
                </button>

                <!-- Кнопка удаления фрагмента -->
                <button
                  type="button"
                  class="w-4 h-4 bg-red-600/90 hover:bg-red-500 text-white rounded-[2px] flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity text-[9px] font-mono shadow"
                  aria-label="Удалить фрагмент"
                  title="Удалить этот клип"
                  on:click={(e) => handleRemoveFragment(frag.id, e)}
                >
                  ✕
                </button>
              </div>
            </div>

            <!-- Центральная область: индикатор воспроизведения превью при клике -->
            <div class="flex-1 w-full flex items-center justify-center gap-1.5 pointer-events-none px-2">
              <div class="w-5 h-5 rounded-full {isActivePreview ? 'bg-[#23c55e] text-black' : 'bg-black/45 text-white group-hover:bg-[#5865f2]'} flex items-center justify-center transition-colors shadow">
                {#if isActivePreview}
                  <Pause class="w-3 h-3" />
                {:else}
                  <Play class="w-3 h-3 ml-0.5" />
                {/if}
              </div>
              {#if fragWidth > 80}
                <span class="text-[9px] font-mono {isActivePreview ? 'text-[#23c55e] font-bold' : 'text-white/75 group-hover:text-white'} truncate">
                  {isActivePreview ? 'Идет превью...' : 'Превью клипа'}
                </span>
              {/if}
            </div>

            <!-- Левая ручка изменения размера (Start Resize Handle) -->
            <!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions a11y_click_events_have_key_events -->
            <div
              class="resize-handle absolute top-0 bottom-0 left-0 w-3 cursor-ew-resize bg-white/10 hover:bg-[#23c55e] flex items-center justify-center transition-colors z-30"
              on:mousedown={(e) => startResizeLeft(frag, e)}
              on:click|stopPropagation
              role="separator"
              tabindex="0"
              title="Потяните для точной настройки начала клипа ({formatTime(frag.start)})"
            >
              <div class="w-0.5 h-5 bg-white/80 rounded"></div>
            </div>

            <!-- Правая ручка изменения размера (End Resize Handle) -->
            <!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions a11y_click_events_have_key_events -->
            <div
              class="resize-handle absolute top-0 bottom-0 right-0 w-3 cursor-ew-resize bg-white/10 hover:bg-[#23c55e] flex items-center justify-center transition-colors z-30"
              on:mousedown={(e) => startResizeRight(frag, e)}
              on:click|stopPropagation
              role="separator"
              tabindex="0"
              title="Потяните для точной настройки конца клипа ({formatTime(frag.end)})"
            >
              <div class="w-0.5 h-5 bg-white/80 rounded"></div>
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
</div>


