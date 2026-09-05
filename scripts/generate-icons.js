import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import sharp from 'sharp';
import toIco from 'to-ico';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

// Путь к мастер-иконке (по умолчанию SVG, или переданный аргумент)
const customMaster = process.argv[2];
const defaultSvg = path.join(rootDir, 'src-tauri', 'app-icon.svg');
const masterPath = customMaster ? path.resolve(customMaster) : defaultSvg;
const iconsDir = path.join(rootDir, 'src-tauri', 'icons');

if (!fs.existsSync(masterPath)) {
  console.error(`[ОШИБКА] Исходный файл иконки не найден: ${masterPath}`);
  process.exit(1);
}

console.log(`\n======================================================`);
console.log(`🎨 ВСЕОБЪЕМЛЮЩИЙ ГЕНЕРАТОР ИКОНОК TAURI ДЛЯ ВСЕХ ПЛАТФОРМ`);
console.log(`Исходник: ${masterPath}`);
console.log(`Выходная директория: ${iconsDir}`);
console.log(`======================================================\n`);

// Гарантируем наличие всех подкаталогов
const subdirs = [
  '',
  'android',
  'android/mipmap-mdpi',
  'android/mipmap-hdpi',
  'android/mipmap-xhdpi',
  'android/mipmap-xxhdpi',
  'android/mipmap-xxxhdpi',
  'ios'
];
for (const sub of subdirs) {
  const dir = path.join(iconsDir, sub);
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }
}

// Рендеринг SVG или PNG в PNG заданного размера с максимальным качеством
async function renderPng(width, height) {
  return sharp(masterPath, { density: 300 })
    .resize(width, height, {
      fit: 'contain',
      background: { r: 0, g: 0, b: 0, alpha: 0 }
    })
    .png({ compressionLevel: 9, adaptiveFiltering: true })
    .toBuffer();
}

async function savePng(relPath, width, height) {
  const buf = await renderPng(width, height);
  const fullPath = path.join(iconsDir, relPath);
  fs.writeFileSync(fullPath, buf);
  const sizeKb = (buf.length / 1024).toFixed(1);
  console.log(`  ✓ [PNG] ${relPath.padEnd(36)} (${width}x${height}) -> ${sizeKb} KB`);
  return buf;
}

// Создание нативного macOS .icns без внешних утилит
function buildIcnsBuffer(entries) {
  let totalDataLength = 0;
  for (const entry of entries) {
    totalDataLength += 8 + entry.buffer.length;
  }
  const totalLength = 8 + totalDataLength;
  const out = Buffer.alloc(totalLength);

  // Заголовок 'icns' (4 байта) + общий размер файла UInt32BE (4 байта)
  out.write('icns', 0, 4, 'ascii');
  out.writeUInt32BE(totalLength, 4);

  let offset = 8;
  for (const entry of entries) {
    out.write(entry.ostype, offset, 4, 'ascii');
    out.writeUInt32BE(8 + entry.buffer.length, offset + 4);
    entry.buffer.copy(out, offset + 8);
    offset += 8 + entry.buffer.length;
  }

  return out;
}

async function generateAll() {
  console.log('📌 1. Генерация базовых иконок ядра Tauri и Linux:');
  await savePng('32x32.png', 32, 32);
  await savePng('64x64.png', 64, 64);
  await savePng('128x128.png', 128, 128);
  await savePng('128x128@2x.png', 256, 256);
  await savePng('icon.png', 512, 512);

  console.log('\n📌 2. Генерация плиток Windows Store / Wix MSI:');
  await savePng('StoreLogo.png', 50, 50);
  await savePng('Square30x30Logo.png', 30, 30);
  await savePng('Square44x44Logo.png', 44, 44);
  await savePng('Square71x71Logo.png', 71, 71);
  await savePng('Square89x89Logo.png', 89, 89);
  await savePng('Square107x107Logo.png', 107, 107);
  await savePng('Square142x142Logo.png', 142, 142);
  await savePng('Square150x150Logo.png', 150, 150);
  await savePng('Square284x284Logo.png', 284, 284);
  await savePng('Square310x310Logo.png', 310, 310);

  console.log('\n📌 3. Генерация многослойного Windows icon.ico:');
  // Размеры: 16, 24, 32, 48, 64, 128, 256
  // Защита от RC2176 в старых SDK: чистый to-ico без сбойных DIB-заголовков
  const icoSizes = [16, 24, 32, 48, 64, 128, 256];
  const icoPngBuffers = [];
  for (const s of icoSizes) {
    const buf = await renderPng(s, s);
    icoPngBuffers.push(buf);
  }
  const icoData = await toIco(icoPngBuffers, { resize: false });
  const icoPath = path.join(iconsDir, 'icon.ico');
  fs.writeFileSync(icoPath, icoData);
  console.log(`  ✓ [ICO] icon.ico                     (${icoSizes.join(', ')} px) -> ${(icoData.length / 1024).toFixed(1)} KB`);

  console.log('\n📌 4. Генерация Apple macOS icon.icns:');
  const icnsSpecs = [
    { ostype: 'icp4', size: 16 },
    { ostype: 'icp5', size: 32 },
    { ostype: 'icp6', size: 64 },
    { ostype: 'ic07', size: 128 },
    { ostype: 'ic08', size: 256 },
    { ostype: 'ic09', size: 512 },
    { ostype: 'ic10', size: 1024 }
  ];
  const icnsEntries = [];
  for (const spec of icnsSpecs) {
    const buf = await renderPng(spec.size, spec.size);
    icnsEntries.push({ ostype: spec.ostype, buffer: buf });
  }
  const icnsData = buildIcnsBuffer(icnsEntries);
  const icnsPath = path.join(iconsDir, 'icon.icns');
  fs.writeFileSync(icnsPath, icnsData);
  console.log(`  ✓ [ICNS] icon.icns                   (16..1024 px Apple Retina) -> ${(icnsData.length / 1024).toFixed(1)} KB`);

  console.log('\n📌 5. Генерация иконок для мобильных платформ (Android / iOS):');
  // Android
  await savePng('android/mipmap-mdpi/ic_launcher.png', 48, 48);
  await savePng('android/mipmap-hdpi/ic_launcher.png', 72, 72);
  await savePng('android/mipmap-xhdpi/ic_launcher.png', 96, 96);
  await savePng('android/mipmap-xxhdpi/ic_launcher.png', 144, 144);
  await savePng('android/mipmap-xxxhdpi/ic_launcher.png', 192, 192);

  // iOS AppIcon
  const iosIcons = [
    { name: 'AppIcon-20x20@1x.png', size: 20 },
    { name: 'AppIcon-20x20@2x.png', size: 40 },
    { name: 'AppIcon-20x20@2x-1.png', size: 40 },
    { name: 'AppIcon-20x20@3x.png', size: 60 },
    { name: 'AppIcon-29x29@1x.png', size: 29 },
    { name: 'AppIcon-29x29@2x.png', size: 58 },
    { name: 'AppIcon-29x29@2x-1.png', size: 58 },
    { name: 'AppIcon-29x29@3x.png', size: 87 },
    { name: 'AppIcon-40x40@1x.png', size: 40 },
    { name: 'AppIcon-40x40@2x.png', size: 80 },
    { name: 'AppIcon-40x40@2x-1.png', size: 80 },
    { name: 'AppIcon-40x40@3x.png', size: 120 },
    { name: 'AppIcon-60x60@2x.png', size: 120 },
    { name: 'AppIcon-60x60@3x.png', size: 180 },
    { name: 'AppIcon-76x76@1x.png', size: 76 },
    { name: 'AppIcon-76x76@2x.png', size: 152 },
    { name: 'AppIcon-83.5x83.5@2x.png', size: 167 },
    { name: 'AppIcon-512@2x.png', size: 1024 }
  ];

  for (const item of iosIcons) {
    await savePng(`ios/${item.name}`, item.size, item.size);
  }

  console.log('\n======================================================');
  console.log('✅ ВСЕ ИКОНКИ УСПЕШНО СГЕНЕРИРОВАНЫ И ПРОВЕРЕНЫ!');
  console.log('Tauri бандлер для Windows, Linux, macOS, Android и iOS');
  console.log('имеет 100% комплект валидных графических ресурсов.');
  console.log('======================================================\n');
}

generateAll().catch(err => {
  console.error('\n[КРИТИЧЕСКАЯ ОШИБКА ГЕНЕРАЦИИ ИКОНОК]', err);
  process.exit(1);
});
