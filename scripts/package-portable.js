import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import { execSync } from 'child_process';
import AdmZip from 'adm-zip';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

const tauriConfPath = path.join(rootDir, 'src-tauri', 'tauri.conf.json');
const tauriConf = JSON.parse(fs.readFileSync(tauriConfPath, 'utf8'));

const appName = (tauriConf.productName || 'Video Cutter Pro').replace(/\s+/g, '-');
const version = tauriConf.version || '2.0.0';
const target = process.env.TARGET || '';
const platform = process.platform;

const distReleasesDir = path.join(rootDir, 'dist-releases');
fs.mkdirSync(distReleasesDir, { recursive: true });

console.log(`\n=== УПАКОВКА РЕЛИЗОВ И PORTABLE ВЕРСИЙ [${platform}] ===`);
console.log(`Приложение: ${appName} v${version}`);
console.log(`Целевой таргет (TARGET): ${target || '(по умолчанию)'}`);
console.log(`Папка назначения: ${distReleasesDir}\n`);

// Поиск всех возможных директорий release
const possibleReleaseDirs = [];
if (target) {
  possibleReleaseDirs.push(path.join(rootDir, 'src-tauri', 'target', target, 'release'));
}
possibleReleaseDirs.push(path.join(rootDir, 'src-tauri', 'target', 'release'));

const tauriTargetDir = path.join(rootDir, 'src-tauri', 'target');
if (fs.existsSync(tauriTargetDir)) {
  const targetSubdirs = fs.readdirSync(tauriTargetDir, { withFileTypes: true });
  for (const dir of targetSubdirs) {
    if (dir.isDirectory()) {
      const candidate = path.join(tauriTargetDir, dir.name, 'release');
      if (fs.existsSync(candidate) && !possibleReleaseDirs.includes(candidate)) {
        possibleReleaseDirs.push(candidate);
      }
    }
  }
}

console.log('Проверяемые директории сборки:');
possibleReleaseDirs.forEach(d => console.log(`  - ${d} (существует: ${fs.existsSync(d)})`));
console.log('');

function copyFileSafe(src, dest) {
  if (fs.existsSync(src)) {
    fs.copyFileSync(src, dest);
    const sizeMb = (fs.statSync(dest).size / (1024 * 1024)).toFixed(2);
    console.log(`[УСПЕХ] Скопировано: ${path.basename(dest)} (${sizeMb} MB)`);
    return true;
  }
  return false;
}

function copyDirRecursive(src, dest) {
  if (!fs.existsSync(src)) return;
  fs.mkdirSync(dest, { recursive: true });
  const entries = fs.readdirSync(src, { withFileTypes: true });
  for (const entry of entries) {
    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);
    if (entry.isDirectory()) {
      copyDirRecursive(srcPath, destPath);
    } else {
      fs.copyFileSync(srcPath, destPath);
    }
  }
}

function findFilesWithExt(dir, ext) {
  const results = [];
  if (!fs.existsSync(dir)) return results;
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      results.push(...findFilesWithExt(full, ext));
    } else if (entry.name.toLowerCase().endsWith(ext.toLowerCase())) {
      results.push(full);
    }
  }
  return results;
}

function findBundles(subfolder, ext) {
  const found = [];
  for (const relDir of possibleReleaseDirs) {
    const specificDir = path.join(relDir, 'bundle', subfolder);
    found.push(...findFilesWithExt(specificDir, ext));
    const bundleDir = path.join(relDir, 'bundle');
    found.push(...findFilesWithExt(bundleDir, ext));
  }
  if (found.length === 0) {
    found.push(...findFilesWithExt(path.join(rootDir, 'src-tauri', 'target'), ext));
  }
  return Array.from(new Set(found));
}

function findBinary(binName) {
  for (const relDir of possibleReleaseDirs) {
    const candidate = path.join(relDir, binName);
    if (fs.existsSync(candidate)) return candidate;
  }
  const allCandidates = findFilesWithExt(path.join(rootDir, 'src-tauri', 'target'), path.extname(binName) || binName);
  for (const candidate of allCandidates) {
    if (path.basename(candidate).toLowerCase() === binName.toLowerCase() &&
        !candidate.includes('bundle') &&
        !candidate.includes('deps') &&
        !candidate.includes('incremental') &&
        !candidate.includes('examples')) {
      return candidate;
    }
  }
  return null;
}

// 1. Windows Packaging
if (platform === 'win32') {
  console.log('--- Поиск Windows инсталляторов и бинарников ---');
  
  // A. NSIS установщики
  const nsisFiles = findBundles('nsis', '.exe').filter(f => !f.toLowerCase().includes('uninstall'));
  for (const nsis of nsisFiles) {
    const dest = path.join(distReleasesDir, `${appName}_${version}_windows_x64_setup.exe`);
    copyFileSafe(nsis, dest);
  }

  // B. MSI установщики
  const msiFiles = findBundles('msi', '.msi');
  for (const msi of msiFiles) {
    const dest = path.join(distReleasesDir, `${appName}_${version}_windows_x64.msi`);
    copyFileSafe(msi, dest);
  }

  // C. Portable ZIP
  const exeSrc = findBinary('video-cutter.exe') || findBinary(`${appName}.exe`);
  if (exeSrc) {
    console.log(`Найден исполняемый файл для Portable: ${exeSrc}`);
    const portableDirName = `${appName}_${version}_windows_x64_portable`;
    const portableDir = path.join(distReleasesDir, portableDirName);
    fs.rmSync(portableDir, { recursive: true, force: true });
    fs.mkdirSync(portableDir, { recursive: true });

    fs.copyFileSync(exeSrc, path.join(portableDir, `${appName}.exe`));

    const resourcesSrc = path.join(rootDir, 'src-tauri', 'resources');
    const modelsSrc = path.join(rootDir, 'src-tauri', 'models');
    copyDirRecursive(resourcesSrc, path.join(portableDir, 'resources'));
    copyDirRecursive(modelsSrc, path.join(portableDir, 'models'));

    const readme = `=== ${appName} v${version} Portable ===\n\n` +
      `Данная версия не требует установки в систему.\n` +
      `Для запуска откройте "${appName}.exe".\n` +
      `Все локальные модели нейросетей размещаются в папке "models".\n`;
    fs.writeFileSync(path.join(portableDir, 'PORTABLE_README.txt'), readme, 'utf8');

    const zipDest = path.join(distReleasesDir, `${appName}_${version}_windows_x64_portable.zip`);
    console.log(`Создание архива Portable ZIP: ${path.basename(zipDest)}...`);
    const zip = new AdmZip();
    zip.addLocalFolder(portableDir, portableDirName);
    zip.writeZip(zipDest);
    const sizeMb = (fs.statSync(zipDest).size / (1024 * 1024)).toFixed(2);
    console.log(`[УСПЕХ] Создан Portable ZIP: ${path.basename(zipDest)} (${sizeMb} MB)`);

    fs.rmSync(portableDir, { recursive: true, force: true });
  } else {
    console.warn('[ПРЕДУПРЕЖДЕНИЕ] video-cutter.exe не найден для создания Portable ZIP');
  }
}

// 2. Linux Packaging
if (platform === 'linux') {
  console.log('--- Поиск Linux пакетов и бинарников ---');

  // A. DEB пакет
  const debFiles = findBundles('deb', '.deb');
  for (const deb of debFiles) {
    const dest = path.join(distReleasesDir, `${appName}_${version}_linux_amd64.deb`);
    copyFileSafe(deb, dest);
  }

  // B. AppImage
  const appImages = findBundles('appimage', '.AppImage');
  for (const appImg of appImages) {
    const dest = path.join(distReleasesDir, `${appName}_${version}_linux_amd64.AppImage`);
    copyFileSafe(appImg, dest);
  }

  // C. Portable tar.gz
  const binSrc = findBinary('video-cutter') || findBinary(appName);
  if (binSrc) {
    console.log(`Найден исполняемый файл для Portable: ${binSrc}`);
    const portableDirName = `${appName}_${version}_linux_x64_portable`;
    const portableDir = path.join(distReleasesDir, portableDirName);
    fs.rmSync(portableDir, { recursive: true, force: true });
    fs.mkdirSync(portableDir, { recursive: true });

    fs.copyFileSync(binSrc, path.join(portableDir, 'video-cutter'));
    fs.chmodSync(path.join(portableDir, 'video-cutter'), '755');

    copyDirRecursive(path.join(rootDir, 'src-tauri', 'resources'), path.join(portableDir, 'resources'));
    copyDirRecursive(path.join(rootDir, 'src-tauri', 'models'), path.join(portableDir, 'models'));

    const tarDest = path.join(distReleasesDir, `${appName}_${version}_linux_x64_portable.tar.gz`);
    console.log(`Создание архива Portable tar.gz: ${path.basename(tarDest)}...`);
    execSync(`tar -czf "${tarDest}" -C "${distReleasesDir}" "${portableDirName}"`);
    const sizeMb = (fs.statSync(tarDest).size / (1024 * 1024)).toFixed(2);
    console.log(`[УСПЕХ] Создан Portable tar.gz: ${path.basename(tarDest)} (${sizeMb} MB)`);

    fs.rmSync(portableDir, { recursive: true, force: true });
  } else {
    console.warn('[ПРЕДУПРЕЖДЕНИЕ] video-cutter не найден для создания Portable tar.gz');
  }
}

// 3. macOS Packaging
if (platform === 'darwin') {
  console.log('--- Поиск macOS бандлов и бинарников ---');

  let archTag = 'universal';
  if (target.includes('aarch64') || target.includes('arm64')) {
    archTag = 'arm64';
  } else if (target.includes('x86_64') || target.includes('x64')) {
    archTag = 'x64';
  } else if (process.arch === 'arm64') {
    archTag = 'arm64';
  } else {
    archTag = 'x64';
  }

  // A. DMG
  const dmgFiles = findBundles('dmg', '.dmg');
  for (const dmg of dmgFiles) {
    const dest = path.join(distReleasesDir, `${appName}_${version}_macos_${archTag}.dmg`);
    copyFileSafe(dmg, dest);
  }

  // B. Portable .app в .zip
  const allApps = [];
  for (const relDir of possibleReleaseDirs) {
    const macosBundleDir = path.join(relDir, 'bundle', 'macos');
    if (fs.existsSync(macosBundleDir)) {
      const apps = fs.readdirSync(macosBundleDir).filter(f => f.endsWith('.app')).map(f => path.join(macosBundleDir, f));
      allApps.push(...apps);
    }
    const directBundle = path.join(relDir, 'bundle');
    if (fs.existsSync(directBundle)) {
      const apps = fs.readdirSync(directBundle).filter(f => f.endsWith('.app')).map(f => path.join(directBundle, f));
      allApps.push(...apps);
    }
  }

  const uniqueApps = Array.from(new Set(allApps));
  for (const appPath of uniqueApps) {
    const zipDest = path.join(distReleasesDir, `${appName}_${version}_macos_${archTag}_portable.zip`);
    console.log(`Создание macOS Portable ZIP из ${appPath}...`);
    execSync(`ditto -c -k --keepParent "${appPath}" "${zipDest}"`);
    const sizeMb = (fs.statSync(zipDest).size / (1024 * 1024)).toFixed(2);
    console.log(`[УСПЕХ] Создан macOS Portable ZIP: ${path.basename(zipDest)} (${sizeMb} MB)`);
  }
}

console.log('\n--- СПИСОК ВСЕХ СОЗДАННЫХ ДИСТРИБУТИВОВ В dist-releases: ---');
const allReleases = fs.readdirSync(distReleasesDir);
for (const file of allReleases) {
  const stat = fs.statSync(path.join(distReleasesDir, file));
  console.log(`  * ${file} (${(stat.size / (1024 * 1024)).toFixed(2)} MB)`);
}

if (allReleases.length === 0) {
  console.error('\n❌ ОШИБКА: Ни одного дистрибутива не было создано в dist-releases!');
  if (fs.existsSync(tauriTargetDir)) {
    console.error('Содержимое src-tauri/target:', fs.readdirSync(tauriTargetDir));
  }
  process.exit(1);
}

console.log('============================================================\n');
