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

const releaseDir = path.join(rootDir, 'src-tauri', 'target', 'release');
const bundleDir = path.join(releaseDir, 'bundle');
const distReleasesDir = path.join(rootDir, 'dist-releases');

fs.mkdirSync(distReleasesDir, { recursive: true });

const platform = process.platform;
console.log(`\n=== УПАКОВКА РЕЛИЗОВ И PORTABLE ВЕРСИЙ [${platform}] ===`);
console.log(`Приложение: ${appName} v${version}`);
console.log(`Папка сборки: ${releaseDir}`);
console.log(`Папка назначения: ${distReleasesDir}\n`);

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

// 1. Windows Packaging
if (platform === 'win32') {
  // A. Скопировать установщики NSIS и MSI
  const nsisFiles = findFilesWithExt(path.join(bundleDir, 'nsis'), '.exe');
  for (const nsis of nsisFiles) {
    const dest = path.join(distReleasesDir, `${appName}_${version}_windows_x64_setup.exe`);
    copyFileSafe(nsis, dest);
  }

  const msiFiles = findFilesWithExt(path.join(bundleDir, 'msi'), '.msi');
  for (const msi of msiFiles) {
    const dest = path.join(distReleasesDir, `${appName}_${version}_windows_x64.msi`);
    copyFileSafe(msi, dest);
  }

  // B. Собрать Portable версию
  const exeSrc = path.join(releaseDir, 'video-cutter.exe');
  if (fs.existsSync(exeSrc)) {
    const portableDirName = `${appName}_${version}_windows_x64_portable`;
    const portableDir = path.join(distReleasesDir, portableDirName);
    fs.rmSync(portableDir, { recursive: true, force: true });
    fs.mkdirSync(portableDir, { recursive: true });

    // Копируем исполняемый файл
    fs.copyFileSync(exeSrc, path.join(portableDir, `${appName}.exe`));

    // Копируем ресурсы и sidecars
    const resourcesSrc = path.join(rootDir, 'src-tauri', 'resources');
    const modelsSrc = path.join(rootDir, 'src-tauri', 'models');
    copyDirRecursive(resourcesSrc, path.join(portableDir, 'resources'));
    copyDirRecursive(modelsSrc, path.join(portableDir, 'models'));

    // Инструкция для Portable
    const readme = `=== ${appName} v${version} Portable ===\n\n` +
      `Данная версия не требует установки в систему.\n` +
      `Для запуска откройте "${appName}.exe".\n` +
      `Все локальные модели нейросетей размещаются в папке "models".\n`;
    fs.writeFileSync(path.join(portableDir, 'PORTABLE_README.txt'), readme, 'utf8');

    // Архивируем в ZIP
    const zipDest = path.join(distReleasesDir, `${appName}_${version}_windows_x64_portable.zip`);
    console.log(`Создание архива Portable ZIP: ${path.basename(zipDest)}...`);
    const zip = new AdmZip();
    zip.addLocalFolder(portableDir, portableDirName);
    zip.writeZip(zipDest);
    const sizeMb = (fs.statSync(zipDest).size / (1024 * 1024)).toFixed(2);
    console.log(`[УСПЕХ] Создан Portable ZIP: ${path.basename(zipDest)} (${sizeMb} MB)`);

    // Удаляем временную распакованную папку
    fs.rmSync(portableDir, { recursive: true, force: true });
  }
}

// 2. Linux Packaging
if (platform === 'linux') {
  // A. Установщик DEB
  const debFiles = findFilesWithExt(path.join(bundleDir, 'deb'), '.deb');
  for (const deb of debFiles) {
    const dest = path.join(distReleasesDir, `${appName}_${version}_linux_amd64.deb`);
    copyFileSafe(deb, dest);
  }

  // B. AppImage (уже является готовым портативным исполняемым файлом)
  const appImages = findFilesWithExt(path.join(bundleDir, 'appimage'), '.AppImage');
  for (const appImg of appImages) {
    const dest = path.join(distReleasesDir, `${appName}_${version}_linux_amd64.AppImage`);
    copyFileSafe(appImg, dest);
  }

  // C. Portable tar.gz
  const binSrc = path.join(releaseDir, 'video-cutter');
  if (fs.existsSync(binSrc)) {
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
  }
}

// 3. macOS Packaging
if (platform === 'darwin') {
  // A. Установщик DMG
  const dmgFiles = findFilesWithExt(path.join(bundleDir, 'dmg'), '.dmg');
  for (const dmg of dmgFiles) {
    const dest = path.join(distReleasesDir, `${appName}_${version}_macos.dmg`);
    copyFileSafe(dmg, dest);
  }

  // B. Portable .app в .zip
  const macosBundleDir = path.join(bundleDir, 'macos');
  if (fs.existsSync(macosBundleDir)) {
    const apps = fs.readdirSync(macosBundleDir).filter(f => f.endsWith('.app'));
    for (const app of apps) {
      const appPath = path.join(macosBundleDir, app);
      const zipDest = path.join(distReleasesDir, `${appName}_${version}_macos_portable.zip`);
      console.log(`Создание macOS Portable ZIP из ${app}...`);
      execSync(`ditto -c -k --keepParent "${appPath}" "${zipDest}"`);
      const sizeMb = (fs.statSync(zipDest).size / (1024 * 1024)).toFixed(2);
      console.log(`[УСПЕХ] Создан macOS Portable ZIP: ${path.basename(zipDest)} (${sizeMb} MB)`);
    }
  }
}

console.log('\n--- СПИСОК ВСЕХ СОЗДАННЫХ ДИСТРИБУТИВОВ В dist-releases: ---');
const allReleases = fs.readdirSync(distReleasesDir);
for (const file of allReleases) {
  const stat = fs.statSync(path.join(distReleasesDir, file));
  console.log(`  * ${file} (${(stat.size / (1024 * 1024)).toFixed(2)} MB)`);
}
console.log('============================================================\n');
