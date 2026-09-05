<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { pipelineStore } from '../stores/pipeline';
  import Timeline from './Timeline.svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';

  let videoEl: HTMLVideoElement;
  let videoSrc = '';
  let duration = 0;
  let currentTime = 0;

  // Reactively update videoSrc when pipelineStore.video changes
  $: {
    if ($pipelineStore.video?.path) {
      // In a real Tauri app, we need to convert the file path so the webview can load it
      try {
        const isTauri = typeof window !== 'undefined' && (window as any).__TAURI_INTERNALS__ !== undefined;
        if (isTauri) {
          videoSrc = convertFileSrc($pipelineStore.video.path);
        } else {
          // If in browser simulating, we can't load absolute paths. 
          // We'd rely on a Blob URL if we had it, but here we just pass an empty string or a placeholder
          videoSrc = ''; // Or a test video URL
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
      // Update duration in store if needed
      if ($pipelineStore.video) {
        // We can't mutate directly, but we let Timeline use our local 'duration'
      }
    }
  }

  function handleSeek(event: CustomEvent<{ time: number }>) {
    if (videoEl) {
      videoEl.currentTime = event.detail.time;
      // Optional: Play on seek if desired
      // videoEl.play().catch(() => {});
    }
  }
</script>

<div class="flex flex-col gap-4 bg-slate-900/60 border border-slate-800 rounded-2xl p-5 fade-in duration-300">
  <div class="relative w-full aspect-video bg-black rounded-xl overflow-hidden border border-slate-800 flex items-center justify-center shadow-lg">
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
        Ваш браузер не поддерживает элемент видео.
      </video>
    {:else}
      <div class="text-slate-500 font-mono text-sm flex flex-col items-center gap-2">
        <svg class="h-8 w-8 text-slate-600" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 10l4.553-2.276A1 1 0 0121 8.618v6.764a1 1 0 01-1.447.894L15 14M5 18h8a2 2 0 002-2V8a2 2 0 00-2-2H5a2 2 0 00-2 2v8a2 2 0 002 2z" />
        </svg>
        <span>Превью видео недоступно (требуется окружение Tauri)</span>
      </div>
    {/if}
  </div>

  <Timeline 
    videoDuration={duration || ($pipelineStore.video?.duration || 0)}
    {currentTime}
    on:seek={handleSeek}
  />
</div>
