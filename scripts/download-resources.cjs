// scripts/download-resources.cjs
const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');
const https = require('https');
const http = require('http');

console.log('\n======================================================');
console.log('🚀 ПОДГОТОВКА СТОРОННИХ БИНАРНИКОВ К СБОРКЕ');
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

// Гарантируем наличие placeholder.dll для маски resources/*.dll в tauri.conf.json на всех ОС
const placeholderDll = path.join(resourcesDir, 'placeholder.dll');
if (!fs.existsSync(placeholderDll)) {
    fs.writeFileSync(placeholderDll, '# Placeholder DLL file for tauri glob matching\n');
}

// Гарантируем наличие placeholder для маски models/* в tauri.conf.json (все модели скачиваются пользователем по требованию в приложении)
const placeholderModel = path.join(modelsDir, '.gitkeep');
if (!fs.existsSync(placeholderModel)) {
    fs.writeFileSync(placeholderModel, '# Models are downloaded on demand in-app\n');
}

// Ссылки на архивы
const isWindows = target.includes('windows');
const isMac = target.includes('apple-darwin');

let ffmpegUrls = [], whisperUrls = [], llamaUrls = [], ffprobeUrls = [];

if (isWindows) {
    // Надежные источники FFmpeg: Gyan.dev (официальный рекомендованный релиз) + BtbN + Tyrrrz
    ffmpegUrls = [
        'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip',
        'https://github.com/Tyrrrz/FFmpegBin/releases/download/9.0.2/ffmpeg-windows-x64.zip',
        'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip'
    ];
    ffprobeUrls = ffmpegUrls;
    whisperUrls = [
        'https://github.com/ggml-org/whisper.cpp/releases/download/v1.5.4/whisper-cublas-12.2.0-bin-x64.zip',
        'https://github.com/ggerganov/whisper.cpp/releases/download/v1.5.4/whisper-cublas-12.2.0-bin-x64.zip',
        'https://github.com/ggml-org/whisper.cpp/releases/download/v1.5.4/whisper-bin-x64.zip'
    ];
    llamaUrls = [
        'https://github.com/ggml-org/llama.cpp/releases/download/b3201/llama-b3201-bin-win-cuda-cu12.2.0-x64.zip',
        'https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-win-cuda-cu12.2.0-x64.zip'
    ];
} else if (isMac) {
    const isArm64 = target.includes('aarch64') || target.includes('arm64');
    const macArch = isArm64 ? 'arm64' : 'x64';
    ffmpegUrls = [
        `https://github.com/Tyrrrz/FFmpegBin/releases/download/9.0.2/ffmpeg-osx-${macArch}.zip`
    ];
    ffprobeUrls = ffmpegUrls;
    whisperUrls = isArm64 ? [
        'https://github.com/bizenlabs/whisper-cpp-macos-bin/releases/download/v1.8.2/whisper-cpp-v1.8.2-macos-arm64-metal.zip',
        'https://github.com/bizenlabs/whisper-cpp-macos-bin/releases/latest/download/whisper-cpp-macos-arm64-metal.zip'
    ] : [
        'https://github.com/bizenlabs/whisper-cpp-macos-bin/releases/download/v1.8.2/whisper-cpp-v1.8.2-macos-x86_64-accelerate.zip',
        'https://github.com/bizenlabs/whisper-cpp-macos-bin/releases/latest/download/whisper-cpp-macos-x86_64-accelerate.zip'
    ];
    llamaUrls = isArm64 ? [
        'https://github.com/ggml-org/llama.cpp/releases/download/b3201/llama-b3201-bin-macos-arm64.zip',
        'https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-macos-arm64.zip'
    ] : [
        'https://github.com/ggml-org/llama.cpp/releases/download/b3201/llama-b3201-bin-macos-x64.zip',
        'https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-macos-x64.zip'
    ];
} else {
    ffmpegUrls = [
        'https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz'
    ];
    ffprobeUrls = ffmpegUrls;
    whisperUrls = [
        'https://github.com/ggml-org/whisper.cpp/releases/download/v1.5.4/whisper-bin-x64.zip',
        'https://github.com/ggerganov/whisper.cpp/releases/download/v1.5.4/whisper-bin-x64.zip'
    ];
    llamaUrls = [
        'https://github.com/ggml-org/llama.cpp/releases/download/b3201/llama-b3201-bin-ubuntu-x64.zip',
        'https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-ubuntu-x64.zip'
    ];
}

const sidecars = [
    {
        name: 'ffmpeg',
        binName: isWindows ? 'ffmpeg.exe' : 'ffmpeg',
        urls: ffmpegUrls,
        archiveName: isWindows || isMac ? 'ffmpeg.zip' : 'ffmpeg.tar.xz'
    },
    {
        name: 'ffprobe',
        binName: isWindows ? 'ffprobe.exe' : 'ffprobe',
        urls: ffprobeUrls,
        archiveName: isWindows || isMac ? 'ffmpeg.zip' : 'ffmpeg.tar.xz'
    },
    {
        name: 'whisper',
        binName: isWindows ? 'main.exe' : 'main',
        urls: whisperUrls,
        archiveName: 'whisper.zip'
    },
    {
        name: 'llama-vision',
        binName: isWindows ? 'llama-minicopic.exe' : 'llama-minicopic',
        urls: llamaUrls,
        archiveName: 'llama.zip'
    },
    {
        name: 'llama-cli',
        binName: isWindows ? 'llama-cli.exe' : 'llama-cli',
        urls: llamaUrls,
        archiveName: 'llama.zip'
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

// Загрузка файла с поддержкой редиректов, повторных попыток (curl --retry) и заголовков
function downloadFile(url, destPath) {
    try {
        console.log(`Загрузка (curl с авто-повтором при 502/504): ${url} -> ${destPath}`);
        execSync(
            `curl -fL --retry 5 --retry-delay 3 --retry-all-errors --connect-timeout 30 -A "Mozilla/5.0 VideoCutterPro/2.1" -o "${destPath}" "${url}"`,
            { stdio: 'inherit' }
        );
        if (fs.existsSync(destPath) && fs.statSync(destPath).size > 1024) {
            console.log(`✓ Сохранено: ${destPath} (${(fs.statSync(destPath).size / (1024 * 1024)).toFixed(1)} MB)`);
            return Promise.resolve();
        }
    } catch (curlErr) {
        console.warn(`Предупреждение curl (${curlErr.message}), переключаемся на встроенный HTTPS-загрузчик Node.js...`);
    }

    return new Promise((resolve, reject) => {
        console.log(`Загрузка (Node HTTPS): ${url} -> ${destPath}`);
        const protocol = url.startsWith('https') ? https : http;
        
        const options = {
            headers: {
                'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) VideoCutterPro/2.1',
                'Accept': '*/*'
            }
        };

        const req = protocol.get(url, options, (response) => {
            if (response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
                let redirectUrl = response.headers.location;
                if (!redirectUrl.startsWith('http')) {
                    const parsed = new URL(url);
                    redirectUrl = `${parsed.protocol}//${parsed.host}${redirectUrl}`;
                }
                response.resume();
                req.setTimeout(0);
                return downloadFile(redirectUrl, destPath).then(resolve).catch(reject);
            }

            if (response.statusCode !== 200) {
                response.resume();
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

        req.setTimeout(300000, () => {
            req.destroy();
            fs.unlink(destPath, () => {});
            reject(new Error('Превышен таймаут загрузки (300 секунд)'));
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

// Подготовка директории моделей (базовая модель ggml-base.bin для оффлайн-работы из коробки)
async function prepareModelsDirectory() {
    console.log('\n--- 1. Подготовка директории src-tauri/models ---');
    if (!fs.existsSync(modelsDir)) {
        fs.mkdirSync(modelsDir, { recursive: true });
    }
    const placeholderModel = path.join(modelsDir, '.gitkeep');
    if (!fs.existsSync(placeholderModel)) {
        fs.writeFileSync(placeholderModel, '# Models directory\n');
    }

    const isMobile = target.includes('android') || target.includes('ios');
    if (isMobile) {
        console.log('✓ Мобильная цель сборки: пропуск предзагрузки ggml-base.bin.');
        return;
    }

    const baseModelPath = path.join(modelsDir, 'ggml-base.bin');
    const baseModelUrl = 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin';

    if (fs.existsSync(baseModelPath) && fs.statSync(baseModelPath).size > 100 * 1024 * 1024) {
        console.log(`✓ Базовая модель Whisper (ggml-base.bin) уже загружена (${(fs.statSync(baseModelPath).size / (1024 * 1024)).toFixed(1)} MB).`);
        return;
    }

    console.log('Загрузка базовой модели Whisper (ggml-base.bin, ~141 MB) для работы из коробки...');
    await downloadFile(baseModelUrl, baseModelPath);
}

// Загрузка и подготовка бинарников Sidecar
async function downloadSidecars() {
    console.log('\n--- 2. Проверка и подготовка sidecar бинарников в src-tauri/resources ---');
    const isMobile = target.includes('android') || target.includes('ios');

    for (const sc of sidecars) {
        const outName = `${sc.name}-${target}${ext}`;
        const outPath = path.join(resourcesDir, outName);

        if (isMobile) {
            if (!fs.existsSync(outPath)) {
                fs.writeFileSync(outPath, `# Mobile sidecar stub for ${sc.name}\n`);
            }
            console.log(`✓ Создан мобильный stub-ресурс: ${outName}`);
            continue;
        }

        if (fs.existsSync(outPath) && fs.statSync(outPath).size > 100 * 1024) {
            console.log(`✓ Бинарник ${outName} уже существует (${(fs.statSync(outPath).size / (1024 * 1024)).toFixed(1)} MB). Пропуск.`);
            continue;
        }

        console.log(`\nПодготовка sidecar: ${sc.name} (${outName})`);

        // На macOS: если whisper-cli уже установлен через Homebrew и имеет реальный размер (не шпионский wrapper-скрипт)
        if (isMac && !fs.existsSync(outPath) && sc.name === 'whisper') {
            const candidates = [
                '/opt/homebrew/bin/whisper-cli',
                '/usr/local/bin/whisper-cli'
            ];
            for (const c of candidates) {
                if (fs.existsSync(c) && fs.statSync(c).size > 100 * 1024) {
                    console.log(`✓ Найден системный Homebrew бинарник для ${sc.name}: ${c}`);
                    fs.copyFileSync(c, outPath);
                    fs.chmodSync(outPath, '755');
                    console.log(`✓ Успешно установлен из системы: ${outPath} (${(fs.statSync(outPath).size / (1024 * 1024)).toFixed(1)} MB)`);
                    break;
                }
            }
            if (fs.existsSync(outPath)) {
                continue;
            }
        }

        const archivePath = path.join(tempDir, sc.archiveName);

        if (!fs.existsSync(archivePath) || fs.statSync(archivePath).size < 100 * 1024) {
            let downloaded = false;
            let lastError = null;
            const urlList = Array.isArray(sc.urls) ? sc.urls : [sc.url];
            for (const currentUrl of urlList) {
                try {
                    console.log(`Попытка загрузки ${sc.name} из источника: ${currentUrl}`);
                    await downloadFile(currentUrl, archivePath);
                    if (fs.existsSync(archivePath) && fs.statSync(archivePath).size > 100 * 1024) {
                        downloaded = true;
                        break;
                    }
                } catch (err) {
                    lastError = err;
                    console.warn(`[ПРЕДУПРЕЖДЕНИЕ] Ошибка загрузки из ${currentUrl}: ${err.message}`);
                    if (fs.existsSync(archivePath)) {
                        fs.unlinkSync(archivePath);
                    }
                }
            }
            if (!downloaded) {
                throw new Error(`Не удалось скачать архив для sidecar ${sc.name}: ${lastError ? lastError.message : 'все источники недоступны'}`);
            }
        }

        const extractPath = path.join(tempDir, `extract_${sc.name}`);
        fs.rmSync(extractPath, { recursive: true, force: true });

        extractArchive(archivePath, extractPath);

        let srcBin = findFileRecursive(extractPath, sc.binName);
        if (!srcBin) {
            let searchMasks = [sc.binName];
            if (sc.name === 'llama-vision') {
                searchMasks = ['llava-cli', 'minicpm', 'minicopic', 'llava', 'llama-cli', 'main'];
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

        // Копируем все зависимые DLL (whisper.dll, ggml.dll, cublas, cudart, SDL2 и т.д.)
        copyAllDlls(extractPath, resourcesDir);

        if (srcBin) {
            fs.copyFileSync(srcBin, outPath);
            console.log(`✓ Успешно установлен: ${outPath} (${(fs.statSync(outPath).size / (1024 * 1024)).toFixed(1)} MB)`);
            if (process.platform !== 'win32') {
                fs.chmodSync(outPath, '755');
            }
        } else {
            throw new Error(`Критическая ошибка: исполняемый файл ${sc.binName} для ${sc.name} не найден в распакованном архиве ${sc.archiveName}`);
        }

        fs.rmSync(extractPath, { recursive: true, force: true });
    }
}

// Предварительное кэширование NSIS и WiX (WixTools314) для Tauri на Windows (защита от HTTP 504 при сборке инсталляторов .exe и .msi)
async function prewarmWindowsBundlerTools() {
    if (!isWindows || process.platform !== 'win32') return;
    const localAppData = process.env.LOCALAPPDATA;
    if (!localAppData) return;

    console.log('\n--- 3. Предварительное кэширование инструментов сборки Tauri (NSIS & WiX 3.14) ---');
    const tauriCacheDir = path.join(localAppData, 'tauri');
    const nsisDir = path.join(tauriCacheDir, 'NSIS');
    const makensisPath = path.join(nsisDir, 'makensis.exe');
    const nsisZipPath = path.join(tempDir, 'nsis-3.zip');
    const nsisUrl = 'https://github.com/tauri-apps/binary-releases/releases/download/nsis-3/nsis-3.zip';

    const utilsDllDir = path.join(nsisDir, 'Plugins', 'x86-unicode');
    const utilsDllPath = path.join(utilsDllDir, 'nsis_tauri_utils.dll');
    const utilsDllUrl = 'https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.4.1/nsis_tauri_utils.dll';

    const appIdDllPath = path.join(utilsDllDir, 'ApplicationID.dll');
    const appIdZipPath = path.join(tempDir, 'NSIS-ApplicationID.zip');
    const appIdUrl = 'https://github.com/tauri-apps/binary-releases/releases/download/nsis-plugins-v0/NSIS-ApplicationID.zip';

    try {
        if (!fs.existsSync(makensisPath)) {
            fs.mkdirSync(nsisDir, { recursive: true });
            await downloadFile(nsisUrl, nsisZipPath);
            extractArchive(nsisZipPath, nsisDir);
            const foundMakensis = findFileRecursive(nsisDir, 'makensis.exe');
            if (foundMakensis && path.dirname(foundMakensis) !== nsisDir) {
                const subDir = path.dirname(foundMakensis);
                for (const item of fs.readdirSync(subDir)) {
                    const srcItem = path.join(subDir, item);
                    const dstItem = path.join(nsisDir, item);
                    if (!fs.existsSync(dstItem)) {
                        fs.renameSync(srcItem, dstItem);
                    }
                }
            }
            console.log('✓ Кэш NSIS для Tauri успешно подготовлен.');
        } else {
            console.log('✓ Кэш NSIS уже присутствует.');
        }

        fs.mkdirSync(utilsDllDir, { recursive: true });

        if (!fs.existsSync(appIdDllPath)) {
            await downloadFile(appIdUrl, appIdZipPath);
            const appIdExtractDir = path.join(tempDir, 'extract_appid');
            fs.rmSync(appIdExtractDir, { recursive: true, force: true });
            extractArchive(appIdZipPath, appIdExtractDir);
            const foundAppId = findFileRecursive(appIdExtractDir, 'ApplicationID.dll');
            if (foundAppId) {
                fs.copyFileSync(foundAppId, appIdDllPath);
                console.log('✓ Плагин ApplicationID.dll успешно закэширован.');
            }
            fs.rmSync(appIdExtractDir, { recursive: true, force: true });
        }

        if (!fs.existsSync(utilsDllPath)) {
            await downloadFile(utilsDllUrl, utilsDllPath);
            console.log('✓ Плагин nsis_tauri_utils.dll успешно закэширован.');
        }
    } catch (err) {
        console.warn('Предупреждение при прогреве кэша NSIS:', err.message);
    }

    // Прогрев WiX Toolset 3.14 (для сборки .msi без HTTP 504 от GitHub Releases)
    const wixDir = path.join(tauriCacheDir, 'WixTools314');
    const candlePath = path.join(wixDir, 'candle.exe');
    const lightPath = path.join(wixDir, 'light.exe');
    const wixZipPath = path.join(tempDir, 'wix314-binaries.zip');
    const wixUrl = 'https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip';

    try {
        if (!fs.existsSync(candlePath) || !fs.existsSync(lightPath)) {
            fs.mkdirSync(wixDir, { recursive: true });
            await downloadFile(wixUrl, wixZipPath);
            extractArchive(wixZipPath, wixDir);
            const foundCandle = findFileRecursive(wixDir, 'candle.exe');
            if (foundCandle && path.dirname(foundCandle) !== wixDir) {
                const subDir = path.dirname(foundCandle);
                for (const item of fs.readdirSync(subDir)) {
                    const srcItem = path.join(subDir, item);
                    const dstItem = path.join(wixDir, item);
                    if (!fs.existsSync(dstItem)) {
                        fs.renameSync(srcItem, dstItem);
                    }
                }
            }
            console.log('✓ Кэш WiX Toolset 3.14 (WixTools314) успешно подготовлен.');
        } else {
            console.log('✓ Кэш WiX Toolset 3.14 уже присутствует.');
        }
    } catch (err) {
        console.warn('Предупреждение при прогреве кэша WiX 3.14:', err.message);
    }
}

// Верификация целостности всех бинарников и библиотек перед сборкой десктопного бандла
function verifyDesktopResources() {
    const isMobile = target.includes('android') || target.includes('ios');
    if (isMobile) return;

    console.log('\n--- 4. Верификация целостности десктопных ресурсов ---');
    for (const sc of sidecars) {
        const outName = `${sc.name}-${target}${ext}`;
        const outPath = path.join(resourcesDir, outName);
        if (!fs.existsSync(outPath)) {
            throw new Error(`Отсутствует обязательный sidecar-бинарник: ${outPath}`);
        }
        const size = fs.statSync(outPath).size;
        if (size < 100 * 1024) {
            throw new Error(`Поврежден или является заглушкой sidecar-бинарник ${outName} (${size} байт)`);
        }
        if (isWindows) {
            const fd = fs.openSync(outPath, 'r');
            const header = Buffer.alloc(2);
            fs.readSync(fd, header, 0, 2, 0);
            fs.closeSync(fd);
            if (header.toString('ascii') !== 'MZ') {
                throw new Error(`Бинарник ${outName} не имеет корректного PE-заголовка MZ`);
            }
        }
        console.log(`✓ Проверен sidecar: ${outName} (${(size / (1024 * 1024)).toFixed(1)} MB)`);
    }

    const baseModelPath = path.join(modelsDir, 'ggml-base.bin');
    if (!fs.existsSync(baseModelPath) || fs.statSync(baseModelPath).size < 100 * 1024 * 1024) {
        throw new Error('Отсутствует или повреждена базовая модель src-tauri/models/ggml-base.bin');
    }
    console.log(`✓ Проверена базовая модель: ggml-base.bin (${(fs.statSync(baseModelPath).size / (1024 * 1024)).toFixed(1)} MB)`);
}

// Рекурсивное копирование всех DLL библиотек из архива
function copyAllDlls(fromDir, toDir) {
    if (!fs.existsSync(fromDir)) return;
    const entries = fs.readdirSync(fromDir, { withFileTypes: true });
    for (const entry of entries) {
        const fullPath = path.join(fromDir, entry.name);
        if (entry.isDirectory()) {
            copyAllDlls(fullPath, toDir);
        } else if (entry.name.toLowerCase().endsWith('.dll')) {
            const destDll = path.join(toDir, entry.name);
            fs.copyFileSync(fullPath, destDll);
            console.log(`✓ Скопирована зависимость DLL: ${entry.name} (${(fs.statSync(destDll).size / (1024 * 1024)).toFixed(1)} MB)`);
        }
    }
}

async function run() {
    await prepareModelsDirectory();
    await downloadSidecars();
    await prewarmWindowsBundlerTools();
    verifyDesktopResources();

    try {
        fs.rmSync(tempDir, { recursive: true, force: true });
    } catch (err) {
        console.warn('Предупреждение при очистке временной папки:', err.message);
    }

    console.log('\n======================================================');
    console.log('✅ ВСЕ БИНАРНИКИ И МОДЕЛИ УСПЕШНО ПОДГОТОВЛЕНЫ И ПРОВЕРЕНЫ!');
    console.log('======================================================\n');
}

run().catch(err => {
    console.error('Ошибка выполнения скрипта:', err);
    process.exit(1);
});
