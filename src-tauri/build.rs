// src-tauri/build.rs
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let resources_dir = Path::new("resources");
    let models_dir = Path::new("models");
    let temp_download_dir = Path::new("target/downloads");

    // Создаем нужные папки
    if let Err(e) = fs::create_dir_all(resources_dir) {
        panic!("Не удалось создать папку resources: {}", e);
    }
    if let Err(e) = fs::create_dir_all(models_dir) {
        panic!("Не удалось создать папку models: {}", e);
    }
    if let Err(e) = fs::create_dir_all(temp_download_dir) {
        panic!("Не удалось создать папку target/downloads: {}", e);
    }

    // Создаем плейсхолдер в models/ и resources/, чтобы glob в tauri.conf.json никогда не падал
    let placeholder_path = models_dir.join("placeholder.txt");
    if !placeholder_path.exists() {
        let _ = fs::write(placeholder_path, "Local AI models go here.");
    }

    let dll_placeholder_path = resources_dir.join("placeholder.dll");
    if !dll_placeholder_path.exists() {
        let _ = fs::write(dll_placeholder_path, "Placeholder for DLL files.");
    }

    // Определяем текущий target triple (например, x86_64-pc-windows-msvc)
    let target = std::env::var("TARGET").unwrap_or_else(|_| "x86_64-pc-windows-msvc".to_string());
    println!("Сборка под целевую платформу: {}", target);

    // Очищаем устаревшую переменную окружения STATIC_VCRUNTIME (которую выставляет @tauri-apps/cli ~2.0.0),
    // чтобы tauri-build не генерировал предупреждение об устаревании STATIC_VCRUNTIME
    std::env::remove_var("STATIC_VCRUNTIME");

    let is_windows = target.contains("windows");
    let is_mac = target.contains("apple-darwin");
    let is_mobile = target.contains("android") || target.contains("ios");
    let ext = if is_windows { ".exe" } else { "" };

    let sidecar_names = ["ffmpeg", "ffprobe", "whisper", "llama-vision", "llama-cli"];

    // Для мобильных таргетов (Android / iOS) создаем легкие заглушки sidecar-файлов без скачивания десктопных архивов
    if is_mobile {
        println!(
            "Мобильная целевая платформа ({}): создание кроссплатформенных stub-ресурсов без десктопных бинарников.",
            target
        );
        for sidecar_name in &sidecar_names {
            let sidecar_filename = format!("{}-{}{}", sidecar_name, target, ext);
            let sidecar_fullname = resources_dir.join(&sidecar_filename);
            if !sidecar_fullname.exists() {
                let _ = fs::write(
                    &sidecar_fullname,
                    format!("# Mobile sidecar stub for {}\n", sidecar_name),
                );
            }
        }
        tauri_build::build();
        return;
    }

    // Ссылки на архивы, синхронизированные с scripts/download-resources.cjs
    let (ffmpeg_url, ffprobe_url, whisper_url, llama_url) = if is_windows {
        let ff = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";
        (
            ff,
            ff,
            "https://github.com/ggerganov/whisper.cpp/releases/download/v1.5.4/whisper-cublas-12.2.0-bin-x64.zip",
            "https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-win-cuda-cu12.2.0-x64.zip",
        )
    } else if is_mac {
        let is_arm64 = target.contains("aarch64");
        (
            "https://evermeet.cx/ffmpeg/getrelease/zip",
            "https://evermeet.cx/ffmpeg/getrelease/ffprobe/zip",
            if is_arm64 {
                "https://github.com/ggerganov/whisper.cpp/releases/download/v1.5.4/whisper-bin-macos-arm64.zip"
            } else {
                "https://github.com/ggerganov/whisper.cpp/releases/download/v1.5.4/whisper-bin-macos-x64.zip"
            },
            if is_arm64 {
                "https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-macos-arm64.zip"
            } else {
                "https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-macos-x64.zip"
            },
        )
    } else {
        let ff = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz";
        (
            ff,
            ff,
            "https://github.com/ggerganov/whisper.cpp/releases/download/v1.5.4/whisper-bin-x64.zip",
            "https://github.com/ggerganov/llama.cpp/releases/download/b3201/llama-b3201-bin-ubuntu-x64.zip",
        )
    };

    // Опишем структуру: (имя sidecar, имя файла внутри архива, URL архива, имя архивного файла)
    let sidecars = vec![
        (
            "ffmpeg",
            if is_windows { "ffmpeg.exe" } else { "ffmpeg" },
            ffmpeg_url,
            if is_windows || is_mac { "ffmpeg.zip" } else { "ffmpeg.tar.xz" },
        ),
        (
            "ffprobe",
            if is_windows { "ffprobe.exe" } else { "ffprobe" },
            ffprobe_url,
            if is_windows {
                "ffmpeg.zip"
            } else if is_mac {
                "ffprobe.zip"
            } else {
                "ffmpeg.tar.xz"
            },
        ),
        (
            "whisper",
            if is_windows { "main.exe" } else { "main" },
            whisper_url,
            "whisper.zip",
        ),
        (
            "llama-vision",
            if is_windows { "llama-llava-cli.exe" } else { "llama-llava-cli" },
            llama_url,
            "llama.zip",
        ),
        (
            "llama-cli",
            if is_windows { "llama-cli.exe" } else { "llama-cli" },
            llama_url,
            "llama.zip",
        ),
    ];

    for (sidecar_name, target_bin_file, archive_url, archive_file_name) in sidecars {
        let sidecar_filename = format!("{}-{}{}", sidecar_name, target, ext);
        let sidecar_fullname = resources_dir.join(&sidecar_filename);

        if sidecar_fullname.exists() {
            println!("Бинарник {} уже скачан.", sidecar_filename);
            continue;
        }

        println!("Запуск подготовки {}...", sidecar_filename);

        // Путь скачанного архива
        let archive_path = temp_download_dir.join(archive_file_name);

        // Скачиваем архив, если еще нет
        if !archive_path.exists() {
            if let Err(e) = download_file(archive_url, &archive_path) {
                panic!("Критическая ошибка загрузки {}: {}", archive_url, e);
            }
        }

        // Временная папка распаковки
        let extract_dir = temp_download_dir.join(format!("extract_{}", sidecar_name));
        let _ = fs::remove_dir_all(&extract_dir); // Очищаем старые следы

        if let Err(e) = extract_archive(&archive_path, &extract_dir) {
            panic!("Критическая ошибка распаковки {:?}: {}", archive_path, e);
        }

        // Ищем целевой бинарник рекурсивно в распакованном каталоге
        if let Some(src_bin_path) = find_file_recursive(&extract_dir, target_bin_file) {
            // Копируем бинарник в финальный resources sidecar
            if let Err(e) = fs::copy(&src_bin_path, &sidecar_fullname) {
                panic!(
                    "Не удалось скопировать бинарник {:?} -> {:?}: {}",
                    src_bin_path, sidecar_fullname, e
                );
            }
            println!(
                "Успешно извлечён и подготовлен sidecar {}",
                sidecar_filename
            );
        } else {
            // Если точное имя не найдено, поищем любое частичное совпадение
            let fallback_masks: &[&str] = match sidecar_name {
                "llama-vision" => &["llava-cli", "minicpm", "minicopic", "llama-cli", "main"],
                "whisper" => &["whisper-cli", "whisper", "main"],
                "llama-cli" => &["llama-cli", "main"],
                _ => &[target_bin_file],
            };

            let mut found = false;
            for mask in fallback_masks {
                if let Some(src_bin_path) = find_file_partial(&extract_dir, mask) {
                    if let Err(e) = fs::copy(&src_bin_path, &sidecar_fullname) {
                        panic!(
                            "Не удалось скопировать fallback-бинарник {:?} -> {:?}: {}",
                            src_bin_path, sidecar_fullname, e
                        );
                    }
                    println!(
                        "Успешно подготовлен fallback-sidecar {} из {:?}",
                        sidecar_filename, src_bin_path
                    );
                    found = true;
                    break;
                }
            }

            if !found {
                panic!(
                    "Критическая ошибка: бинарный файл '{}' не найден в распакованном архиве {:?}",
                    target_bin_file, extract_dir
                );
            }
        }

        // Копируем все зависимые DLL (whisper.dll, ggml.dll, cublas, cudart, SDL2 и т.д.)
        copy_all_dlls(&extract_dir, resources_dir);

        // Очищаем временную папку распаковки ради экономии места
        let _ = fs::remove_dir_all(&extract_dir);
    }

    // Вызываем стандартный Tauri сборщик
    tauri_build::build();
}

fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    println!("Скачивание архива: {} -> {:?}", url, dest);

    // Сначала пробуем использовать системный curl с передачей OsStr без unwrap()
    let output = Command::new("curl")
        .arg("-L")
        .arg("-f")
        .arg("-S")
        .arg("-o")
        .arg(dest.as_os_str())
        .arg(url)
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            return Ok(());
        }
    }

    // Для Windows запускаем PowerShell в качестве резервного инструмента
    if cfg!(target_os = "windows") {
        let dest_lossy = dest.to_string_lossy();
        let ps_cmd = format!(
            "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; Invoke-WebRequest -Uri '{}' -OutFile '{}'",
            url, dest_lossy
        );
        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps_cmd])
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    return Ok(());
                } else {
                    return Err(format!(
                        "PowerShell failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    ));
                }
            }
            Err(e) => return Err(format!("Failed to execute PowerShell: {}", e)),
        }
    }

    Err("Все доступные методы загрузки (curl/powershell) исчерпаны или завершились с ошибкой.".to_string())
}

fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<(), String> {
    println!(
        "Распаковка архива: {:?} -> {:?}",
        archive_path, dest_dir
    );
    fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;

    let is_zip = archive_path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("zip"))
        .unwrap_or(false);

    if is_zip {
        // На Windows пробуем PowerShell Expand-Archive
        if cfg!(target_os = "windows") {
            let ps_cmd = format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                archive_path.to_string_lossy(),
                dest_dir.to_string_lossy()
            );
            let output = Command::new("powershell")
                .args(["-NoProfile", "-Command", &ps_cmd])
                .output();

            if let Ok(out) = output {
                if out.status.success() {
                    return Ok(());
                }
            }
        }

        // Пробуем универсальный tar -xf
        let output = Command::new("tar")
            .arg("-xf")
            .arg(archive_path.as_os_str())
            .arg("-C")
            .arg(dest_dir.as_os_str())
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                return Ok(());
            }
        }

        // Пробуем unzip для zip под Linux/macOS/Unix
        let output = Command::new("unzip")
            .arg("-q")
            .arg("-o")
            .arg(archive_path.as_os_str())
            .arg("-d")
            .arg(dest_dir.as_os_str())
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                return Ok(());
            }
        }
    } else {
        // Для .tar.gz или .tar.xz
        let output = Command::new("tar")
            .arg("-xf")
            .arg(archive_path.as_os_str())
            .arg("-C")
            .arg(dest_dir.as_os_str())
            .output();

        match output {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => {
                return Err(format!(
                    "tar failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                ))
            }
            Err(e) => return Err(format!("failed to run tar command: {}", e)),
        }
    }

    Err("Не удалось распаковать архив ни одним из поддерживаемых способов.".to_string())
}

fn find_file_recursive(dir: &Path, target_name: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = find_file_recursive(&path, target_name) {
                    return Some(found);
                }
            } else if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                if file_name.to_lowercase() == target_name.to_lowercase() {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn find_file_partial(dir: &Path, mask: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = find_file_partial(&path, mask) {
                    return Some(found);
                }
            } else if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                let fname = file_name.to_lowercase();
                if fname.contains(&mask.to_lowercase()) && (fname.ends_with(".exe") || !mut_is_windows_target()) {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn mut_is_windows_target() -> bool {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "x86_64-pc-windows-msvc".to_string());
    target.contains("windows")
}

fn copy_all_dlls(from_dir: &Path, to_dir: &Path) {
    if let Ok(entries) = fs::read_dir(from_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                copy_all_dlls(&path, to_dir);
            } else if path.extension().and_then(|s| s.to_str()).map(|ext| ext.eq_ignore_ascii_case("dll")).unwrap_or(false) {
                if let Some(file_name) = path.file_name() {
                    let dest = to_dir.join(file_name);
                    let _ = fs::copy(&path, &dest);
                    println!("Скопирована зависимая библиотека DLL: {:?}", file_name);
                }
            }
        }
    }
}
