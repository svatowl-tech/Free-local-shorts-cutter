import { writable } from 'svelte/store';

export interface Fragment {
  id: string;
  start: number; // in seconds
  end: number;   // in seconds
}

export const fragmentsStore = writable<Fragment[]>([]);

export const addFragment = (start: number, duration: number = 5) => {
  fragmentsStore.update(f => [
    ...f,
    { id: Math.random().toString(36).substring(2, 9), start, end: start + duration }
  ]);
};

export const removeFragment = (id: string) => {
  fragmentsStore.update(f => f.filter(frag => frag.id !== id));
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

