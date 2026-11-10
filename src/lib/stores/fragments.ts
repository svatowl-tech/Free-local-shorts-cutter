import { writable } from 'svelte/store';

export interface Fragment {
  id: string;
  start: number; // in seconds
  end: number;   // in seconds
  title?: string;
  repaired?: boolean;
  repair_note?: string;
}

export const fragmentsStore = writable<Fragment[]>([]);

export const addFragment = (start: number, duration: number = 15) => {
  const safeStart = Math.max(0, start);
  fragmentsStore.update(f => [
    ...f,
    { 
      id: Math.random().toString(36).substring(2, 9), 
      start: safeStart, 
      end: safeStart + duration,
      title: `Клип #${f.length + 1}`
    }
  ]);
};

export const addFragmentRange = (start: number, end: number, title?: string) => {
  const safeStart = Math.max(0, Math.min(start, end));
  const safeEnd = Math.max(start, end);
  if (safeEnd - safeStart < 0.2) return;

  fragmentsStore.update(f => [
    ...f,
    { 
      id: Math.random().toString(36).substring(2, 9), 
      start: safeStart, 
      end: safeEnd,
      title: title || `Клип #${f.length + 1}`
    }
  ]);
};

export const removeFragment = (id: string) => {
  fragmentsStore.update(f => {
    const filtered = f.filter(frag => frag.id !== id);
    // Перенумеровываем заголовки по порядку, сохраняя отметку о доработке
    return filtered.map((frag, idx) => ({
      ...frag,
      title: frag.repaired ? `Клип #${idx + 1} (✓ Доработан ИИ)` : `Клип #${idx + 1}`
    }));
  });
};

export const updateFragment = (id: string, updates: Partial<Fragment>) => {
  fragmentsStore.update(f => f.map(frag => frag.id === id ? { ...frag, ...updates } : frag));
};

export const clearFragments = () => {
  fragmentsStore.set([]);
};

export const loadFragments = (fragments: Fragment[]) => {
  fragmentsStore.set(fragments);
};

