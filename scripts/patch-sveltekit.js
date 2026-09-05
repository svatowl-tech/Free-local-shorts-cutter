// Этот скрипт патчит SvelteKit, чтобы исправить ошибку:
// "Cannot set property fetch of #<Window> which has only a getter" в Tauri 2
import fs from 'fs';
import path from 'path';

const fetcherPath = path.resolve('node_modules', '@sveltejs', 'kit', 'src', 'runtime', 'client', 'fetcher.js');

if (fs.existsSync(fetcherPath)) {
  let content = fs.readFileSync(fetcherPath, 'utf8');

  // Если уже пропатчен, пропускаем
  if (!content.includes("Object.defineProperty(window, 'fetch'")) {
    content = content.replace(/window\.fetch = \((.*?)\) => \{/g, `Object.defineProperty(window, 'fetch', {
 configurable: true,
 writable: true,
 value: ($1) => {`);
    content = content.replace(/return native_fetch\(input, init\);\s*\};/g, `return native_fetch(input, init);\n\t}\n});`);
    
    fs.writeFileSync(fetcherPath, content);
    console.log('Successfully patched SvelteKit fetcher.js for Tauri 2 compatibility!');
  } else {
    console.log('SvelteKit fetcher.js is already patched.');
  }
} else {
  console.log('SvelteKit fetcher.js not found, skipping patch.');
}
