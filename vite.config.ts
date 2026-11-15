import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [sveltekit()],
	optimizeDeps: {
		include: [
			'clsx',
			'tailwind-merge',
			'@lucide/svelte',
			'@tauri-apps/api/core',
			'@tauri-apps/api/event',
			'@tauri-apps/plugin-dialog',
			'@tauri-apps/plugin-shell'
		]
	},
	server: {
		port: 3000,
		strictPort: true,
		warmup: {
			clientFiles: [
				'./src/routes/+layout.svelte',
				'./src/routes/+page.svelte',
				'./src/lib/components/*.svelte'
			]
		},
		watch: {
			ignored: [
				'**/src-tauri/**',
				'**/build/**',
				'**/dist-releases/**'
			]
		}
	}
});


