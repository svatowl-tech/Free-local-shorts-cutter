<script lang="ts">
  import { pipelineStore } from '../stores/pipeline';
  import Timeline from './Timeline.svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';

  let videoEl: HTMLVideoElement;
  let videoSrc = '';
  let duration = 0;
  let currentTime = 0;

  $: {
    if ($pipelineStore.video?.path) {
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
    } else {
      videoSrc = '';
    }
  }

  function handleTimeUpdate() {
    if (videoEl) {
      currentTime = videoEl.currentTime;
    }
  }

  function handleLoadedMetadata() {
    if (videoEl) {
      duration = videoEl.duration;
    }
  }

  function handleSeek(event: CustomEvent<{ time: number }>) {
    if (videoEl) {
      videoEl.currentTime = event.detail.time;
    }
  }
</script>

<div class="panel p-6 flex flex-col gap-4">
  <div class="panel-title flex items-center justify-between">
    <span>Видеопросмотр & Таймлайн</span>
    <span class="label-mono">Предпросмотр</span>
  </div>

  <div class="relative w-full aspect-video bg-[#0c0c0e] rounded-[2px] overflow-hidden border border-[rgba(226,226,224,0.1)] flex items-center justify-center">
    {#if videoSrc}
      <!-- svelte-ignore a11y-media-has-caption -->
      <video
        bind:this={videoEl}
        src={videoSrc}
        class="w-full h-full object-contain"
        controls
        controlsList="nodownload"
        on:timeupdate={handleTimeUpdate}
        on:loadedmetadata={handleLoadedMetadata}
      >
        Ваш браузер не поддерживает видео.
      </video>
    {:else}
      <div class="text-[rgba(226,226,224,0.4)] font-mono text-xs flex flex-col items-center gap-2">
        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <rect x="2" y="6" width="14" height="12" rx="2"/>
          <path d="m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5"/>
        </svg>
        <span>Превью видео доступно в окружении приложения</span>
      </div>
    {/if}
  </div>

  <Timeline 
    videoDuration={duration || ($pipelineStore.video?.duration || 0)}
    {currentTime}
    on:seek={handleSeek}
  />
</div>
