<!-- src/lib/components/BumperModal.svelte -->
<script lang="ts">
  import { createEventDispatcher, onDestroy } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { pipelineStore } from '../stores/pipeline';
  import {
    renderBumperPreview,
    probeAudioFileDuration,
    exportBumperPresetBundle,
    importBumperPresetBundle,
    saveBumperConfig,
    loadBumperConfig,
    type BumperGlobalConfig,
    type BumperPreset,
    type BumperItemConfig
  } from '../ipc';
  import { isTauriRuntime, getFileName, formatErrorMessage } from '../utils/path';
  import {
    X,
    Upload,
    Download,
    Sparkles,
    Film,
    Music,
    Image as ImageIcon,
    Trash2,
    Plus,
    Check,
    RefreshCw,
    Play,
    Square,
    Sliders,
    Type
  } from '@lucide/svelte';

  export let show = false;
  const dispatch = createEventDispatcher();

  let activeTab: 'v916' | 'v169' = 'v916';
  let activeSubTab: { preroll: 'media' | 'text' | 'effects'; postroll: 'media' | 'text' | 'effects' } = {
    preroll: 'media',
    postroll: 'media'
  };

  const createDefaultItem = (title: string, sub: string): BumperItemConfig => ({
    enabled: true,
    mode: 'generate',
    custom_video_path: null,
    custom_audio_path: null,
    custom_audio_duration: null,
    sync_duration_to_audio: true,
    audio_volume: 1.0,
    audio_fade_in: true,
    audio_fade_out: true,
    title,
    subtitle: sub,
    duration_sec: 2.5,
    bg_mode: 'blur_frame',
    bg_video_path: null,
    bg_image_path: null,
    bg_color_primary: '#0f172a',
    bg_color_secondary: '#312e81',
    bg_blur: 12,
    bg_dim: 0.35,
    bg_scale: 1.1,
    bg_saturation: 1.0,
    bg_contrast: 1.0,
    bg_vignette: true,
    bg_video_speed: 1.0,
    logo_image_path: null,
    logo_scale: 0.45,
    logo_x_percent: 50,
    logo_y_percent: 36,
    logo_opacity: 1.0,
    logo_radius: 0,
    logo_shadow: true,
    title_color: '#FFFFFF',
    title_size: 36,
    subtitle_size: 18,
    subtitle_color: '#cbd5e1',
    outline_color: '#000000',
    outline_width: 2,
    plate_style: 'glass',
    plate_color: '#000000',
    plate_opacity: 0.55,
    title_x_percent: 50,
    title_y_percent: 74,
    animation: 'fade_zoom',
    transition: 'crossfade',
    overlay_png_base64: null
  });

  function normalizeItem(raw: Partial<BumperItemConfig> | undefined, fallbackTitle: string, fallbackSub: string): BumperItemConfig {
    const def = createDefaultItem(fallbackTitle, fallbackSub);
    if (!raw) return def;
    return {
      ...def,
      ...raw,
      sync_duration_to_audio: raw.sync_duration_to_audio ?? true,
      audio_volume: raw.audio_volume ?? 1.0,
      audio_fade_in: raw.audio_fade_in ?? true,
      audio_fade_out: raw.audio_fade_out ?? true,
      bg_mode: raw.bg_mode || (raw.bg_video_path ? 'custom_video' : raw.bg_image_path ? 'custom_image' : 'blur_frame'),
      bg_color_primary: raw.bg_color_primary || '#0f172a',
      bg_color_secondary: raw.bg_color_secondary || '#312e81',
      bg_blur: typeof raw.bg_blur === 'number' ? raw.bg_blur : 12,
      bg_dim: typeof raw.bg_dim === 'number' ? raw.bg_dim : 0.35,
      bg_scale: typeof raw.bg_scale === 'number' ? raw.bg_scale : 1.1,
      bg_saturation: typeof raw.bg_saturation === 'number' ? raw.bg_saturation : 1.0,
      bg_contrast: typeof raw.bg_contrast === 'number' ? raw.bg_contrast : 1.0,
      bg_vignette: raw.bg_vignette ?? true,
      bg_video_speed: typeof raw.bg_video_speed === 'number' ? raw.bg_video_speed : 1.0,
      logo_opacity: typeof raw.logo_opacity === 'number' ? raw.logo_opacity : 1.0,
      logo_radius: typeof raw.logo_radius === 'number' ? raw.logo_radius : 0,
      logo_shadow: raw.logo_shadow ?? true,
      subtitle_size: typeof raw.subtitle_size === 'number' ? raw.subtitle_size : 18,
      subtitle_color: raw.subtitle_color || '#cbd5e1',
      outline_color: raw.outline_color || '#000000',
      outline_width: typeof raw.outline_width === 'number' ? raw.outline_width : 2,
      plate_style: raw.plate_style || 'glass',
      plate_color: raw.plate_color || '#000000',
      plate_opacity: typeof raw.plate_opacity === 'number' ? raw.plate_opacity : 0.55,
      animation: raw.animation || 'fade_zoom'
    };
  }

  const createDefaultPreset = (id: string, name: string): BumperPreset => ({
    id,
    name,
    v916: {
      preroll: createDefaultItem('Студия озвучки', 'Озвучено специально для вас'),
      postroll: createDefaultItem('Подпишись на канал', 'Ставь лайк и пиши комментарий!')
    },
    v169: {
      preroll: createDefaultItem('Студия озвучки', 'Приятного просмотра'),
      postroll: createDefaultItem('Спасибо за просмотр!', 'Подписывайтесь на наши соцсети')
    },
    created_at: Date.now()
  });

  function normalizePreset(p: BumperPreset): BumperPreset {
    return {
      ...p,
      v916: {
        preroll: normalizeItem(p.v916?.preroll, 'Студия озвучки', 'Преролл 9:16'),
        postroll: normalizeItem(p.v916?.postroll, 'Подпишись на канал', 'Постролл 9:16')
      },
      v169: {
        preroll: normalizeItem(p.v169?.preroll, 'Студия озвучки', 'Преролл 16:9'),
        postroll: normalizeItem(p.v169?.postroll, 'Спасибо за просмотр', 'Постролл 16:9')
      }
    };
  }

  let config: BumperGlobalConfig = {
    enabled: true,
    active_preset_id: 'default',
    presets: [createDefaultPreset('default', 'Студия озвучки (По умолчанию)')]
  };

  let initializedFromStore = false;
  $: if (show && !initializedFromStore) {
    activeTab = $pipelineStore.config.vertical_format ? 'v916' : 'v169';
    if ($pipelineStore.config.bumper_config && $pipelineStore.config.bumper_config.presets?.length > 0) {
      const clone = JSON.parse(JSON.stringify($pipelineStore.config.bumper_config)) as BumperGlobalConfig;
      clone.presets = clone.presets.map(normalizePreset);
      config = clone;
    } else {
      config = {
        enabled: true,
        active_preset_id: 'default',
        presets: [createDefaultPreset('default', 'Студия озвучки (По умолчанию)')]
      };
      if (isTauriRuntime()) {
        loadBumperConfig().then((diskCfg) => {
          if (diskCfg && diskCfg.presets && diskCfg.presets.length > 0) {
            diskCfg.presets = diskCfg.presets.map(normalizePreset);
            config = diskCfg;
            pipelineStore.updateConfig({ bumper_config: config });
          }
        }).catch(() => {});
      }
    }
    initializedFromStore = true;
  } else if (!show) {
    initializedFromStore = false;
    stopLiveAudioPreview();
  }

  function handleSyncCurrentTabToOther() {
    stopLiveAudioPreview();
    const sourceKey = activeTab;
    const targetKey = activeTab === 'v916' ? 'v169' : 'v916';
    const clonedPreroll = JSON.parse(JSON.stringify(activePreset[sourceKey].preroll)) as BumperItemConfig;
    const clonedPostroll = JSON.parse(JSON.stringify(activePreset[sourceKey].postroll)) as BumperItemConfig;
    clonedPreroll.overlay_png_base64 = null;
    clonedPostroll.overlay_png_base64 = null;
    activePreset[targetKey] = {
      preroll: clonedPreroll,
      postroll: clonedPostroll
    };
    config = { ...config };
    saveConfigToStore();
    renderFeedback = `✓ Настройки заставок скопированы из ${sourceKey === 'v916' ? '9:16' : '16:9'} в формат ${targetKey === 'v916' ? '9:16' : '16:9'}!`;
    setTimeout(() => (renderFeedback = ''), 3500);
  }

  $: activePreset =
    config.presets.find((p) => p.id === config.active_preset_id) ||
    config.presets[0] ||
    createDefaultPreset('default', 'По умолчанию');
  $: category = activePreset[activeTab];

  let isRendering = false;
  let renderFeedback = '';
  let renderError = '';
  let newPresetName = '';
  let showNewPresetInput = false;

  let previewVideoUrl: { preroll: string | null; postroll: string | null } = {
    preroll: null,
    postroll: null
  };

  let isRenderingSingle: { preroll: boolean; postroll: boolean } = {
    preroll: false,
    postroll: false
  };

  // Интерактивное проигрывание превью прямо в окне со звуком и анимацией
  let isPlayingLive: { preroll: boolean; postroll: boolean } = {
    preroll: false,
    postroll: false
  };
  let liveProgress: { preroll: number; postroll: number } = {
    preroll: 0,
    postroll: 0
  };
  let activeAudioEl: HTMLAudioElement | null = null;
  let liveTimer: ReturnType<typeof setInterval> | null = null;

  onDestroy(() => {
    stopLiveAudioPreview();
  });

  function resolveMediaSrc(pathOrData?: string | null): string {
    if (!pathOrData) return '';
    if (
      pathOrData.startsWith('data:') ||
      pathOrData.startsWith('blob:') ||
      pathOrData.startsWith('http://') ||
      pathOrData.startsWith('https://')
    ) {
      return pathOrData;
    }
    if (isTauriRuntime()) {
      try {
        return convertFileSrc(pathOrData);
      } catch {
        return pathOrData;
      }
    }
    return pathOrData;
  }

  function hexToRgba(hex: string, alpha: number): string {
    const clean = (hex || '#000000').replace('#', '');
    if (clean.length === 6) {
      const r = parseInt(clean.slice(0, 2), 16) || 0;
      const g = parseInt(clean.slice(2, 4), 16) || 0;
      const b = parseInt(clean.slice(4, 6), 16) || 0;
      return `rgba(${r}, ${g}, ${b}, ${Math.max(0, Math.min(1, alpha))})`;
    }
    return `rgba(0, 0, 0, ${alpha})`;
  }

  function buildBgFilterCss(item: BumperItemConfig): string {
    const blur = item.bg_blur ?? 0;
    const sat = (item.bg_saturation ?? 1.0) * 100;
    const cont = (item.bg_contrast ?? 1.0) * 100;
    const bright = Math.max(10, Math.round((1 - (item.bg_dim ?? 0.35) * 0.85) * 100));
    return `blur(${Math.round(blur * 0.45)}px) saturate(${Math.round(sat)}%) contrast(${Math.round(cont)}%) brightness(${bright}%)`;
  }

  function stopLiveAudioPreview() {
    if (activeAudioEl) {
      activeAudioEl.pause();
      activeAudioEl = null;
    }
    if (liveTimer) {
      clearInterval(liveTimer);
      liveTimer = null;
    }
    isPlayingLive = { preroll: false, postroll: false };
    liveProgress = { preroll: 0, postroll: 0 };
  }

  function toggleLivePreviewPlay(slot: 'preroll' | 'postroll') {
    if (isPlayingLive[slot]) {
      stopLiveAudioPreview();
      return;
    }
    stopLiveAudioPreview();
    const item = category[slot];
    const totalDur = Math.max(0.5, item.duration_sec || 2.5);

    isPlayingLive[slot] = true;
    liveProgress[slot] = 0;

    if (item.custom_audio_path) {
      const src = resolveMediaSrc(item.custom_audio_path);
      if (src) {
        const audio = new Audio(src);
        audio.volume = Math.max(0, Math.min(1, item.audio_volume ?? 1.0));
        audio.play().catch((err) => console.warn('Audio preview play error:', err));
        activeAudioEl = audio;
      }
    }

    const startedAt = Date.now();
    liveTimer = setInterval(() => {
      const elapsed = (Date.now() - startedAt) / 1000;
      if (elapsed >= totalDur) {
        stopLiveAudioPreview();
      } else {
        liveProgress[slot] = Math.min(100, (elapsed / totalDur) * 100);
      }
    }, 40);
  }

  function saveConfigToStore() {
    config = { ...config, enabled: true };
    pipelineStore.updateConfig({ bumper_config: config });
    if (isTauriRuntime()) {
      saveBumperConfig(config).catch((err) => {
        console.warn('Не удалось сохранить конфигурацию заставок на диск:', err);
      });
    }
  }

  function handleClose() {
    stopLiveAudioPreview();
    saveConfigToStore();
    dispatch('close');
  }

  function handleGlobalToggle(e: Event) {
    config.enabled = (e.target as HTMLInputElement).checked;
    saveConfigToStore();
  }

  function handleSelectPreset(id: string) {
    stopLiveAudioPreview();
    config.active_preset_id = id;
    previewVideoUrl = { preroll: null, postroll: null };
    saveConfigToStore();
  }

  function handleCreatePreset() {
    if (!newPresetName.trim()) return;
    const newId = `preset_${Date.now().toString(36)}`;
    const newP = createDefaultPreset(newId, newPresetName.trim());
    config.presets = [...config.presets, newP];
    config.active_preset_id = newId;
    newPresetName = '';
    showNewPresetInput = false;
    saveConfigToStore();
  }

  function handleDeletePreset(id: string) {
    if (config.presets.length <= 1) return;
    config.presets = config.presets.filter((p) => p.id !== id);
    if (config.active_preset_id === id) {
      config.active_preset_id = config.presets[0].id;
    }
    saveConfigToStore();
  }

  /// Генерирует прозрачный PNG-оверлей с логотипом, плашкой и текстом в точном разрешении (1080x1920 или 1920x1080)
  async function buildOverlayPngBase64(item: BumperItemConfig, isVertical: boolean): Promise<string | null> {
    if (typeof document === 'undefined') return null;
    const w = isVertical ? 1080 : 1920;
    const h = isVertical ? 1920 : 1080;
    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext('2d');
    if (!ctx) return null;

    ctx.clearRect(0, 0, w, h);

    // 1. Отрисовка кастомного логотипа
    if (item.logo_image_path) {
      const logoSrc = resolveMediaSrc(item.logo_image_path);
      if (logoSrc) {
        await new Promise<void>((resolve) => {
          const img = new Image();
          img.crossOrigin = 'anonymous';
          img.onload = () => {
            const targetW = w * Math.max(0.08, Math.min(1.5, item.logo_scale || 0.45));
            const aspect = img.height > 0 ? img.width / img.height : 1;
            const targetH = targetW / Math.max(0.1, aspect);
            const cx = w * ((item.logo_x_percent ?? 50) / 100);
            const cy = h * ((item.logo_y_percent ?? 36) / 100);
            const x = cx - targetW / 2;
            const y = cy - targetH / 2;

            ctx.save();
            ctx.globalAlpha = Math.max(0.1, Math.min(1.0, item.logo_opacity ?? 1.0));
            if (item.logo_shadow ?? true) {
              ctx.shadowColor = 'rgba(0, 0, 0, 0.65)';
              ctx.shadowBlur = 28;
              ctx.shadowOffsetY = 8;
            }
            const radiusPct = item.logo_radius ?? 0;
            if (radiusPct > 0) {
              const r = Math.min(targetW, targetH) * (radiusPct / 100);
              ctx.beginPath();
              ctx.roundRect(x, y, targetW, targetH, r);
              ctx.clip();
            }
            ctx.drawImage(img, x, y, targetW, targetH);
            ctx.restore();
            resolve();
          };
          img.onerror = () => resolve();
          img.src = logoSrc;
        });
      }
    }

    // 2. Отрисовка плашки, заголовка и подзаголовка
    const hasTitle = Boolean(item.title && item.title.trim());
    const hasSub = Boolean(item.subtitle && item.subtitle.trim());
    if (hasTitle || hasSub) {
      const scaleFactor = w / 1080;
      const titlePx = Math.round((item.title_size || 36) * scaleFactor * 1.35);
      const subPx = Math.round((item.subtitle_size || 18) * scaleFactor * 1.25);

      ctx.save();
      ctx.font = `700 ${titlePx}px sans-serif`;
      const titleMetrics = hasTitle ? ctx.measureText(item.title.trim()) : { width: 0 };

      ctx.font = `500 ${subPx}px monospace`;
      const subMetrics = hasSub ? ctx.measureText((item.subtitle || '').trim()) : { width: 0 };

      const contentW = Math.max(titleMetrics.width, subMetrics.width);
      const padX = Math.round(44 * scaleFactor);
      const padY = Math.round(28 * scaleFactor);
      const gapY = hasTitle && hasSub ? Math.round(14 * scaleFactor) : 0;
      const totalTextH = (hasTitle ? titlePx : 0) + gapY + (hasSub ? subPx : 0);

      const cx = w * ((item.title_x_percent ?? 50) / 100);
      const cy = h * ((item.title_y_percent ?? 74) / 100);
      const boxW = contentW + padX * 2;
      const boxH = totalTextH + padY * 2;
      const boxX = cx - boxW / 2;
      const boxY = cy - boxH / 2;

      if (item.plate_style !== 'none') {
        ctx.fillStyle = hexToRgba(item.plate_color || '#000000', item.plate_opacity ?? 0.55);
        ctx.beginPath();
        ctx.roundRect(boxX, boxY, boxW, boxH, Math.round(18 * scaleFactor));
        ctx.fill();
        if (item.plate_style === 'glass') {
          ctx.strokeStyle = 'rgba(255, 255, 255, 0.16)';
          ctx.lineWidth = Math.max(2, Math.round(2 * scaleFactor));
          ctx.stroke();
        }
      }

      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';
      let curY = boxY + padY;

      if (hasTitle) {
        ctx.font = `700 ${titlePx}px sans-serif`;
        if ((item.outline_width ?? 2) > 0) {
          ctx.strokeStyle = item.outline_color || '#000000';
          ctx.lineWidth = (item.outline_width ?? 2) * scaleFactor * 2;
          ctx.lineJoin = 'round';
          ctx.strokeText(item.title.trim(), cx, curY);
        }
        ctx.fillStyle = item.title_color || '#FFFFFF';
        ctx.fillText(item.title.trim(), cx, curY);
        curY += titlePx + gapY;
      }

      if (hasSub && item.subtitle) {
        ctx.font = `500 ${subPx}px monospace`;
        if ((item.outline_width ?? 2) > 0) {
          ctx.strokeStyle = item.outline_color || '#000000';
          ctx.lineWidth = Math.max(2, (item.outline_width ?? 2) * scaleFactor * 1.2);
          ctx.lineJoin = 'round';
          ctx.strokeText(item.subtitle.trim(), cx, curY);
        }
        ctx.fillStyle = item.subtitle_color || '#cbd5e1';
        ctx.fillText(item.subtitle.trim(), cx, curY);
      }

      ctx.restore();
    }

    return canvas.toDataURL('image/png');
  }

  async function handleRenderAndPlaySingle(bumperType: 'preroll' | 'postroll') {
    stopLiveAudioPreview();
    const item = category[bumperType];
    const isVertical = activeTab === 'v916';
    const sourceVideo = $pipelineStore.video?.path || '';

    isRenderingSingle[bumperType] = true;
    renderError = '';
    try {
      config.enabled = true;
      const overlayPng = await buildOverlayPngBase64(item, isVertical);
      item.overlay_png_base64 = overlayPng;
      saveConfigToStore();

      const videoPath = await renderBumperPreview(item, isVertical, sourceVideo, activePreset.id, bumperType);
      previewVideoUrl[bumperType] = isTauriRuntime() ? `${ resolveMediaSrc(videoPath) }?t=${Date.now()}` : videoPath;
      renderFeedback = `✓ ${bumperType === 'preroll' ? 'Преролл' : 'Постролл'} (${isVertical ? '9:16' : '16:9'}) успешно сгенерирован (${item.duration_sec.toFixed(1)}с)`;
      setTimeout(() => (renderFeedback = ''), 3500);
    } catch (err: unknown) {
      renderError = formatErrorMessage(err);
    } finally {
      isRenderingSingle[bumperType] = false;
    }
  }

  async function handlePreRenderAll() {
    if (isRendering) return;
    stopLiveAudioPreview();
    isRendering = true;
    renderFeedback = '';
    renderError = '';

    const sourceVideo = $pipelineStore.video?.path || '';

    try {
      config.enabled = true;
      const curPreset = activePreset;
      if (curPreset.v916.preroll.enabled) {
        renderFeedback = '⚡ Генерация заставки 9:16 (Преролл)...';
        curPreset.v916.preroll.overlay_png_base64 = await buildOverlayPngBase64(curPreset.v916.preroll, true);
        await renderBumperPreview(curPreset.v916.preroll, true, sourceVideo, curPreset.id, 'preroll');
      }

      if (curPreset.v916.postroll.enabled) {
        renderFeedback = '⚡ Генерация заставки 9:16 (Постролл)...';
        curPreset.v916.postroll.overlay_png_base64 = await buildOverlayPngBase64(curPreset.v916.postroll, true);
        await renderBumperPreview(curPreset.v916.postroll, true, sourceVideo, curPreset.id, 'postroll');
      }

      if (curPreset.v169.preroll.enabled) {
        renderFeedback = '⚡ Генерация заставки 16:9 (Преролл)...';
        curPreset.v169.preroll.overlay_png_base64 = await buildOverlayPngBase64(curPreset.v169.preroll, false);
        await renderBumperPreview(curPreset.v169.preroll, false, sourceVideo, curPreset.id, 'preroll');
      }

      if (curPreset.v169.postroll.enabled) {
        renderFeedback = '⚡ Генерация заставки 16:9 (Постролл)...';
        curPreset.v169.postroll.overlay_png_base64 = await buildOverlayPngBase64(curPreset.v169.postroll, false);
        await renderBumperPreview(curPreset.v169.postroll, false, sourceVideo, curPreset.id, 'postroll');
      }

      saveConfigToStore();
      renderFeedback = '✓ Все заставки (9:16 и 16:9) предрендерены в MP4 и активированы для экспорта!';
      setTimeout(() => {
        renderFeedback = '';
      }, 4500);
    } catch (err: unknown) {
      renderError = formatErrorMessage(err);
    } finally {
      isRendering = false;
    }
  }

  /// Клиентская сборка стандартного ZIP-архива (.vcpb) для браузерного режима
  function buildClientZipArchive(entries: Array<{ name: string; data: Uint8Array }>): Uint8Array {
    const crcTable = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) {
        c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      }
      crcTable[n] = c >>> 0;
    }
    const crc32 = (bytes: Uint8Array): number => {
      let crc = 0xffffffff;
      for (let i = 0; i < bytes.length; i++) {
        crc = crcTable[(crc ^ bytes[i]) & 0xff] ^ (crc >>> 8);
      }
      return (crc ^ 0xffffffff) >>> 0;
    };

    const encoder = new TextEncoder();
    const localChunks: Uint8Array[] = [];
    const centralChunks: Uint8Array[] = [];
    let offset = 0;

    for (const entry of entries) {
      const nameBytes = encoder.encode(entry.name);
      const crc = crc32(entry.data);
      const size = entry.data.length;

      const localHeader = new Uint8Array(30 + nameBytes.length);
      const lv = new DataView(localHeader.buffer);
      lv.setUint32(0, 0x04034b50, true);
      lv.setUint16(4, 20, true);
      lv.setUint16(6, 0x0800, true);
      lv.setUint16(8, 0, true);
      lv.setUint32(14, crc, true);
      lv.setUint32(18, size, true);
      lv.setUint32(22, size, true);
      lv.setUint16(26, nameBytes.length, true);
      localHeader.set(nameBytes, 30);

      localChunks.push(localHeader, entry.data);

      const centralHeader = new Uint8Array(46 + nameBytes.length);
      const cv = new DataView(centralHeader.buffer);
      cv.setUint32(0, 0x02014b50, true);
      cv.setUint16(4, 20, true);
      cv.setUint16(6, 20, true);
      cv.setUint16(8, 0x0800, true);
      cv.setUint16(10, 0, true);
      cv.setUint32(16, crc, true);
      cv.setUint32(20, size, true);
      cv.setUint32(24, size, true);
      cv.setUint16(28, nameBytes.length, true);
      cv.setUint32(42, offset, true);
      centralHeader.set(nameBytes, 46);
      centralChunks.push(centralHeader);

      offset += localHeader.length + size;
    }

    const cdSize = centralChunks.reduce((acc, c) => acc + c.length, 0);
    const eocd = new Uint8Array(22);
    const ev = new DataView(eocd.buffer);
    ev.setUint32(0, 0x06054b50, true);
    ev.setUint16(8, entries.length, true);
    ev.setUint16(10, entries.length, true);
    ev.setUint32(12, cdSize, true);
    ev.setUint32(16, offset, true);

    const totalLen = offset + cdSize + eocd.length;
    const result = new Uint8Array(totalLen);
    let pos = 0;
    for (const chunk of localChunks) {
      result.set(chunk, pos);
      pos += chunk.length;
    }
    for (const chunk of centralChunks) {
      result.set(chunk, pos);
      pos += chunk.length;
    }
    result.set(eocd, pos);
    return result;
  }

  async function handleExportPresetArchive() {
    renderError = '';
    const safeName = activePreset.name.replace(/[^a-zA-Z0-9а-яА-Я_-]/g, '_');

    if (isTauriRuntime()) {
      try {
        const { save } = await import('@tauri-apps/plugin-dialog');
        const dest = await save({
          defaultPath: `preset_${safeName}.vcpb`,
          filters: [
            {
              name: 'Архив пресета заставок (.vcpb / .zip)',
              extensions: ['vcpb', 'zip']
            }
          ]
        });
        if (dest) {
          await exportBumperPresetBundle(activePreset, dest);
          renderFeedback = `✓ Пресет и все медиафайлы упакованы в архив: ${getFileName(dest, 'preset.vcpb')}`;
          setTimeout(() => (renderFeedback = ''), 4000);
        }
        return;
      } catch (err: unknown) {
        console.warn('Fallback to browser .vcpb download:', formatErrorMessage(err));
      }
    }

    // Браузерный экспорт .vcpb (ZIP-архив с preset.json)
    const jsonBytes = new TextEncoder().encode(JSON.stringify(activePreset, null, 2));
    const zipBytes = buildClientZipArchive([{ name: 'preset.json', data: jsonBytes }]);
    const blob = new Blob([zipBytes], { type: 'application/zip' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `preset_${safeName}.vcpb`;
    a.click();
    URL.revokeObjectURL(url);
    renderFeedback = '✓ Архив пресета (.vcpb) сохранён!';
    setTimeout(() => (renderFeedback = ''), 3500);
  }

  async function handleImportPresetArchiveClick() {
    if (isTauriRuntime()) {
      try {
        const { open } = await import('@tauri-apps/plugin-dialog');
        const selected = await open({
          multiple: false,
          filters: [
            {
              name: 'Архив пресета (.vcpb, .zip, .json)',
              extensions: ['vcpb', 'zip', 'vcp-preset', 'json']
            }
          ]
        });
        if (selected && typeof selected === 'string') {
          const imported = await importBumperPresetBundle(selected);
          const norm = normalizePreset(imported);
          norm.name = `${norm.name} (Импорт)`;
          config.presets = [...config.presets, norm];
          config.active_preset_id = norm.id;
          saveConfigToStore();
          renderFeedback = `✓ Пресет "${norm.name}" со всеми медиафайлами импортирован!`;
          setTimeout(() => (renderFeedback = ''), 4000);
          return;
        }
      } catch (err: unknown) {
        renderError = formatErrorMessage(err);
      }
    } else {
      document.getElementById('bumper_preset_import_input')?.click();
    }
  }

  async function handleImportPresetFileBrowser(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    try {
      const buf = new Uint8Array(await file.arrayBuffer());
      let jsonStr = '';
      if (buf.length > 30 && buf[0] === 0x50 && buf[1] === 0x4b) {
        // Читаем preset.json из локального заголовка ZIP
        let pos = 0;
        const decoder = new TextDecoder();
        while (pos + 30 <= buf.length) {
          const view = new DataView(buf.buffer, buf.byteOffset + pos);
          if (view.getUint32(0, true) !== 0x04034b50) break;
          const compSize = view.getUint32(18, true);
          const nameLen = view.getUint16(26, true);
          const extraLen = view.getUint16(28, true);
          const name = decoder.decode(buf.subarray(pos + 30, pos + 30 + nameLen));
          const dataStart = pos + 30 + nameLen + extraLen;
          if (name === 'preset.json') {
            jsonStr = decoder.decode(buf.subarray(dataStart, dataStart + compSize));
            break;
          }
          pos = dataStart + compSize;
        }
      } else {
        jsonStr = new TextDecoder().decode(buf);
      }

      const parsed = JSON.parse(jsonStr) as BumperPreset;
      if (parsed && parsed.v916 && parsed.v169) {
        const norm = normalizePreset(parsed);
        norm.id = `preset_imp_${Date.now().toString(36)}`;
        norm.name = `${norm.name} (Импорт)`;
        config.presets = [...config.presets, norm];
        config.active_preset_id = norm.id;
        saveConfigToStore();
        renderFeedback = `✓ Пресет "${norm.name}" успешно импортирован!`;
        setTimeout(() => (renderFeedback = ''), 3500);
      }
    } catch {
      renderError = 'Не удалось прочитать файл архива пресета (.vcpb / .zip)';
    }
  }

  function triggerFileInput(id: string) {
    document.getElementById(id)?.click();
  }

  /// Определяет длину аудиофайла через Web Audio / HTMLAudioElement и синхронизирует длительность бампера
  function detectAudioDurationFromUrl(url: string): Promise<number> {
    return new Promise((resolve) => {
      const audio = new Audio();
      const timeout = setTimeout(() => resolve(0), 3500);
      audio.onloadedmetadata = () => {
        clearTimeout(timeout);
        if (audio.duration && isFinite(audio.duration) && audio.duration > 0.1) {
          resolve(Math.round(audio.duration * 100) / 100);
        } else {
          resolve(0);
        }
      };
      audio.onerror = () => {
        clearTimeout(timeout);
        resolve(0);
      };
      audio.src = url;
    });
  }

  async function handleCustomAudioSelected(e: Event, slot: 'preroll' | 'postroll') {
    const file = (e.target as HTMLInputElement).files?.[0] as (File & { path?: string }) | undefined;
    if (!file) return;

    const item = category[slot];
    const objectUrl = URL.createObjectURL(file);
    let detectedDur = await detectAudioDurationFromUrl(objectUrl);
    URL.revokeObjectURL(objectUrl);

    const reader = new FileReader();
    reader.onload = async (ev) => {
      const dataUrl = ev.target?.result as string;
      const storedPath = file.path || dataUrl;
      item.custom_audio_path = storedPath;

      if (detectedDur <= 0.1 && file.path && isTauriRuntime()) {
        detectedDur = await probeAudioFileDuration(file.path);
      }

      if (detectedDur > 0.1) {
        item.custom_audio_duration = detectedDur;
        if (item.sync_duration_to_audio ?? true) {
          item.duration_sec = detectedDur;
        }
        renderFeedback = `🎵 Загружен звук "${file.name}" (${detectedDur.toFixed(2)} сек) — длина ${slot === 'preroll' ? 'преролла' : 'постролла'} установлена по звуку!`;
        setTimeout(() => (renderFeedback = ''), 4500);
      }
      previewVideoUrl[slot] = null;
      saveConfigToStore();
    };
    reader.readAsDataURL(file);
  }

  function handleSyncDurationToAudioToggle(slot: 'preroll' | 'postroll') {
    const item = category[slot];
    if (item.sync_duration_to_audio && item.custom_audio_duration && item.custom_audio_duration > 0.1) {
      item.duration_sec = item.custom_audio_duration;
    }
    previewVideoUrl[slot] = null;
    saveConfigToStore();
  }

  function handleMediaUpload(
    e: Event,
    slot: 'preroll' | 'postroll',
    targetField: 'custom_video_path' | 'bg_video_path' | 'bg_image_path' | 'logo_image_path'
  ) {
    const file = (e.target as HTMLInputElement).files?.[0] as (File & { path?: string }) | undefined;
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (ev) => {
      const dataUrl = ev.target?.result as string;
      const item = category[slot];
      item[targetField] = file.path || dataUrl;
      if (targetField === 'bg_video_path') {
        item.bg_mode = 'custom_video';
      } else if (targetField === 'bg_image_path') {
        item.bg_mode = 'custom_image';
      }
      previewVideoUrl[slot] = null;
      saveConfigToStore();
    };
    reader.readAsDataURL(file);
  }
</script>

{#if show}
  <!-- Backdrop -->
  <div class="fixed inset-0 bg-black/80 backdrop-blur-sm z-50 flex items-center justify-center p-3 sm:p-5">
    <!-- Modal Shell -->
    <div class="bg-[#121215] border border-[rgba(226,226,224,0.16)] rounded-lg w-full max-w-6xl max-h-[92vh] flex flex-col shadow-2xl overflow-hidden">
      
      <!-- Modal Header -->
      <div class="px-5 py-4 border-b border-[rgba(226,226,224,0.1)] flex items-center justify-between bg-[#18181c]">
        <div class="flex items-center gap-3">
          <div class="w-8 h-8 rounded bg-[#5865f2]/20 border border-[#5865f2]/50 flex items-center justify-center text-[#5865f2]">
            <Film class="w-4 h-4" />
          </div>
          <div>
            <h2 class="text-sm sm:text-base font-semibold text-white">Студия Преролла и Постролла (Заставки 9:16 и 16:9)</h2>
            <p class="text-[11px] text-slate-400 font-mono">Полный контроль звука, логотипа, фонового видео с эффектами и живое превью</p>
          </div>
        </div>

        <div class="flex items-center gap-3">
          <label class="flex items-center gap-2 cursor-pointer select-none px-3 py-1.5 bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] rounded">
            <input 
              type="checkbox" 
              checked={config.enabled} 
              on:change={handleGlobalToggle} 
              class="w-4 h-4 text-[#5865f2] rounded accent-[#5865f2]" 
            />
            <span class="text-xs font-mono text-white">Активно при экспорте</span>
          </label>

          <button 
            type="button"
            on:click={handleClose} 
            class="text-slate-400 hover:text-white p-1.5 rounded hover:bg-[#222228] transition-colors cursor-pointer"
          >
            <X class="w-5 h-5" />
          </button>
        </div>
      </div>

      <!-- Presets Toolbar -->
      <div class="px-5 py-2.5 bg-[#0c0c0e] border-b border-[rgba(226,226,224,0.1)] flex flex-wrap items-center justify-between gap-3 text-xs">
        <div class="flex flex-wrap items-center gap-2.5">
          <span class="font-mono text-slate-400">Пресет:</span>
          <select 
            value={config.active_preset_id} 
            on:change={(e) => handleSelectPreset(e.currentTarget.value)}
            class="bg-[#18181c] border border-[rgba(226,226,224,0.2)] text-white font-mono rounded px-3 py-1.5 focus:border-[#5865f2] outline-none cursor-pointer"
          >
            {#each config.presets as p}
              <option value={p.id}>{p.name}</option>
            {/each}
          </select>

          {#if showNewPresetInput}
            <div class="flex items-center gap-1.5">
              <input 
                type="text" 
                placeholder="Имя нового пресета..." 
                bind:value={newPresetName}
                class="bg-[#18181c] border border-[#5865f2] text-white px-2.5 py-1 rounded text-xs font-mono outline-none"
              />
              <button type="button" on:click={handleCreatePreset} class="p-1.5 bg-[#5865f2] text-white rounded hover:bg-[#4752c4] cursor-pointer">
                <Check class="w-3.5 h-3.5" />
              </button>
            </div>
          {:else}
            <button 
              type="button"
              on:click={() => (showNewPresetInput = true)} 
              class="flex items-center gap-1 px-2.5 py-1.5 bg-[#18181c] hover:bg-[#222228] border border-[rgba(226,226,224,0.15)] text-slate-300 rounded font-mono cursor-pointer"
            >
              <Plus class="w-3.5 h-3.5" />
              <span>Создать пресет</span>
            </button>
          {/if}

          {#if config.presets.length > 1}
            <button 
              type="button"
              on:click={() => handleDeletePreset(config.active_preset_id)}
              class="p-1.5 text-red-400 hover:text-red-300 hover:bg-red-500/10 rounded cursor-pointer"
              title="Удалить пресет"
            >
              <Trash2 class="w-3.5 h-3.5" />
            </button>
          {/if}
        </div>

        <div class="flex flex-wrap items-center gap-2">
          <!-- Pre-render All Button -->
          <button 
            type="button"
            on:click={handlePreRenderAll}
            disabled={isRendering}
            class="flex items-center gap-1.5 px-3 py-1.5 bg-[#5865f2] hover:bg-[#4752c4] text-white font-mono font-medium rounded transition-colors disabled:opacity-50 cursor-pointer"
          >
            {#if isRendering}
              <RefreshCw class="w-3.5 h-3.5 animate-spin" />
              <span>Рендеринг MP4...</span>
            {:else}
              <Sparkles class="w-3.5 h-3.5" />
              <span>⚡ Предрендерить заставки в MP4</span>
            {/if}
          </button>

          <!-- Export Preset (.vcpb / .zip) -->
          <button 
            type="button"
            on:click={handleExportPresetArchive}
            class="flex items-center gap-1 px-2.5 py-1.5 bg-[#18181c] hover:bg-[#222228] border border-[rgba(226,226,224,0.15)] text-slate-300 rounded font-mono cursor-pointer"
            title="Экспортировать пресет со всеми звуками, логотипами и фонами в единый ZIP-архив (.vcpb)"
          >
            <Download class="w-3.5 h-3.5 text-[#5865f2]" />
            <span>Экспорт (.vcpb)</span>
          </button>

          <!-- Import Preset (.vcpb / .zip) -->
          <button 
            type="button"
            on:click={handleImportPresetArchiveClick}
            class="flex items-center gap-1 px-2.5 py-1.5 bg-[#18181c] hover:bg-[#222228] border border-[rgba(226,226,224,0.15)] text-slate-300 rounded font-mono cursor-pointer"
            title="Импортировать архив пресета (.vcpb / .zip)"
          >
            <Upload class="w-3.5 h-3.5 text-[#5865f2]" />
            <span>Импорт (.vcpb)</span>
          </button>
          <input
            id="bumper_preset_import_input"
            type="file"
            accept=".vcpb,.zip,.vcp-preset,.json"
            on:change={handleImportPresetFileBrowser}
            class="hidden"
          />
        </div>
      </div>

      {#if renderFeedback}
        <div class="px-5 py-2 bg-emerald-500/10 border-b border-emerald-500/20 text-emerald-400 text-xs font-mono">
          {renderFeedback}
        </div>
      {/if}
      {#if renderError}
        <div class="px-5 py-2 bg-red-500/10 border-b border-red-500/20 text-red-400 text-xs font-mono">
          {renderError}
        </div>
      {/if}

      <!-- Format Tabs (9:16 vs 16:9) -->
      <div class="flex items-center border-b border-[rgba(226,226,224,0.1)] bg-[#16161a]">
        <button 
          type="button"
          on:click={() => {
            stopLiveAudioPreview();
            activeTab = 'v916';
            previewVideoUrl = { preroll: null, postroll: null };
          }}
          class="flex-1 py-2.5 text-xs font-mono font-medium text-center border-b-2 transition-colors cursor-pointer {
            activeTab === 'v916' 
              ? 'border-[#5865f2] text-[#5865f2] bg-[#121215]' 
              : 'border-transparent text-slate-400 hover:text-white'
          }"
        >
          📱 Шортсы / Reels / TikTok (9:16 Вертикальные)
        </button>

        <button 
          type="button"
          on:click={() => {
            stopLiveAudioPreview();
            activeTab = 'v169';
            previewVideoUrl = { preroll: null, postroll: null };
          }}
          class="flex-1 py-2.5 text-xs font-mono font-medium text-center border-b-2 transition-colors cursor-pointer {
            activeTab === 'v169' 
              ? 'border-[#5865f2] text-[#5865f2] bg-[#121215]' 
              : 'border-transparent text-slate-400 hover:text-white'
          }"
        >
          🎬 Хайлайты / Ток-видео / YouTube (16:9 Горизонтальные)
        </button>

        <button
          type="button"
          on:click={handleSyncCurrentTabToOther}
          title="Скопировать настройки преролла и постролла из текущей вкладки во вторую ориентацию (9:16 ↔ 16:9)"
          class="px-3 py-2 mx-2 text-[11px] font-mono bg-[#18181c] hover:bg-[#5865f2]/20 border border-[rgba(226,226,224,0.18)] hover:border-[#5865f2] text-slate-300 hover:text-white rounded transition-colors cursor-pointer shrink-0"
        >
          ⇄ Скопировать в {activeTab === 'v916' ? '16:9' : '9:16'}
        </button>
      </div>

      <!-- Tab Content Area: Two Independent Columns (PREROLL & POSTROLL) -->
      <div class="p-4 sm:p-6 overflow-y-auto flex-1">
        <div class="grid grid-cols-1 lg:grid-cols-2 gap-6">
          
          {#each (['preroll', 'postroll'] as const) as slot}
            {@const item = category[slot]}
            {@const slotLabel = slot === 'preroll' ? '◀ Преролл (В начале ролика)' : '▶ Постролл (В конце ролика)'}
            <div class="bg-[#18181c] border border-[rgba(226,226,224,0.12)] rounded-lg p-4 sm:p-5 flex flex-col gap-4">
              
              <!-- Slot Header -->
              <div class="flex items-center justify-between pb-2.5 border-b border-[rgba(226,226,224,0.1)]">
                <div class="flex items-center gap-2">
                  <span class="text-xs font-mono font-bold uppercase tracking-wider text-[#5865f2]">
                    {slotLabel}
                  </span>
                  <span class="px-2 py-0.5 rounded bg-[#0c0c0e] border border-[rgba(226,226,224,0.12)] text-[10px] font-mono text-slate-300 tabular-nums">
                    {item.duration_sec.toFixed(2)} сек
                  </span>
                </div>
                <label class="flex items-center gap-2 cursor-pointer select-none">
                  <input 
                    type="checkbox" 
                    bind:checked={item.enabled} 
                    on:change={saveConfigToStore}
                    class="w-4 h-4 text-[#5865f2] rounded accent-[#5865f2]" 
                  />
                  <span class="text-xs font-mono text-slate-200">{item.enabled ? 'Включен' : 'Выключен'}</span>
                </label>
              </div>

              {#if item.enabled}
                <!-- LIVE INTERACTIVE VISUAL PREVIEW BOX -->
                <div class="relative bg-black rounded-lg overflow-hidden border border-[rgba(226,226,224,0.2)] flex flex-col items-center justify-center shadow-inner">
                  <div class="relative w-full overflow-hidden flex items-center justify-center bg-[#09090c] {activeTab === 'v916' ? 'aspect-[9/16] max-h-[290px]' : 'aspect-[16/9] max-h-[235px]'}">
                    
                    {#if previewVideoUrl[slot]}
                      <video 
                        src={previewVideoUrl[slot]} 
                        controls 
                        autoplay 
                        class="w-full h-full object-contain bg-black"
                      >
                        <track kind="captions" />
                      </video>
                      <button
                        type="button"
                        on:click={() => (previewVideoUrl[slot] = null)}
                        class="absolute top-2 right-2 px-2 py-0.5 bg-black/80 hover:bg-black text-[10px] font-mono text-slate-200 rounded border border-white/20 cursor-pointer"
                      >
                        ← К живому конструктору
                      </button>
                    {:else if item.mode === 'custom_video' && item.custom_video_path}
                      <video 
                        src={resolveMediaSrc(item.custom_video_path)} 
                        controls 
                        class="w-full h-full object-contain bg-black"
                      >
                        <track kind="captions" />
                      </video>
                    {:else}
                      <!-- 1. Задний план с эффектами в реальном времени -->
                      {#if item.bg_mode === 'custom_video' && item.bg_video_path}
                        <video
                          src={resolveMediaSrc(item.bg_video_path)}
                          autoplay
                          loop
                          muted
                          playsinline
                          class="absolute inset-0 w-full h-full object-cover transition-transform duration-150"
                          style="filter: {buildBgFilterCss(item)}; transform: scale({item.bg_scale ?? 1.1});"
                        ></video>
                      {:else if item.bg_mode === 'custom_image' && item.bg_image_path}
                        <img
                          src={resolveMediaSrc(item.bg_image_path)}
                          alt="Background"
                          class="absolute inset-0 w-full h-full object-cover transition-transform duration-150"
                          style="filter: {buildBgFilterCss(item)}; transform: scale({item.bg_scale ?? 1.1});"
                        />
                      {:else if item.bg_mode === 'blur_frame' && ($pipelineStore.video?.objectUrl || $pipelineStore.video?.path)}
                        <video
                          src={$pipelineStore.video?.objectUrl || resolveMediaSrc($pipelineStore.video?.path)}
                          muted
                          playsinline
                          class="absolute inset-0 w-full h-full object-cover transition-transform duration-150"
                          style="filter: {buildBgFilterCss(item)}; transform: scale({item.bg_scale ?? 1.1});"
                        ></video>
                      {:else}
                        <div
                          class="absolute inset-0 transition-transform duration-150"
                          style="background: linear-gradient(135deg, {item.bg_color_primary || '#0f172a'}, {item.bg_color_secondary || '#312e81'}); filter: {buildBgFilterCss(item)}; transform: scale({item.bg_scale ?? 1.1});"
                        ></div>
                      {/if}

                      <!-- Слой затемнения (Dim) и Виньетки (Vignette) -->
                      <div
                        class="absolute inset-0 pointer-events-none"
                        style="background-color: rgba(0,0,0,{(item.bg_dim ?? 0.35) * 0.65}); {item.bg_vignette ? 'box-shadow: inset 0 0 65px rgba(0,0,0,0.85);' : ''}"
                      ></div>

                      <!-- 2. Кастомный логотип с масштабом, позицией X/Y, скруглением и тенью -->
                      {#if item.logo_image_path}
                        <div 
                          class="absolute transition-all duration-75 pointer-events-none {isPlayingLive[slot] && item.animation === 'pulse' ? 'animate-pulse' : ''}"
                          style="
                            left: {item.logo_x_percent ?? 50}%;
                            top: {item.logo_y_percent ?? 36}%;
                            width: {Math.round((item.logo_scale || 0.45) * 100)}%;
                            opacity: {item.logo_opacity ?? 1.0};
                            transform: translate(-50%, -50%) {isPlayingLive[slot] && item.animation === 'fade_zoom' ? 'scale(1.06)' : 'scale(1)'};
                          "
                        >
                          <img
                            src={resolveMediaSrc(item.logo_image_path)}
                            alt="Logo"
                            class="w-full h-auto max-h-[150px] object-contain mx-auto"
                            style="
                              border-radius: {item.logo_radius ?? 0}%;
                              filter: {item.logo_shadow !== false ? 'drop-shadow(0 8px 18px rgba(0,0,0,0.75))' : 'none'};
                            "
                          />
                        </div>
                      {/if}

                      <!-- 3. Настраиваемый текст и плашка в реальном времени -->
                      {#if item.title || item.subtitle}
                        <div 
                          class="absolute transition-all duration-75 text-center max-w-[92%] px-3.5 py-2 rounded-md pointer-events-none"
                          style="
                            left: {item.title_x_percent ?? 50}%;
                            top: {item.title_y_percent ?? 74}%;
                            transform: translate(-50%, -50%);
                            background-color: {item.plate_style === 'none' ? 'transparent' : hexToRgba(item.plate_color || '#000000', item.plate_opacity ?? 0.55)};
                            backdrop-filter: {item.plate_style === 'glass' ? 'blur(8px)' : 'none'};
                            border: {item.plate_style === 'glass' ? '1px solid rgba(255,255,255,0.14)' : 'none'};
                          "
                        >
                          {#if item.title}
                            <div 
                              class="font-bold leading-tight"
                              style="
                                color: {item.title_color || '#FFFFFF'};
                                font-size: {Math.max(11, Math.round((item.title_size || 36) * (activeTab === 'v916' ? 0.44 : 0.40)))}px;
                                text-shadow: {(item.outline_width ?? 2) > 0 ? `0 2px 6px ${item.outline_color || '#000000'}, 0 0 ${(item.outline_width ?? 2) * 2}px ${item.outline_color || '#000000'}` : 'none'};
                              "
                            >
                              {item.title}
                            </div>
                          {/if}
                          {#if item.subtitle}
                            <div
                              class="font-mono mt-0.5 leading-snug"
                              style="
                                color: {item.subtitle_color || '#cbd5e1'};
                                font-size: {Math.max(9, Math.round((item.subtitle_size || 18) * (activeTab === 'v916' ? 0.48 : 0.44)))}px;
                              "
                            >
                              {item.subtitle}
                            </div>
                          {/if}
                        </div>
                      {/if}

                      <!-- Live Indicator Badge -->
                      <div class="absolute top-2 left-2 px-2 py-0.5 bg-black/70 backdrop-blur text-[10px] font-mono text-slate-200 rounded border border-white/10 flex items-center gap-1.5 pointer-events-none select-none">
                        <span class="w-1.5 h-1.5 rounded-full {isPlayingLive[slot] ? 'bg-amber-400 animate-ping' : 'bg-emerald-400 animate-pulse'}"></span>
                        <span class="tabular-nums">
                          {isPlayingLive[slot] ? `Проигрывание (${item.duration_sec.toFixed(1)}с)` : `Живое превью · ${item.duration_sec.toFixed(1)}с`}
                        </span>
                      </div>

                      {#if isPlayingLive[slot]}
                        <div class="absolute bottom-0 left-0 right-0 h-1 bg-white/15">
                          <div class="h-full bg-[#5865f2] transition-all duration-75" style="width: {liveProgress[slot]}%;"></div>
                        </div>
                      {/if}
                    {/if}
                  </div>

                  <!-- Action Bar Under Preview: Play Live with Sound + Render MP4 -->
                  <div class="w-full px-3 py-2 bg-[#101014] border-t border-[rgba(226,226,224,0.1)] flex items-center justify-between gap-2">
                    <button
                      type="button"
                      on:click={() => toggleLivePreviewPlay(slot)}
                      class="px-2.5 py-1 bg-[#1e1e26] hover:bg-[#2a2a35] border border-[rgba(226,226,224,0.18)] text-slate-200 font-mono text-[11px] rounded flex items-center gap-1.5 cursor-pointer transition-colors"
                      title="Проиграть превью с загруженным звуком и анимацией прямо в окне"
                    >
                      {#if isPlayingLive[slot]}
                        <Square class="w-3 h-3 text-amber-400 fill-current" />
                        <span>Стоп</span>
                      {:else}
                        <Play class="w-3 h-3 text-emerald-400 fill-current" />
                        <span>Слушать превью ({item.duration_sec.toFixed(1)}с)</span>
                      {/if}
                    </button>

                    <button 
                      type="button"
                      on:click={() => handleRenderAndPlaySingle(slot)}
                      disabled={isRenderingSingle[slot]}
                      class="px-2.5 py-1 bg-[#5865f2] hover:bg-[#4752c4] text-white font-mono text-[11px] font-semibold rounded shadow flex items-center gap-1.5 transition-colors disabled:opacity-50 cursor-pointer"
                      title="Сгенерировать готовый видеофайл MP4 через FFmpeg с вашим звуком и эффектами"
                    >
                      {#if isRenderingSingle[slot]}
                        <RefreshCw class="w-3 h-3 animate-spin" />
                        <span>Сборка MP4...</span>
                      {:else}
                        <Sparkles class="w-3 h-3" />
                        <span>{previewVideoUrl[slot] ? 'Пересобрать MP4' : 'Собрать MP4'}</span>
                      {/if}
                    </button>
                  </div>
                </div>

                <!-- Mode Selector: Studio Generator vs Ready Video Bumper -->
                <div class="flex gap-1.5 p-1 bg-[#0c0c0e] rounded border border-[rgba(226,226,224,0.1)]">
                  <button 
                    type="button"
                    on:click={() => { item.mode = 'generate'; previewVideoUrl[slot] = null; saveConfigToStore(); }}
                    class="flex-1 py-1.5 text-xs font-mono rounded transition-colors cursor-pointer {
                      item.mode === 'generate' ? 'bg-[#5865f2] text-white font-medium' : 'text-slate-400 hover:text-white'
                    }"
                  >
                    Конструктор заставки
                  </button>
                  <button 
                    type="button"
                    on:click={() => { item.mode = 'custom_video'; previewVideoUrl[slot] = null; saveConfigToStore(); }}
                    class="flex-1 py-1.5 text-xs font-mono rounded transition-colors cursor-pointer {
                      item.mode === 'custom_video' ? 'bg-[#5865f2] text-white font-medium' : 'text-slate-400 hover:text-white'
                    }"
                  >
                    Готовая видео-заставка
                  </button>
                </div>

                {#if item.mode === 'custom_video'}
                  <!-- Ready Video Bumper + Optional Custom Sound -->
                  <div class="space-y-3 text-xs bg-[#121216] p-3.5 rounded border border-[rgba(226,226,224,0.1)]">
                    <div>
                      <span class="text-slate-300 font-mono block mb-1">Готовый видеофайл заставки (.mp4, .mov, .webm)</span>
                      <div class="flex gap-2">
                        <input 
                          type="text" 
                          placeholder="Путь или файл заставки..." 
                          value={item.custom_video_path ? getFileName(item.custom_video_path, 'Готовая заставка.mp4') : ''}
                          readonly
                          class="flex-1 bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-xs font-mono text-white px-3 py-1.5 rounded outline-none"
                        />
                        <button 
                          type="button"
                          on:click={() => triggerFileInput(`file_${slot}_vid_${activeTab}`)}
                          class="px-3 py-1.5 bg-[#5865f2] hover:bg-[#4752c4] text-xs font-mono text-white rounded cursor-pointer"
                        >
                          Загрузить заставку
                        </button>
                        <input 
                          id={`file_${slot}_vid_${activeTab}`} 
                          type="file" 
                          accept="video/*" 
                          on:change={(e) => handleMediaUpload(e, slot, 'custom_video_path')} 
                          class="hidden" 
                        />
                      </div>
                    </div>

                    <div>
                      <span class="text-slate-300 font-mono block mb-1">Заменить звук на свой аудиофайл (опционально)</span>
                      <div class="flex gap-2">
                        <input 
                          type="text" 
                          placeholder="Используется звук из видеофайла..." 
                          value={item.custom_audio_path ? getFileName(item.custom_audio_path, 'Кастомный звук.mp3') : ''}
                          readonly
                          class="flex-1 bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-xs font-mono text-white px-3 py-1.5 rounded outline-none"
                        />
                        <button 
                          type="button"
                          on:click={() => triggerFileInput(`file_${slot}_aud_cv_${activeTab}`)}
                          class="px-3 py-1.5 bg-[#222228] hover:bg-[#2c2c34] text-xs font-mono text-white rounded border border-[rgba(226,226,224,0.15)] cursor-pointer"
                        >
                          Свой звук
                        </button>
                        {#if item.custom_audio_path}
                          <button
                            type="button"
                            on:click={() => { item.custom_audio_path = null; item.custom_audio_duration = null; saveConfigToStore(); }}
                            class="px-2 py-1.5 text-red-400 hover:bg-red-500/10 rounded cursor-pointer"
                            title="Сбросить звук"
                          >
                            ✕
                          </button>
                        {/if}
                        <input 
                          id={`file_${slot}_aud_cv_${activeTab}`} 
                          type="file" 
                          accept="audio/*" 
                          on:change={(e) => handleCustomAudioSelected(e, slot)} 
                          class="hidden" 
                        />
                      </div>
                    </div>
                  </div>
                {:else}
                  <!-- Sub-Tabs inside Studio Generator: 1. Медиа и Звук | 2. Текст и Плашка | 3. Эффекты заднего видео -->
                  <div class="grid grid-cols-3 gap-1 p-1 bg-[#0c0c0e] rounded border border-[rgba(226,226,224,0.08)] text-[11px] font-mono">
                    <button
                      type="button"
                      on:click={() => (activeSubTab[slot] = 'media')}
                      class="py-1.5 px-2 rounded flex items-center justify-center gap-1 cursor-pointer transition-colors {activeSubTab[slot] === 'media' ? 'bg-[#22222c] text-white border border-[#5865f2]/50' : 'text-slate-400 hover:text-white'}"
                    >
                      <Music class="w-3 h-3 text-[#5865f2]" />
                      <span>Звук и Медиа</span>
                    </button>
                    <button
                      type="button"
                      on:click={() => (activeSubTab[slot] = 'text')}
                      class="py-1.5 px-2 rounded flex items-center justify-center gap-1 cursor-pointer transition-colors {activeSubTab[slot] === 'text' ? 'bg-[#22222c] text-white border border-[#5865f2]/50' : 'text-slate-400 hover:text-white'}"
                    >
                      <Type class="w-3 h-3 text-[#5865f2]" />
                      <span>Текст и Стили</span>
                    </button>
                    <button
                      type="button"
                      on:click={() => (activeSubTab[slot] = 'effects')}
                      class="py-1.5 px-2 rounded flex items-center justify-center gap-1 cursor-pointer transition-colors {activeSubTab[slot] === 'effects' ? 'bg-[#22222c] text-white border border-[#5865f2]/50' : 'text-slate-400 hover:text-white'}"
                    >
                      <Sliders class="w-3 h-3 text-[#5865f2]" />
                      <span>Фон и Эффекты</span>
                    </button>
                  </div>

                  {#if activeSubTab[slot] === 'media'}
                    <!-- TAB 1: ЗВУК (С АВТО-ДЛИТЕЛЬНОСТЬЮ), ЛОГОТИП И ФОНОВЫЕ ФАЙЛЫ -->
                    <div class="space-y-3.5 text-xs bg-[#121216] p-3.5 rounded border border-[rgba(226,226,224,0.08)]">
                      
                      <!-- 1. Кастомный звук и синхронизация длительности -->
                      <div class="p-2.5 bg-[#0c0c0e] rounded border border-[rgba(226,226,224,0.1)] space-y-2">
                        <div class="flex items-center justify-between">
                          <span class="font-mono text-slate-200 font-medium flex items-center gap-1.5">
                            <Music class="w-3.5 h-3.5 text-[#5865f2]" />
                            <span>Кастомный звук (.mp3, .wav, .ogg, .m4a)</span>
                          </span>
                          {#if item.custom_audio_duration}
                            <span class="text-[10px] font-mono text-emerald-400 tabular-nums">
                              Длина аудио: {item.custom_audio_duration.toFixed(2)}с
                            </span>
                          {/if}
                        </div>

                        <div class="flex gap-1.5">
                          <input 
                            type="text" 
                            placeholder="Звук не выбран (тишина)..." 
                            value={item.custom_audio_path ? getFileName(item.custom_audio_path, 'Кастомное_аудио.mp3') : ''}
                            readonly
                            class="flex-1 bg-[#16161c] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono text-[11px]"
                          />
                          <button 
                            type="button"
                            on:click={() => triggerFileInput(`file_${slot}_aud_${activeTab}`)}
                            class="px-3 py-1.5 bg-[#5865f2] hover:bg-[#4752c4] text-white rounded font-mono text-[11px] cursor-pointer"
                          >
                            Загрузить звук
                          </button>
                          {#if item.custom_audio_path}
                            <button
                              type="button"
                              on:click={() => { item.custom_audio_path = null; item.custom_audio_duration = null; saveConfigToStore(); }}
                              class="px-2 py-1.5 bg-[#16161c] hover:bg-red-500/20 text-red-400 rounded font-mono text-[11px] cursor-pointer"
                              title="Удалить звук"
                            >
                              ✕
                            </button>
                          {/if}
                          <input 
                            id={`file_${slot}_aud_${activeTab}`} 
                            type="file" 
                            accept="audio/*" 
                            on:change={(e) => handleCustomAudioSelected(e, slot)} 
                            class="hidden" 
                          />
                        </div>

                        <div class="flex flex-wrap items-center justify-between gap-2 pt-1">
                          <label class="flex items-center gap-2 cursor-pointer select-none">
                            <input
                              type="checkbox"
                              bind:checked={item.sync_duration_to_audio}
                              on:change={() => handleSyncDurationToAudioToggle(slot)}
                              class="w-3.5 h-3.5 accent-[#5865f2] rounded"
                            />
                            <span class="text-[11px] font-mono text-slate-300">Длительность по длине звука</span>
                          </label>

                          <div class="flex items-center gap-2 font-mono text-[11px] tabular-nums">
                            <span class="text-slate-400">Длина (с):</span>
                            <input
                              type="number"
                              min="0.5"
                              max="30"
                              step="0.1"
                              bind:value={item.duration_sec}
                              on:input={() => { item.sync_duration_to_audio = false; saveConfigToStore(); }}
                              class="w-16 bg-[#16161c] border border-[rgba(226,226,224,0.18)] text-white px-2 py-0.5 rounded text-right tabular-nums"
                            />
                          </div>
                        </div>

                        {#if item.custom_audio_path}
                          <div class="grid grid-cols-3 gap-2 pt-1 border-t border-[rgba(226,226,224,0.08)] items-center">
                            <div class="col-span-1">
                              <span class="text-[10px] font-mono text-slate-400 tabular-nums">Громкость: {Math.round((item.audio_volume ?? 1) * 100)}%</span>
                              <input
                                type="range"
                                min="0.1"
                                max="1.5"
                                step="0.05"
                                bind:value={item.audio_volume}
                                on:input={saveConfigToStore}
                                class="w-full accent-[#5865f2]"
                              />
                            </div>
                            <label class="flex items-center gap-1.5 text-[10px] font-mono text-slate-300 cursor-pointer">
                              <input type="checkbox" bind:checked={item.audio_fade_in} on:change={saveConfigToStore} class="accent-[#5865f2]" />
                              <span>Fade-In звука</span>
                            </label>
                            <label class="flex items-center gap-1.5 text-[10px] font-mono text-slate-300 cursor-pointer">
                              <input type="checkbox" bind:checked={item.audio_fade_out} on:change={saveConfigToStore} class="accent-[#5865f2]" />
                              <span>Fade-Out звука</span>
                            </label>
                          </div>
                        {/if}
                      </div>

                      <!-- 2. Кастомный логотип и его размер/позиция -->
                      <div class="p-2.5 bg-[#0c0c0e] rounded border border-[rgba(226,226,224,0.1)] space-y-2">
                        <span class="font-mono text-slate-200 font-medium flex items-center gap-1.5">
                          <ImageIcon class="w-3.5 h-3.5 text-[#5865f2]" />
                          <span>Кастомный логотип / картинка (PNG, GIF, WEBP, JPG)</span>
                        </span>

                        <div class="flex gap-1.5">
                          <input 
                            type="text" 
                            placeholder="Логотип не загружен..." 
                            value={item.logo_image_path ? getFileName(item.logo_image_path, 'Логотип.png') : ''}
                            readonly
                            class="flex-1 bg-[#16161c] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono text-[11px]"
                          />
                          <button 
                            type="button"
                            on:click={() => triggerFileInput(`file_${slot}_logo_${activeTab}`)}
                            class="px-3 py-1.5 bg-[#222228] hover:bg-[#2c2c34] text-white rounded font-mono text-[11px] border border-[rgba(226,226,224,0.15)] cursor-pointer"
                          >
                            Выбрать логотип
                          </button>
                          {#if item.logo_image_path}
                            <button
                              type="button"
                              on:click={() => { item.logo_image_path = null; saveConfigToStore(); }}
                              class="px-2 py-1.5 bg-[#16161c] hover:bg-red-500/20 text-red-400 rounded font-mono text-[11px] cursor-pointer"
                              title="Удалить логотип"
                            >
                              ✕
                            </button>
                          {/if}
                          <input 
                            id={`file_${slot}_logo_${activeTab}`} 
                            type="file" 
                            accept="image/*" 
                            on:change={(e) => handleMediaUpload(e, slot, 'logo_image_path')} 
                            class="hidden" 
                          />
                        </div>

                        {#if item.logo_image_path}
                          <div class="grid grid-cols-2 sm:grid-cols-3 gap-2.5 pt-1">
                            <div>
                              <span class="text-[10px] text-slate-400 font-mono block tabular-nums">Размер лого: {Math.round(item.logo_scale * 100)}%</span>
                              <input type="range" min="0.1" max="1.4" step="0.02" bind:value={item.logo_scale} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                            </div>
                            <div>
                              <span class="text-[10px] text-slate-400 font-mono block tabular-nums">Позиция X: {Math.round(item.logo_x_percent)}%</span>
                              <input type="range" min="10" max="90" step="1" bind:value={item.logo_x_percent} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                            </div>
                            <div>
                              <span class="text-[10px] text-slate-400 font-mono block tabular-nums">Позиция Y: {Math.round(item.logo_y_percent)}%</span>
                              <input type="range" min="10" max="90" step="1" bind:value={item.logo_y_percent} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                            </div>
                            <div>
                              <span class="text-[10px] text-slate-400 font-mono block tabular-nums">Прозрачность: {Math.round((item.logo_opacity ?? 1) * 100)}%</span>
                              <input type="range" min="0.1" max="1" step="0.05" bind:value={item.logo_opacity} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                            </div>
                            <div>
                              <span class="text-[10px] text-slate-400 font-mono block tabular-nums">Скругление: {Math.round(item.logo_radius ?? 0)}%</span>
                              <input type="range" min="0" max="50" step="2" bind:value={item.logo_radius} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                            </div>
                            <label class="flex items-center gap-1.5 text-[10px] font-mono text-slate-300 cursor-pointer self-end pb-1">
                              <input type="checkbox" bind:checked={item.logo_shadow} on:change={saveConfigToStore} class="accent-[#5865f2]" />
                              <span>Тень логотипа</span>
                            </label>
                          </div>
                        {/if}
                      </div>
                    </div>

                  {:else if activeSubTab[slot] === 'text'}
                    <!-- TAB 2: НАСТРОЙКА ТЕКСТА, РАЗМЕРОВ, ЦВЕТОВ И ПЛАШКИ -->
                    <div class="space-y-3 text-xs bg-[#121216] p-3.5 rounded border border-[rgba(226,226,224,0.08)]">
                      <div class="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
                        <div>
                          <span class="text-slate-400 font-mono block mb-1">Заголовок</span>
                          <input 
                            type="text" 
                            placeholder="Название канала / студии..."
                            bind:value={item.title}
                            on:input={saveConfigToStore}
                            class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono"
                          />
                        </div>
                        <div>
                          <span class="text-slate-400 font-mono block mb-1">Подзаголовок</span>
                          <input 
                            type="text" 
                            placeholder="Призыв к подписке / соцсети..."
                            bind:value={item.subtitle}
                            on:input={saveConfigToStore}
                            class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono"
                          />
                        </div>
                      </div>

                      <div class="grid grid-cols-2 sm:grid-cols-4 gap-2.5">
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block mb-1">Цвет заголовка</span>
                          <input type="color" bind:value={item.title_color} on:input={saveConfigToStore} class="w-full h-7 bg-transparent cursor-pointer rounded" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block mb-1 tabular-nums">Размер загол. ({item.title_size}px)</span>
                          <input type="range" min="16" max="76" step="2" bind:value={item.title_size} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block mb-1">Цвет подзагол.</span>
                          <input type="color" bind:value={item.subtitle_color} on:input={saveConfigToStore} class="w-full h-7 bg-transparent cursor-pointer rounded" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block mb-1 tabular-nums">Размер подз. ({item.subtitle_size ?? 18}px)</span>
                          <input type="range" min="12" max="44" step="1" bind:value={item.subtitle_size} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                      </div>

                      <div class="grid grid-cols-2 sm:grid-cols-4 gap-2.5">
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block mb-1 tabular-nums">Позиция X ({Math.round(item.title_x_percent)}%)</span>
                          <input type="range" min="15" max="85" step="1" bind:value={item.title_x_percent} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block mb-1 tabular-nums">Позиция Y ({Math.round(item.title_y_percent)}%)</span>
                          <input type="range" min="12" max="90" step="1" bind:value={item.title_y_percent} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block mb-1">Стиль плашки</span>
                          <select bind:value={item.plate_style} on:change={saveConfigToStore} class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.18)] text-white px-2 py-1 rounded font-mono text-[11px]">
                            <option value="glass">Матовая (Glass)</option>
                            <option value="solid">Сплошная</option>
                            <option value="none">Без плашки</option>
                          </select>
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block mb-1 tabular-nums">Прозр. плашки ({Math.round((item.plate_opacity ?? 0.55) * 100)}%)</span>
                          <input type="range" min="0.1" max="0.95" step="0.05" bind:value={item.plate_opacity} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                      </div>

                      <div class="grid grid-cols-2 gap-2.5 pt-1 border-t border-[rgba(226,226,224,0.08)]">
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block mb-1">Анимация текста и лого</span>
                          <select bind:value={item.animation} on:change={saveConfigToStore} class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.18)] text-white px-2 py-1 rounded font-mono text-[11px]">
                            <option value="fade_zoom">Плавный зум (Fade & Zoom)</option>
                            <option value="pulse">Пульсация акцента</option>
                            <option value="slide_up">Выезд снизу</option>
                            <option value="none">Статично</option>
                          </select>
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block mb-1">Переход склейки с клипом</span>
                          <select bind:value={item.transition} on:change={saveConfigToStore} class="w-full bg-[#0c0c0e] border border-[rgba(226,226,224,0.18)] text-white px-2 py-1 rounded font-mono text-[11px]">
                            <option value="crossfade">Мягкий наплыв (Crossfade)</option>
                            <option value="fade_black">Через затемнение (Fade Black)</option>
                            <option value="cut">Прямая склейка (Cut)</option>
                          </select>
                        </div>
                      </div>
                    </div>

                  {:else}
                    <!-- TAB 3: ФОНОВОЕ ВИДЕО, КАСТОМНОЕ ФОНОВОЕ ИЗОБРАЖЕНИЕ И ЭФФЕКТЫ -->
                    <div class="space-y-3 text-xs bg-[#121216] p-3.5 rounded border border-[rgba(226,226,224,0.08)]">
                      
                      <!-- Выбор источника заднего плана -->
                      <div>
                        <span class="text-slate-400 font-mono block mb-1.5">Источник заднего плана</span>
                        <div class="grid grid-cols-2 sm:grid-cols-4 gap-1.5 font-mono text-[11px]">
                          <button
                            type="button"
                            on:click={() => { item.bg_mode = 'blur_frame'; saveConfigToStore(); }}
                            class="py-1.5 px-2 rounded border cursor-pointer {item.bg_mode === 'blur_frame' ? 'bg-[#5865f2] border-[#5865f2] text-white' : 'bg-[#0c0c0e] border-[rgba(226,226,224,0.12)] text-slate-300'}"
                          >
                            Кадр из видео
                          </button>
                          <button
                            type="button"
                            on:click={() => { item.bg_mode = 'custom_video'; saveConfigToStore(); }}
                            class="py-1.5 px-2 rounded border cursor-pointer {item.bg_mode === 'custom_video' ? 'bg-[#5865f2] border-[#5865f2] text-white' : 'bg-[#0c0c0e] border-[rgba(226,226,224,0.12)] text-slate-300'}"
                          >
                            Заднее видео
                          </button>
                          <button
                            type="button"
                            on:click={() => { item.bg_mode = 'custom_image'; saveConfigToStore(); }}
                            class="py-1.5 px-2 rounded border cursor-pointer {item.bg_mode === 'custom_image' ? 'bg-[#5865f2] border-[#5865f2] text-white' : 'bg-[#0c0c0e] border-[rgba(226,226,224,0.12)] text-slate-300'}"
                          >
                            Своя картинка
                          </button>
                          <button
                            type="button"
                            on:click={() => { item.bg_mode = 'gradient'; saveConfigToStore(); }}
                            class="py-1.5 px-2 rounded border cursor-pointer {item.bg_mode === 'gradient' ? 'bg-[#5865f2] border-[#5865f2] text-white' : 'bg-[#0c0c0e] border-[rgba(226,226,224,0.12)] text-slate-300'}"
                          >
                            Градиент
                          </button>
                        </div>
                      </div>

                      {#if item.bg_mode === 'custom_video'}
                        <div class="p-2.5 bg-[#0c0c0e] rounded border border-[rgba(226,226,224,0.1)] space-y-1.5">
                          <span class="text-[11px] font-mono text-slate-300 block">Загрузить заднее фоновое видео (.mp4, .mov, .webm)</span>
                          <div class="flex gap-1.5">
                            <input
                              type="text"
                              placeholder="Фоновое видео не выбрано..."
                              value={item.bg_video_path ? getFileName(item.bg_video_path, 'Фоновое_видео.mp4') : ''}
                              readonly
                              class="flex-1 bg-[#16161c] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono text-[11px]"
                            />
                            <button
                              type="button"
                              on:click={() => triggerFileInput(`file_${slot}_bgvid_${activeTab}`)}
                              class="px-3 py-1.5 bg-[#5865f2] hover:bg-[#4752c4] text-white rounded font-mono text-[11px] cursor-pointer"
                            >
                              Выбрать видео
                            </button>
                            <input
                              id={`file_${slot}_bgvid_${activeTab}`}
                              type="file"
                              accept="video/*"
                              on:change={(e) => handleMediaUpload(e, slot, 'bg_video_path')}
                              class="hidden"
                            />
                          </div>
                        </div>
                      {:else if item.bg_mode === 'custom_image'}
                        <div class="p-2.5 bg-[#0c0c0e] rounded border border-[rgba(226,226,224,0.1)] space-y-1.5">
                          <span class="text-[11px] font-mono text-slate-300 block">Загрузить кастомное фоновое изображение (JPG, PNG, WEBP)</span>
                          <div class="flex gap-1.5">
                            <input
                              type="text"
                              placeholder="Фоновое изображение не выбрано..."
                              value={item.bg_image_path ? getFileName(item.bg_image_path, 'Фон.png') : ''}
                              readonly
                              class="flex-1 bg-[#16161c] border border-[rgba(226,226,224,0.15)] text-white px-2.5 py-1.5 rounded font-mono text-[11px]"
                            />
                            <button
                              type="button"
                              on:click={() => triggerFileInput(`file_${slot}_bgimg_${activeTab}`)}
                              class="px-3 py-1.5 bg-[#5865f2] hover:bg-[#4752c4] text-white rounded font-mono text-[11px] cursor-pointer"
                            >
                              Выбрать фон
                            </button>
                            <input
                              id={`file_${slot}_bgimg_${activeTab}`}
                              type="file"
                              accept="image/*"
                              on:change={(e) => handleMediaUpload(e, slot, 'bg_image_path')}
                              class="hidden"
                            />
                          </div>
                        </div>
                      {:else if item.bg_mode === 'gradient'}
                        <div class="grid grid-cols-2 gap-2.5 p-2.5 bg-[#0c0c0e] rounded border border-[rgba(226,226,224,0.1)]">
                          <div>
                            <span class="text-[10px] font-mono text-slate-400 block mb-1">Основной цвет фона</span>
                            <input type="color" bind:value={item.bg_color_primary} on:input={saveConfigToStore} class="w-full h-7 bg-transparent cursor-pointer rounded" />
                          </div>
                          <div>
                            <span class="text-[10px] font-mono text-slate-400 block mb-1">Второй цвет градиента</span>
                            <input type="color" bind:value={item.bg_color_secondary} on:input={saveConfigToStore} class="w-full h-7 bg-transparent cursor-pointer rounded" />
                          </div>
                        </div>
                      {/if}

                      <!-- Слайдеры эффектов заднего видео и фона -->
                      <div class="grid grid-cols-2 sm:grid-cols-3 gap-2.5 p-2.5 bg-[#0c0c0e] rounded border border-[rgba(226,226,224,0.1)]">
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block tabular-nums">Размытие (Blur): {Math.round(item.bg_blur ?? 12)}px</span>
                          <input type="range" min="0" max="40" step="1" bind:value={item.bg_blur} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block tabular-nums">Затемнение: {Math.round((item.bg_dim ?? 0.35) * 100)}%</span>
                          <input type="range" min="0" max="0.9" step="0.05" bind:value={item.bg_dim} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block tabular-nums">Масштаб фона: {Math.round((item.bg_scale ?? 1.1) * 100)}%</span>
                          <input type="range" min="1.0" max="1.8" step="0.05" bind:value={item.bg_scale} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block tabular-nums">Насыщенность: {Math.round((item.bg_saturation ?? 1.0) * 100)}%</span>
                          <input type="range" min="0.0" max="2.0" step="0.1" bind:value={item.bg_saturation} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block tabular-nums">Контраст: {Math.round((item.bg_contrast ?? 1.0) * 100)}%</span>
                          <input type="range" min="0.6" max="1.5" step="0.05" bind:value={item.bg_contrast} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                        <div>
                          <span class="text-[10px] text-slate-400 font-mono block tabular-nums">Скорость видео: {(item.bg_video_speed ?? 1.0).toFixed(1)}x</span>
                          <input type="range" min="0.5" max="2.0" step="0.1" bind:value={item.bg_video_speed} on:input={saveConfigToStore} class="w-full accent-[#5865f2]" />
                        </div>
                      </div>

                      <div class="flex items-center justify-between px-1">
                        <label class="flex items-center gap-2 text-[11px] font-mono text-slate-300 cursor-pointer">
                          <input type="checkbox" bind:checked={item.bg_vignette} on:change={saveConfigToStore} class="accent-[#5865f2]" />
                          <span>Кинематографичная виньетка по краям кадра</span>
                        </label>
                      </div>
                    </div>
                  {/if}
                {/if}
              {/if}
            </div>
          {/each}

        </div>
      </div>

      <!-- Modal Footer -->
      <div class="px-5 py-3.5 bg-[#18181c] border-t border-[rgba(226,226,224,0.1)] flex flex-wrap items-center justify-between gap-3">
        <div class="text-[11px] font-mono text-slate-400">
          💡 Все изменения мгновенно отображаются на интерактивном превью выше и сохраняются в активный пресет.
        </div>
        <button 
          type="button"
          on:click={handleClose}
          class="px-5 py-2 bg-[#5865f2] hover:bg-[#4752c4] text-white font-mono text-xs font-semibold uppercase tracking-wider rounded transition-colors cursor-pointer"
        >
          Сохранить и закрыть
        </button>
      </div>

    </div>
  </div>
{/if}
