const fs = require('fs');
fs.mkdirSync('.svelte-kit/output/server', { recursive: true });
fs.writeFileSync('.svelte-kit/output/server/internal.js', 'export const internal = {};');
fs.writeFileSync('.svelte-kit/output/server/manifest-full.js', 'export const manifest = {};');
console.log('Prebuild server stubs created successfully.');
