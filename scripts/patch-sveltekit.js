// Этот скрипт патчит SvelteKit для:
// 1. Совместимости window.fetch в Tauri 2 ("Cannot set property fetch of #<Window> which has only a getter")
// 2. Корректного разрешения путей манифеста Vite при использовании Tailwind v4 (@tailwindcss/postcss / @tailwindcss/vite)
import fs from 'fs';
import path from 'path';

const fetcherPath = path.resolve('node_modules', '@sveltejs', 'kit', 'src', 'runtime', 'client', 'fetcher.js');

if (fs.existsSync(fetcherPath)) {
  let content = fs.readFileSync(fetcherPath, 'utf8');

  if (!content.includes("Object.defineProperty(window, 'fetch'")) {
    content = content.replace(/window\.fetch = \((.*?)\) => \{/g, `Object.defineProperty(window, 'fetch', {
 configurable: true,
 writable: true,
 value: ($1) => {`);
    content = content.replace(/return native_fetch\(input, init\);\s*\};/g, `return native_fetch(input, init);\n\t}\n});`);
    
    fs.writeFileSync(fetcherPath, content);
    console.log('Successfully patched SvelteKit fetcher.js for Tauri 2 compatibility!');
  }
}

const buildUtilsPath = path.resolve('node_modules', '@sveltejs', 'kit', 'src', 'exports', 'vite', 'build', 'utils.js');

if (fs.existsSync(buildUtilsPath)) {
  let utilsContent = fs.readFileSync(buildUtilsPath, 'utf8');

  if (!utilsContent.includes('k.endsWith')) {
    utilsContent = utilsContent.replace(
      'while (!manifest[file]) {',
      `while (!manifest[file]) {
		const altKey = Object.keys(manifest).find((k) => k === file || k.endsWith('/' + file));
		if (altKey) {
			file = altKey;
			break;
		}`
    );
    fs.writeFileSync(buildUtilsPath, utilsContent);
    console.log('Successfully patched SvelteKit build/utils.js for Tailwind v4 manifest compatibility!');
  }
}

const syncUtilsPath = path.resolve('node_modules', '@sveltejs', 'kit', 'src', 'core', 'sync', 'utils.js');

if (fs.existsSync(syncUtilsPath)) {
  let syncUtilsContent = fs.readFileSync(syncUtilsPath, 'utf8');

  if (!syncUtilsContent.includes('fs.existsSync(file)')) {
    syncUtilsContent = syncUtilsContent.replace(
      'export function write_if_changed(file, code) {',
      `export function write_if_changed(file, code) {
	if (!previous_contents.has(file) && fs.existsSync(file)) {
		try {
			previous_contents.set(file, fs.readFileSync(file, 'utf8'));
		} catch {}
	}`
    );
    fs.writeFileSync(syncUtilsPath, syncUtilsContent);
    console.log('Successfully patched SvelteKit sync/utils.js to prevent redundant file writes!');
  }
}

