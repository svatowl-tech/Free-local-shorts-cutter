import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

const tauriConfPath = path.join(rootDir, 'src-tauri', 'tauri.conf.json');
const packageJsonPath = path.join(rootDir, 'package.json');
const cargoTomlPath = path.join(rootDir, 'src-tauri', 'Cargo.toml');

// Read current versions
const tauriConf = JSON.parse(fs.readFileSync(tauriConfPath, 'utf8'));
const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'));
const currentVersion = tauriConf.version || packageJson.version || '2.0.0';

const arg = process.argv[2] || 'patch';

function calculateNewVersion(current, mode) {
  // If specific version is passed (e.g. 2.1.0 or v2.1.0)
  const cleanVersion = mode.replace(/^v/, '');
  if (/^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$/.test(cleanVersion)) {
    return cleanVersion;
  }

  const parts = current.split('-')[0].split('.').map(Number);
  let [major = 0, minor = 0, patch = 0] = parts;

  switch (mode.toLowerCase()) {
    case 'major':
      major += 1;
      minor = 0;
      patch = 0;
      break;
    case 'minor':
      minor += 1;
      patch = 0;
      break;
    case 'patch':
    default:
      patch += 1;
      break;
  }

  return `${major}.${minor}.${patch}`;
}

const newVersion = calculateNewVersion(currentVersion, arg);

console.log(`Bumping version: ${currentVersion} -> ${newVersion} (${arg})`);

// 1. Update tauri.conf.json
tauriConf.version = newVersion;
fs.writeFileSync(tauriConfPath, JSON.stringify(tauriConf, null, 2) + '\n', 'utf8');
console.log(`Updated ${tauriConfPath}`);

// 2. Update package.json
packageJson.version = newVersion;
fs.writeFileSync(packageJsonPath, JSON.stringify(packageJson, null, 2) + '\n', 'utf8');
console.log(`Updated ${packageJsonPath}`);

// 3. Update Cargo.toml
if (fs.existsSync(cargoTomlPath)) {
  let cargoContent = fs.readFileSync(cargoTomlPath, 'utf8');
  cargoContent = cargoContent.replace(
    /(\[package\][\s\S]*?version\s*=\s*")[^"]+(")/,
    `$1${newVersion}$2`
  );
  fs.writeFileSync(cargoTomlPath, cargoContent, 'utf8');
  console.log(`Updated ${cargoTomlPath}`);
}

// 4. Output for GitHub Actions
if (process.env.GITHUB_OUTPUT) {
  const output = [
    `version=${newVersion}`,
    `tag=v${newVersion}`,
    `previous_version=${currentVersion}`
  ].join('\n') + '\n';
  fs.appendFileSync(process.env.GITHUB_OUTPUT, output, 'utf8');
  console.log('Written outputs to GITHUB_OUTPUT');
}
