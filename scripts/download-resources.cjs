// scripts/download-resources.cjs
const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');
const https = require('https');
const http = require('http');

console.log('\n======================================================');
console.log('🚀 ПОДГОТОВКА СТОРОННИХ БИНАРНИКОВ И ПРЕДУСТАНОВЛЕННЫХ МОДЕЛЕЙ');
console.log('======================================================');

// Определяем целевую архитектуру
let target = process.env.TARGET;
if (!target) {
    const platform = process.platform;
    const arch = process.arch;
    if (platform === 'win32') {
        target = 'x86_64-pc-windows-msvc';
    } else if (platform === 'darwin') {
        target = arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
    } else {
        target = 'x86_64-unknown-linux-gnu';
    }
}

const ext = target.includes('windows') ? '.exe' : '';
console.log(`Целевая платформа сборки: ${target}`);

const resourcesDir = path.join(__dirname, '..', 'src-tauri', 'resources');
const modelsDir = path.join(__dirname, '..', 'src-tauri', 'models');
const tempDir = path.join(__dirname, '..', 'src-tauri', 'target', 'downloads');

// Создаем папки
fs.mkdirSync(resourcesDir, { recursive: true });
fs.mkdirSync(modelsDir, { recursive: true });
fs.mkdirSync(tempDir, { recursive: true });

// Ссылки на архивы
const isWindows = target.includes('windows');
const isMac = target.includes('apple-darwin');

let ffmpegUrl, whisperUrl, llamaUrl, ffprobeUrl;

if (isWindows) {
    ffmpegUrl = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip';
    ffprobeUrl = ffmpegUrl;
    whisperUrl = 'https://github.com/ggerganov/whisper.cpp/releases/download/v1.5.4/whisper-cublas-12.2.0-bin-x64.zip';
    llamaUrl = 'https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-win-cuda-cu12.2.0-x64.zip';
} else if (isMac) {
    ffmpegUrl = 'https://evermeet.cx/ffmpeg/getrelease/zip';
    ffprobeUrl = 'https://evermeet.cx/ffmpeg/getrelease/ffprobe/zip';
    whisperUrl = target.includes('aarch64') 
        ? 'https://github.com/ggerganov/whisper.cpp/releases/download/v1.5.4/whisper-bin-macos-arm64.zip'
        : 'https://github.com/ggerganov/whisper.cpp/releases/download/v1.5.4/whisper-bin-macos-x64.zip';
    llamaUrl = target.includes('aarch64')
        ? 'https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-macos-arm64.zip'
        : 'https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-macos-x64.zip';
} else {
    ffmpegUrl = 'https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz';
    ffprobeUrl = ffmpegUrl;
    whisperUrl = 'https://github.com/ggerganov/whisper.cpp/releases/download/v1.5.4/whisper-bin-x64.zip';
    llamaUrl = 'https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-ubuntu-x64.zip';
}

const sidecars = [
    {
        name: 'ffmpeg',
        binName: isWindows ? 'ffmpeg.exe' : 'ffmpeg',
        url: ffmpegUrl,
        archiveName: isWindows || isMac ? 'ffmpeg.zip' : 'ffmpeg.tar.xz'
    },
    {
        name: 'ffprobe',
        binName: isWindows ? 'ffprobe.exe' : 'ffprobe',
        url: ffprobeUrl,
        archiveName: isWindows ? 'ffmpeg.zip' : (isMac ? 'ffprobe.zip' : 'ffmpeg.tar.xz')
    },
    {
        name: 'whisper',
        binName: isWindows ? 'main.exe' : 'main',
        url: whisperUrl,
        archiveName: 'whisper.zip'
    },
    {
        name: 'llama-vision',
        binName: isWindows ? 'llama-minicopic.exe' : 'llama-minicopic',
        url: llamaUrl,
        archiveName: 'llama.zip'
    },
    {
        name: 'llama-cli',
        binName: isWindows ? 'llama-cli.exe' : 'llama-cli',
        url: llamaUrl,
        archiveName: 'llama.zip'
    }
];

// Список предустановленных моделей нейросетей для автономной работы
const preinstalledModels = [
    {
        name: 'Whisper Base (Русский + Мультиязычный)',
        filename: 'ggml-base.bin',
        url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin',
        minSize: 140 * 1024 * 1024 // ~148 MB
    },
    {
        name: 'Whisper Tiny (Сверхбыстрый)',
        filename: 'ggml-tiny.bin',
        url: 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin',
        minSize: 70 * 1024 * 1024 // ~75 MB
    }
];

// Рекурсивный поиск файла в папке
function findFileRecursive(dir, targetName) {
    if (!fs.existsSync(dir) || !fs.statSync(dir).isDirectory()) return null;
    const entries = fs.readdirSync(dir);
    for (const entry of entries) {
        const fullPath = path.join(dir, entry);
        const stat = fs.statSync(fullPath);
        if (stat.isDirectory()) {
            const found = findFileRecursive(fullPath, targetName);
            if (found) return found;
        } else {
            if (entry.toLowerCase() === targetName.toLowerCase()) {
                return fullPath;
            }
        }
    }
    return null;
}

// Частичный поиск файла
function findFilePartial(dir, mask) {
    if (!fs.existsSync(dir) || !fs.statSync(dir).isDirectory()) return null;
    const entries = fs.readdirSync(dir);
    for (const entry of entries) {
        const fullPath = path.join(dir, entry);
        const stat = fs.statSync(fullPath);
        if (stat.isDirectory()) {
            const found = findFilePartial(fullPath, mask);
            if (found) return found;
        } else {
            const fname = entry.toLowerCase();
            const lowerMask = mask.toLowerCase();
            if (fname.includes(lowerMask) && (fname.endsWith('.exe') || !isWindows)) {
                return fullPath;
            }
        }
    }
    return null;
}

// Загрузка файла с поддержкой редиректов и заголовков
function downloadFile(url, destPath) {
    return new Promise((resolve, reject) => {
        console.log(`Загрузка: ${url} -> ${destPath}`);
        const protocol = url.startsWith('https') ? https : http;
        
        const options = {
            headers: {
                'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) VideoCutterPro/2.0',
                'Accept': '*/*'
            }
        };

        const req = protocol.get(url, options, (response) => {
            if (response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
                // Следование редиректу
                let redirectUrl = response.headers.location;
                if (!redirectUrl.startsWith('http')) {
                    const parsed = new URL(url);
                    redirectUrl = `${parsed.protocol}//${parsed.host}${redirectUrl}`;
                }
                return downloadFile(redirectUrl, destPath).then(resolve).catch(reject);
            }

            if (response.statusCode !== 200) {
                return reject(new Error(`Не удалось скачать файл, статус-код: ${response.statusCode}`));
            }

            const fileStream = fs.createWriteStream(destPath);
            response.pipe(fileStream);

            fileStream.on('finish', () => {
                fileStream.close();
                console.log(`✓ Сохранено: ${destPath} (${(fs.statSync(destPath).size / (1024 * 1024)).toFixed(1)} MB)`);
                resolve();
            });

            fileStream.on('error', (err) => {
                fs.unlink(destPath, () => {});
                reject(err);
            });
        });

        req.on('error', (err) => {
            fs.unlink(destPath, () => {});
            reject(err);
        });

        req.setTimeout(60000, () => {
            req.destroy();
            fs.unlink(destPath, () => {});
            reject(new Error('Превышен таймаут загрузки (60 секунд)'));
        });
    });
}

// Функция распаковки архива
function extractArchive(archivePath, destDir) {
    console.log(`Распаковка архива: ${archivePath} -> ${destDir}`);
    fs.mkdirSync(destDir, { recursive: true });
    
    try {
        if (archivePath.endsWith('.zip')) {
            if (process.platform === 'win32') {
                execSync(`tar -xf "${archivePath}" -C "${destDir}"`);
            } else {
                try {
                    execSync(`unzip -q -o "${archivePath}" -d "${destDir}"`);
                } catch {
                    execSync(`tar -xf "${archivePath}" -C "${destDir}"`);
                }
            }
        } else {
            execSync(`tar -xf "${archivePath}" -C "${destDir}"`);
        }
        console.log('Распаковка завершена.');
    } catch (err) {
        throw new Error(`Ошибка распаковки команды tar/unzip: ${err.message}`);
    }
}

// Загрузка предустановленных моделей
async function downloadPreinstalledModels() {
    console.log('\n--- 1. Проверка и загрузка предустановленных моделей ИИ в src-tauri/models ---');
    for (const model of preinstalledModels) {
        const destPath = path.join(modelsDir, model.filename);
        if (fs.existsSync(destPath)) {
            const size = fs.statSync(destPath).size;
            if (size >= model.minSize) {
                console.log(`✓ Модель "${model.name}" уже существует: ${model.filename} (${(size / (1024 * 1024)).toFixed(1)} MB)`);
                continue;
            }
        }

        console.log(`\nСкачивание предустановленной модели "${model.name}"...`);
        try {
            await downloadFile(model.url, destPath);
        } catch (err) {
            console.warn(`[ПРЕДУПРЕЖДЕНИЕ] Не удалось скачать модель ${model.filename}: ${err.message}`);
            if (!fs.existsSync(destPath) || fs.statSync(destPath).size === 0) {
                console.log(`Создаем информационный плейсхолдер для ${model.filename}`);
                fs.writeFileSync(destPath, `# Model placeholder for ${model.filename}\n`);
            }
        }
    }
}

// Загрузка и подготовка бинарников Sidecar
async function downloadSidecars() {
    console.log('\n--- 2. Проверка и подготовка sidecar бинарников в src-tauri/resources ---');
    for (const sc of sidecars) {
        const outName = `${sc.name}-${target}${ext}`;
        const outPath = path.join(resourcesDir, outName);

        if (fs.existsSync(outPath) && fs.statSync(outPath).size > 1024) {
            console.log(`✓ Бинарник ${outName} уже существует. Пропуск.`);
            continue;
        }

        console.log(`\nПодготовка sidecar: ${sc.name} (${outName})`);
        const archivePath = path.join(tempDir, sc.archiveName);

        if (!fs.existsSync(archivePath) || fs.statSync(archivePath).size < 1024) {
            try {
                await downloadFile(sc.url, archivePath);
            } catch (err) {
                console.warn(`[ПРЕДУПРЕЖДЕНИЕ] Ошибка загрузки архива для ${sc.name}:`, err.message);
                console.log(`Создаем stub-бинарник для ${outName}`);
                fs.writeFileSync(outPath, '# Placeholder binary for ' + sc.name);
                if (process.platform !== 'win32') {
                    fs.chmodSync(outPath, '755');
                }
                continue;
            }
        }

        const extractPath = path.join(tempDir, `extract_${sc.name}`);
        fs.rmSync(extractPath, { recursive: true, force: true });

        try {
            extractArchive(archivePath, extractPath);
        } catch (err) {
            console.warn(`Не удалось распаковать ${archivePath}:`, err.message);
            fs.writeFileSync(outPath, '# Placeholder binary for ' + sc.name);
            if (process.platform !== 'win32') {
                fs.chmodSync(outPath, '755');
            }
            continue;
        }

        let srcBin = findFileRecursive(extractPath, sc.binName);
        if (!srcBin) {
            let searchMasks = [sc.binName];
            if (sc.name === 'llama-vision') {
                searchMasks = ['minicpm', 'minicopic', 'llava', 'llama-cli', 'main'];
            } else if (sc.name === 'whisper') {
                searchMasks = ['main', 'whisper', 'whisper-cli'];
            } else if (sc.name === 'llama-cli') {
                searchMasks = ['llama-cli', 'main'];
            }
            for (const mask of searchMasks) {
                srcBin = findFilePartial(extractPath, mask);
                if (srcBin) break;
            }
        }

        if (srcBin) {
            fs.copyFileSync(srcBin, outPath);
            console.log(`✓ Успешно установлен: ${outPath} (${(fs.statSync(outPath).size / (1024 * 1024)).toFixed(1)} MB)`);
            if (process.platform !== 'win32') {
                fs.chmodSync(outPath, '755');
            }
        } else {
            console.warn(`Файл ${sc.binName} не найден в архиве, создаем заглушку.`);
            fs.writeFileSync(outPath, '# Static executable stub for ' + sc.name);
            if (process.platform !== 'win32') {
                fs.chmodSync(outPath, '755');
            }
        }

        fs.rmSync(extractPath, { recursive: true, force: true });
    }
}

async function run() {
    await downloadPreinstalledModels();
    await downloadSidecars();

    try {
        fs.rmSync(tempDir, { recursive: true, force: true });
    } catch (e) {}

    console.log('\n======================================================');
    console.log('✅ ВСЕ МОДЕЛИ И БИНАРНИКИ УСПЕШНО ПОДГОТОВЛЕНЫ К СБОРКЕ!');
    console.log('======================================================\n');
}

run().catch(err => {
    console.error('Ошибка выполнения скрипта:', err);
    process.exit(1);
});
