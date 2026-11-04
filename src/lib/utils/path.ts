// src/lib/utils/path.ts

interface TauriWindow extends Window {
  __TAURI_INTERNALS__?: Record<string, unknown>;
}

/**
 * Безопасная проверка запуска внутри среды Tauri v2 без использования `any`.
 */
export function isTauriRuntime(): boolean {
  if (typeof window === 'undefined') {
    return false;
  }
  const win = window as TauriWindow;
  return win.__TAURI_INTERNALS__ !== undefined;
}

/**
 * Приводит разделители путей к единому виду `/`.
 */
export function normalizeSlashes(filePath: string): string {
  return (filePath || '').replace(/\\/g, '/');
}

/**
 * Извлекает имя файла из полного пути (корректно работает с путями Windows `\`, Unix `/` и URI).
 */
export function getFileName(filePath: string, fallback = ''): string {
  if (!filePath) return fallback;
  const normalized = normalizeSlashes(filePath).replace(/\/+$/, '');
  const idx = normalized.lastIndexOf('/');
  const name = idx >= 0 ? normalized.slice(idx + 1) : normalized;
  return name || fallback;
}

/**
 * Извлекает родительскую директорию с сохранением исходного разделителя платформы.
 */
export function getParentDir(filePath: string, fallback = ''): string {
  if (!filePath) return fallback;
  const trimmed = filePath.replace(/[/\\]+$/, '');
  const lastForward = trimmed.lastIndexOf('/');
  const lastBackward = trimmed.lastIndexOf('\\');
  const lastSep = Math.max(lastForward, lastBackward);
  if (lastSep > 0) {
    return trimmed.slice(0, lastSep);
  }
  return fallback;
}

/**
 * Проверяет, содержит ли строка разделитель директорий (полный/относительный путь, а не только имя файла).
 */
export function hasDirectorySeparator(filePath: string): boolean {
  if (!filePath) return false;
  return filePath.includes('/') || filePath.includes('\\');
}

/**
 * Собирает относительный путь из сегментов без дублирования слэшей.
 */
export function joinRelativePath(...segments: string[]): string {
  return segments
    .map((s) => normalizeSlashes(s).replace(/^\/+|\/+$/g, ''))
    .filter((s) => s.length > 0)
    .join('/');
}

/**
 * Универсальное безопасное извлечение текста ошибки из `unknown`.
 */
export function formatErrorMessage(err: unknown): string {
  if (typeof err === 'string') {
    return err;
  }
  if (err instanceof Error) {
    return err.message;
  }
  if (err && typeof err === 'object' && 'message' in err) {
    const msg = (err as { message?: unknown }).message;
    if (typeof msg === 'string') {
      return msg;
    }
  }
  return String(err);
}
