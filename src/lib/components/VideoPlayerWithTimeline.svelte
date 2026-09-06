<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { pipelineStore } from '../stores/pipeline';
  import { fragmentsStore, addFragment, addFragmentRange, removeFragment, updateFragment, clearFragments } from '../stores/fragments';
  import type { Fragment } from '../stores/fragments';
  import Timeline from './Timeline.svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { 
    Play, 
    Pause, 
    RotateCcw, 
    RotateCw, 
    Trash2, 
    Smartphone, 
    Film, 
    Sparkles, 
    Clock, 
    Download, 
    Check,
    Volume2,
    VolumeX
  } from '@lucide/svelte';

  let videoEl: HTMLVideoElement;
  let videoSrc = '';
  let duration = 0;
  let currentTime = 0;
  let isPlaying = false;
  let isMuted = false;
  let playbackSpeed = 1;

  // Рамка предпросмотра 9:16 Reels (кадрирование поверх 16:9)
  let showCropGuide = true;

  // Текущая точка "In" для быстрой разметки с клавиатуры / кнопок
  let inPoint: number | null = null;

  // Воспроизведение конкретного фрагмента с автоостановкой
  let activePlayingFragment: Fragment | null = null;

  // Экспортное состояние
  let isExporting = false;

  $: {
    if ($pipelineStore.video) {
      if ($pipelineStore.video.objectUrl) {
        videoSrc = $pipelineStore.video.objectUrl;
      } else if ($pipelineStore.video.path) {
        try {
          const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;
          if (isTauri) {
            videoSrc = convertFileSrc($pipelineStore.video.path);
          } else {
            videoSrc = '';
          }
        } catch (err) {
          console.error("Failed to convert file src", err);
        }
      }
    } else {
      videoSrc = '';
    }
  }

  function handleTimeUpdate() {
    if (!videoEl) return;
    currentTime = videoEl.currentTime;

    // Если воспроизводим конкретный фрагмент — останавливаем в конце
    if (activePlayingFragment && currentTime >= activePlayingFragment.end) {
      videoEl.pause();
      isPlaying = false;
      activePlayingFragment = null;
    }
  }

  function handleLoadedMetadata() {
    if (videoEl) {
      duration = videoEl.duration;
      if ($pipelineStore.video && !$pipelineStore.video.duration) {
        pipelineStore.setVideo({
          ...$pipelineStore.video,
          duration: videoEl.duration
        });
      }
    }
  }

  function togglePlay() {
    if (!videoEl) return;
    if (videoEl.paused) {
      videoEl.play();
      isPlaying = true;
    } else {
      videoEl.pause();
      isPlaying = false;
      activePlayingFragment = null;
    }
  }

  function toggleMute() {
    if (!videoEl) return;
    videoEl.muted = !videoEl.muted;
    isMuted = videoEl.muted;
  }

  function changeSpeed() {
    if (!videoEl) return;
    const speeds = [1, 1.25, 1.5, 2];
    const nextIdx = (speeds.indexOf(playbackSpeed) + 1) % speeds.length;
    playbackSpeed = speeds[nextIdx];
    videoEl.playbackRate = playbackSpeed;
  }

  function skipTime(delta: number) {
    if (!videoEl) return;
    videoEl.currentTime = Math.max(0, Math.min(duration, videoEl.currentTime + delta));
  }

  function handleSeek(event: CustomEvent<{ time: number }>) {
    if (videoEl) {
      videoEl.currentTime = event.detail.time;
    }
  }

  function playFragment(frag: Fragment) {
    if (!videoEl) return;
    activePlayingFragment = frag;
    videoEl.currentTime = frag.start;
    videoEl.play();
    isPlaying = true;
  }

  // Маркировка In / Out точек
  function setMarkIn() {
    inPoint = currentTime;
  }

  function setMarkOut() {
    if (inPoint !== null) {
      const start = Math.min(inPoint, currentTime);
      const end = Math.max(inPoint, currentTime);
      addFragmentRange(start, end);
      inPoint = null;
    } else {
      // Если In не был нажат, делаем клип от 0 до текущего или от текущего -15с
      const start = Math.max(0, currentTime - 15);
      addFragmentRange(start, currentTime);
    }
  }

  function quickAddClip(seconds: number) {
    const start = currentTime;
    const end = Math.min(duration || 180, currentTime + seconds);
    addFragmentRange(start, end);
  }

  // Запуск экспорта выделенных Reels
  async function handleExportManualClips() {
    if ($fragmentsStore.length === 0) {
      pipelineStore.updateError('Пожалуйста, выделите хотя бы одну область на таймлайне для экспорта в Reels!');
      return;
    }

    if (!$pipelineStore.video) {
      pipelineStore.updateError('Видеофайл не выбран!');
      return;
    }

    isExporting = true;
    try {
      await pipelineStore.exportManualClips(
        $pipelineStore.video.path,
        $fragmentsStore,
        {
          ...$pipelineStore.config,
          vertical_format: true
        }
      );
    } catch (e: any) {
      console.error('Ошибка экспорта клипов:', e);
    } finally {
      isExporting = false;
    }
  }

  const formatTime = (sec: number) => {
    const min = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    const ms = Math.floor((sec % 1) * 10);
    return `${min.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}.${ms}`;
  };

  // Суммарное время всех выделенных клипов
  $: totalClipsDuration = $fragmentsStore.reduce((acc, f) => acc + (f.end - f.start), 0);
</script>

<div class="panel p-6 sm:p-7 flex flex-col gap-5 border border-[rgba(226,226,224,0.15)] bg-[#121216] rounded-[4px]">
  <!-- Верхний заголовок секции разметки -->
  <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-3 border-b border-[rgba(226,226,224,0.1)] pb-4">
    <div class="flex items-center gap-3">
      <div class="w-9 h-9 bg-gradient-to-tr from-[#5865f2] to-[#8b5cf6] rounded-[4px] flex items-center justify-center text-white shadow">
        <Film class="w-5 h-5" />
      </div>
      <div>
        <h2 class="font-syne font-bold text-base text-[#e2e2e4] flex items-center gap-2">
          Ручная разметка Reels & Shorts
          <span class="text-[10px] font-mono px-2 py-0.5 rounded bg-[#5865f2]/20 border border-[#5865f2]/40 text-[#5865f2] uppercase font-bold">
            9:16 + Whisper
          </span>
        </h2>
        <p class="text-xs text-[rgba(226,226,224,0.6)] font-mono">
          Выделяйте мышкой любые области на шкале времени для одновременного экспорта
        </p>
      </div>
    </div>

    <!-- Переключатель визуальной рамки 9:16 -->
    <div class="flex items-center gap-2">
      <button
        type="button"
        on:click={() => showCropGuide = !showCropGuide}
        class="px-3 py-1.5 rounded-[3px] text-xs font-mono border transition-all flex items-center gap-1.5 {
          showCropGuide 
            ? 'bg-[#5865f2] text-white border-[#5865f2] shadow-[0_0_10px_rgba(88,101,242,0.4)]' 
            : 'bg-[#1a1a20] text-[rgba(226,226,224,0.6)] border-[rgba(226,226,224,0.15)] hover:text-white'
        }"
        title="Показывает контур вертикального кадра 9:16 на горизонтальном видео"
      >
        <Smartphone class="w-3.5 h-3.5" />
        <span>Рамка 9:16 Reels</span>
        {#if showCropGuide}
          <span class="w-1.5 h-1.5 rounded-full bg-white animate-pulse"></span>
        {/if}
      </button>
    </div>
  </div>

  <!-- Видеоплеер с рамкой 9:16 Reels -->
  <div class="relative w-full aspect-video bg-[#08080a] rounded-[4px] overflow-hidden border border-[rgba(226,226,224,0.12)] flex items-center justify-center shadow-2xl group">
    {#if videoSrc}
      <!-- svelte-ignore a11y-media-has-caption -->
      <video
        bind:this={videoEl}
        src={videoSrc}
        class="w-full h-full object-contain cursor-pointer"
        on:click={togglePlay}
        on:timeupdate={handleTimeUpdate}
        on:loadedmetadata={handleLoadedMetadata}
        on:play={() => isPlaying = true}
        on:pause={() => isPlaying = false}
      >
        Ваш браузер не поддерживает видео.
      </video>

      <!-- Визуальный оверлей каше 9:16 для вертикальных Reels -->
      {#if showCropGuide}
        <div class="absolute inset-0 pointer-events-none flex items-center justify-center z-10">
          <!-- Левая затемненная шторка -->
          <div class="h-full flex-1 bg-black/75 backdrop-blur-[1px] border-r border-dashed border-[#5865f2]/80"></div>
          
          <!-- Центральная область 9:16 (то, что пойдет в Reels) -->
          <div class="h-full aspect-[9/16] relative border-x-2 border-[#5865f2] shadow-[0_0_20px_rgba(88,101,242,0.3)] flex flex-col justify-between p-3">
            <div class="flex items-center justify-between">
              <span class="text-[10px] font-mono font-bold uppercase bg-[#5865f2] text-white px-2 py-0.5 rounded shadow">
                Кадр 9:16 Reels
              </span>
              <span class="text-[9px] font-mono text-white/80 bg-black/60 px-1.5 py-0.5 rounded">
                1080×1920
              </span>
            </div>

            <!-- Имитация зоны субтитров Whisper -->
            <div class="text-center bg-black/70 backdrop-blur-sm border border-white/20 text-white font-syne font-bold text-xs px-2 py-1.5 rounded shadow mb-3 mx-2">
              [ Субтитры Whisper ASR ]
            </div>
          </div>

          <!-- Правая затемненная шторка -->
          <div class="h-full flex-1 bg-black/75 backdrop-blur-[1px] border-l border-dashed border-[#5865f2]/80"></div>
        </div>
      {/if}

      <!-- Большой Play индикатор по центру при паузе -->
      {#if !isPlaying}
        <!-- svelte-ignore a11y-no-static-element-interactions -->
        <div 
          on:click={togglePlay}
          class="absolute inset-0 flex items-center justify-center bg-black/20 cursor-pointer z-20"
        >
          <div class="w-14 h-14 rounded-full bg-[#5865f2]/90 hover:bg-[#5865f2] text-white flex items-center justify-center shadow-lg transition-transform hover:scale-110">
            <Play class="w-6 h-6 ml-1" />
          </div>
        </div>
      {/if}
    {:else}
      <div class="text-[rgba(226,226,224,0.4)] font-mono text-xs flex flex-col items-center gap-3 p-8 text-center">
        <Film class="w-10 h-10 text-[#5865f2]/40" />
        <div>
          <div class="font-semibold text-sm text-[#e2e2e4] mb-1">Видео не загружено для предпросмотра</div>
          <p class="text-xs text-[rgba(226,226,224,0.5)] max-w-sm">
            Загрузите видеофайл в блоке выше, чтобы разметить области и экспортировать их в 9:16.
          </p>
        </div>
      </div>
    {/if}
  </div>

  <!-- Панель управления воспроизведением видео -->
  <div class="bg-[#0e0e12] border border-[rgba(226,226,224,0.1)] rounded-[4px] p-3 flex flex-wrap items-center justify-between gap-3">
    <!-- Левая группа: Play, перемотка, время -->
    <div class="flex items-center gap-2">
      <button
        type="button"
        on:click={togglePlay}
        disabled={!videoSrc}
        class="w-8 h-8 rounded-[3px] bg-[#5865f2] hover:bg-[#4752c4] disabled:opacity-40 text-white flex items-center justify-center transition-colors shadow"
        title="Воспроизведение / Пауза (Пробел)"
      >
        {#if isPlaying}
          <Pause class="w-4 h-4" />
        {:else}
          <Play class="w-4 h-4 ml-0.5" />
        {/if}
      </button>

      <button
        type="button"
        on:click={() => skipTime(-5)}
        disabled={!videoSrc}
        class="p-1.5 rounded-[3px] bg-[#1a1a20] hover:bg-[#25252e] disabled:opacity-40 text-[#e2e2e4] border border-[rgba(226,226,224,0.1)] transition-colors"
        title="Назад на 5 секунд"
      >
        <RotateCcw class="w-3.5 h-3.5" />
      </button>

      <button
        type="button"
        on:click={() => skipTime(5)}
        disabled={!videoSrc}
        class="p-1.5 rounded-[3px] bg-[#1a1a20] hover:bg-[#25252e] disabled:opacity-40 text-[#e2e2e4] border border-[rgba(226,226,224,0.1)] transition-colors"
        title="Вперед на 5 секунд"
      >
        <RotateCw class="w-3.5 h-3.5" />
      </button>

      <!-- Индикатор текущего времени -->
      <div class="font-mono text-xs text-[#e2e2e4] ml-2 bg-[#141418] px-2.5 py-1 rounded border border-[rgba(226,226,224,0.1)]">
        <span class="text-[#5865f2] font-bold">{formatTime(currentTime)}</span>
        <span class="text-[rgba(226,226,224,0.4)]"> / </span>
        <span class="text-[rgba(226,226,224,0.7)]">{formatTime(duration || 180)}</span>
      </div>

      <!-- Звук и скорость -->
      <button
        type="button"
        on:click={toggleMute}
        class="p-1.5 rounded-[3px] bg-[#1a1a20] hover:bg-[#25252e] text-[#e2e2e4] border border-[rgba(226,226,224,0.1)] transition-colors ml-1"
        title="Вкл / Выкл звук"
      >
        {#if isMuted}
          <VolumeX class="w-3.5 h-3.5 text-red-400" />
        {:else}
          <Volume2 class="w-3.5 h-3.5" />
        {/if}
      </button>

      <button
        type="button"
        on:click={changeSpeed}
        class="px-2 py-1 rounded-[3px] bg-[#1a1a20] hover:bg-[#25252e] text-xs font-mono text-[#e2e2e4] border border-[rgba(226,226,224,0.1)] transition-colors"
        title="Скорость воспроизведения"
      >
        {playbackSpeed}x
      </button>
    </div>

    <!-- Правая группа: Быстрая расстановка точек In/Out и пресеты клипов -->
    <div class="flex flex-wrap items-center gap-1.5">
      <span class="text-[11px] font-mono text-[rgba(226,226,224,0.4)] mr-1">Метки:</span>
      
      <button
        type="button"
        on:click={setMarkIn}
        class="px-2.5 py-1 rounded-[3px] text-xs font-mono border transition-colors {
          inPoint !== null 
            ? 'bg-[#23c55e]/20 text-[#23c55e] border-[#23c55e]/50' 
            : 'bg-[#1a1a20] hover:bg-[#25252e] text-[#e2e2e4] border-[rgba(226,226,224,0.15)]'
        }"
        title="Поставить точку начала фрагмента на текущей секунде"
      >
        [ In {inPoint !== null ? `(${formatTime(inPoint)})` : ''}
      </button>

      <button
        type="button"
        on:click={setMarkOut}
        class="px-2.5 py-1 rounded-[3px] text-xs font-mono bg-[#1a1a20] hover:bg-[#25252e] text-[#e2e2e4] border border-[rgba(226,226,224,0.15)] transition-colors"
        title="Поставить точку конца и создать фрагмент"
      >
        Out ]
      </button>

      <div class="w-px h-4 bg-[rgba(226,226,224,0.15)] mx-1"></div>

      <!-- Быстрое добавление фиксированных интервалов -->
      <button
        type="button"
        on:click={() => quickAddClip(15)}
        class="px-2 py-1 rounded-[3px] text-xs font-mono bg-[#5865f2]/15 hover:bg-[#5865f2]/30 text-[#5865f2] border border-[#5865f2]/30 transition-colors"
        title="Добавить клип 15 секунд с текущей секунды"
      >
        + 15с
      </button>

      <button
        type="button"
        on:click={() => quickAddClip(30)}
        class="px-2 py-1 rounded-[3px] text-xs font-mono bg-[#5865f2]/15 hover:bg-[#5865f2]/30 text-[#5865f2] border border-[#5865f2]/30 transition-colors"
        title="Добавить клип 30 секунд с текущей секунды"
      >
        + 30с
      </button>

      <button
        type="button"
        on:click={() => quickAddClip(60)}
        class="px-2 py-1 rounded-[3px] text-xs font-mono bg-[#5865f2]/15 hover:bg-[#5865f2]/30 text-[#5865f2] border border-[#5865f2]/30 transition-colors"
        title="Добавить клип 60 секунд с текущей секунды"
      >
        + 60с
      </button>

      {#if $fragmentsStore.length > 0}
        <button
          type="button"
          on:click={() => clearFragments()}
          class="p-1.5 rounded-[3px] bg-red-950/30 hover:bg-red-900/50 text-red-400 border border-red-800/40 transition-colors ml-1"
          title="Очистить все выделенные клипы"
        >
          <Trash2 class="w-3.5 h-3.5" />
        </button>
      {/if}
    </div>
  </div>

  <!-- Интерактивная шкала времени с поддержкой протяжки мышкой -->
  <Timeline 
    videoDuration={duration || ($pipelineStore.video?.duration || 180)}
    {currentTime}
    on:seek={handleSeek}
  />

  <!-- Блок списка выделенных областей Reels -->
  <div class="border border-[rgba(226,226,224,0.12)] bg-[#0e0e12] rounded-[4px] p-4 flex flex-col gap-3">
    <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-2 border-b border-[rgba(226,226,224,0.08)] pb-2.5">
      <div class="flex items-center gap-2">
        <Sparkles class="w-4 h-4 text-[#5865f2]" />
        <span class="font-syne font-bold text-sm text-[#e2e2e4]">
          Выделенные области для экспорта ({$fragmentsStore.length})
        </span>
      </div>

      <div class="flex items-center gap-3 text-xs font-mono text-[rgba(226,226,224,0.6)]">
        <span>Суммарно: <strong class="text-[#23c55e]">{totalClipsDuration.toFixed(1)} сек</strong></span>
        <span>•</span>
        <span>Формат: <strong class="text-[#5865f2]">9:16 Reels</strong></span>
        <span>•</span>
        <span>Субтитры: <strong class="text-purple-400">Whisper ASR</strong></span>
      </div>
    </div>

    <!-- Карточки выделенных клипов -->
    {#if $fragmentsStore.length === 0}
      <div class="py-6 px-4 text-center border border-dashed border-[rgba(226,226,224,0.1)] rounded-[3px] bg-[#141418]/50">
        <p class="text-xs text-[rgba(226,226,224,0.7)] font-mono max-w-lg mx-auto leading-relaxed">
          🖱️ <strong>Как выделить область:</strong> зажмите левую кнопку мыши на таймлайне выше и протяните отрезок. 
          Выделите 1-ю область, затем 2-ю область и вообще любое количество областей!
        </p>
      </div>
    {:else}
      <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2.5 max-h-[260px] overflow-y-auto pr-1">
        {#each $fragmentsStore as frag, idx (frag.id)}
          {@const dur = (frag.end - frag.start).toFixed(1)}
          {@const isPlayingThis = activePlayingFragment?.id === frag.id}
          <div class="p-3 bg-[#15151b] border {isPlayingThis ? 'border-[#23c55e] shadow-[0_0_10px_rgba(35,197,94,0.3)]' : 'border-[rgba(226,226,224,0.1)]'} rounded-[3px] flex flex-col justify-between gap-2.5 transition-all">
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-1.5">
                <span class="px-1.5 py-0.5 rounded bg-[#5865f2] text-white font-mono text-[10px] font-bold">
                  #{idx + 1}
                </span>
                <span class="font-mono text-xs font-semibold text-[#e2e2e4]">
                  {formatTime(frag.start)} → {formatTime(frag.end)}
                </span>
              </div>
              <span class="font-mono text-[11px] text-[#23c55e] font-bold bg-[#23c55e]/10 px-1.5 py-0.5 rounded">
                {dur}с
              </span>
            </div>

            <!-- Тонкая подгонка секунд -->
            <div class="flex items-center justify-between text-[10px] font-mono text-[rgba(226,226,224,0.5)] pt-1 border-t border-[rgba(226,226,224,0.06)]">
              <span>9:16 + Сабы</span>
              <div class="flex items-center gap-1.5">
                <!-- Кнопка воспроизведения только этого фрагмента -->
                <button
                  type="button"
                  on:click={() => playFragment(frag)}
                  class="px-2 py-0.5 rounded bg-[#1f1f28] hover:bg-[#5865f2] text-[#e2e2e4] hover:text-white transition-colors flex items-center gap-1"
                  title="Воспроизвести только этот клип"
                >
                  <Play class="w-2.5 h-2.5" />
                  <span>Превью</span>
                </button>

                <!-- Кнопка удаления фрагмента -->
                <button
                  type="button"
                  on:click={() => removeFragment(frag.id)}
                  class="p-1 rounded hover:bg-red-500/20 text-[rgba(226,226,224,0.4)] hover:text-red-400 transition-colors"
                  title="Удалить этот клип"
                >
                  <Trash2 class="w-3 h-3" />
                </button>
              </div>
            </div>
          </div>
        {/each}
      </div>
    {/if}

    <!-- БОЛЬШАЯ КНОПКА ЭКСПОРТА ВЫДЕЛЕННЫХ КЛИПОВ -->
    <div class="pt-2 border-t border-[rgba(226,226,224,0.08)] flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3">
      <div class="text-xs font-mono text-[rgba(226,226,224,0.6)] flex items-center gap-2">
        <Check class="w-4 h-4 text-[#23c55e]" />
        <span>Готово к нарезке в <strong>9:16</strong> со сгенерированными субтитрами Whisper</span>
      </div>

      <button
        type="button"
        on:click={handleExportManualClips}
        disabled={$fragmentsStore.length === 0 || isExporting || $pipelineStore.status === 'running'}
        class="btn btn-primary py-3.5 px-6 rounded-[3px] text-xs sm:text-sm font-bold tracking-wide flex items-center justify-center gap-2.5 shadow-lg shadow-[#5865f2]/25 disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer"
      >
        <Download class="w-4 h-4" />
        <span>
          {#if isExporting}
            Экспорт в процессе...
          {:else if $fragmentsStore.length > 0}
            Экспортировать {$fragmentsStore.length} {$fragmentsStore.length === 1 ? 'клип' : 'клипов'} в 9:16 (+ Whisper)
          {:else}
            Экспортировать клипы (выделите области)
          {/if}
        </span>
      </button>
    </div>
  </div>
</div>
