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
    fs::create_dir_all(resources_dir).unwrap();
    fs::create_dir_all(models_dir).unwrap();
    fs::create_dir_all(temp_download_dir).unwrap();

    // Создаем плейсхолдер в models/, чтобы glob models/* в tauri.conf.json никогда не падал в пустой сборке
    let placeholder_path = models_dir.join("placeholder.txt");
    if !placeholder_path.exists() {
        let _ = fs::write(placeholder_path, "Local AI models go here.");
    }

    // Определяем текущий target triple (например, x86_64-pc-windows-msvc)
    let target = std::env::var("TARGET").unwrap_or_else(|_| "x86_64-pc-windows-msvc".to_string());
    println!("cargo:warning=Сборка под целевую платформу: {}", target);

    // Доступные расширения бинарников
    let ext = if target.contains("windows") { ".exe" } else { "" };

    // JSON-подобный список ресурсов с правильными URL-адресами в зависимости от платформы
    let is_windows = target.contains("windows");
    
    // Ссылки на архивы
    let (ffmpeg_url, whisper_url, llama_url) = if is_windows {
        (
            "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip",
            "https://github.com/ggml-org/whisper.cpp/releases/download/v1.5.4/whisper-cublas-12.2.0-bin-x64.zip",
            "https://github.com/ggml-org/llama.cpp/releases/download/b3500/llama-b3500-bin-win-cuda-cu12.2.0-x64.zip"
        )
    } else {
        // Fallback-ссылки под Linux для контейнера или кросс-компиляции
        (
            "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz",
            "https://github.com/ggml-org/whisper.cpp/releases/download/v1.5.4/whisper-v1.5.4-bin-ubuntu-x64.zip",
            "https://github.com/ggml-org/llama.cpp/releases/download/b3500/llama-b3500-bin-ubuntu-x64.zip"
        )
    };

    // Опишем структуру: (имя sidecar, имя файла внутри архива, URL архива, имя архивного файла)
    // sidecar имя будет <col1>-<target><ext>
    let sidecars = vec![
        (
            "ffmpeg", 
            if is_windows { "ffmpeg.exe" } else { "ffmpeg" }, 
            ffmpeg_url, 
            if is_windows { "ffmpeg.zip" } else { "ffmpeg.tar.xz" }
        ),
        (
            "ffprobe", 
            if is_windows { "ffprobe.exe" } else { "ffprobe" }, 
            ffmpeg_url, 
            if is_windows { "ffmpeg.zip" } else { "ffmpeg.tar.xz" }
        ),
        (
            "whisper", 
            if is_windows { "main.exe" } else { "main" }, 
            whisper_url, 
            "whisper.zip"
        ),
        (
            "llama-vision", 
            if is_windows { "llama-llava-cli.exe" } else { "llama-llava-cli" }, 
            llama_url, 
            "llama.zip"
        ),
        (
            "llama-cli", 
            if is_windows { "llama-cli.exe" } else { "llama-cli" }, 
            llama_url, 
            "llama.zip"
        ),
    ];

    for (sidecar_name, target_bin_file, archive_url, archive_file_name) in sidecars {
        let sidecar_filename = format!("{}-{}{}", sidecar_name, target, ext);
        let sidecar_fullname = resources_dir.join(&sidecar_filename);

        if sidecar_fullname.exists() {
            println!("cargo:warning=Бинарник {} уже скачан.", sidecar_filename);
            continue;
        }

        println!("cargo:warning=Запуск подготовки {}...", sidecar_filename);

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
                panic!("Не удалось скопировать бинарник {:?} -> {:?}: {}", src_bin_path, sidecar_fullname, e);
            }
            println!("cargo:warning=Успешно извлечён и подготовлен sidecar {}", sidecar_filename);
        } else {
            // Если точное имя не найдено, поищем любое частичное совпадение
            // Например для llama-vision вместо llama-llava-cli поищем любое *llava-cli* или *cli*
            let fallback_name = match sidecar_name {
                "llama-vision" => "llava-cli",
                "whisper" => "whisper-cli",
                _ => target_bin_file,
            };

            let mut found = false;
            if let Some(src_bin_path) = find_file_partial(&extract_dir, fallback_name) {
                if let Err(e) = fs::copy(&src_bin_path, &sidecar_fullname) {
                    panic!("Не удалось скопировать fallback-бинарник {:?} -> {:?}: {}", src_bin_path, sidecar_fullname, e);
                }
                println!("cargo:warning=Успешно подготовлен fallback-sidecar {} из {:?}", sidecar_filename, src_bin_path);
                found = true;
            }

            if !found {
                panic!(
                    "Критическая ошибка: бинарный файл '{}' (или маска '{}') не найден в распакованном архиве {:?}", 
                    target_bin_file, fallback_name, extract_dir
                );
            }
        }

        // Очищаем временную папку распаковки ради экономии места
        let _ = fs::remove_dir_all(&extract_dir);
    }

    // Вызываем стандартный Tauri сборщик
    tauri_build::build();
}

fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    println!("cargo:warning=Скачивание архива: {} -> {:?}", url, dest);
    
    // Сначала пробуем использовать системный curl
    let output = Command::new("curl")
        .args(&["-L", "-f", "-S", "-o", dest.to_str().unwrap(), url])
        .output();
    
    match output {
        Ok(out) if out.status.success() => return Ok(()),
        _ => {}
    }

    // Для Windows запускаем PowerShell в качестве мощного резервного инструмента
    if cfg!(target_os = "windows") {
        let ps_cmd = format!(
            "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; Invoke-WebRequest -Uri '{}' -OutFile '{}'",
            url,
            dest.to_str().unwrap()
        );
        let output = Command::new("powershell")
            .args(&["-NoProfile", "-Command", &ps_cmd])
            .output();
        
        match output {
            Ok(out) => {
                if out.status.success() {
                    return Ok(());
                } else {
                    return Err(format!("PowerShell failed: {}", String::from_utf8_lossy(&out.stderr)));
                }
            }
            Err(e) => return Err(format!("Failed to execute PowerShell: {}", e)),
        }
    }

    Err("Все доступные методы загрузки (curl/powershell) исчерпаны или завершились с ошибкой.".to_string())
}

fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<(), String> {
    println!("cargo:warning=Распаковка архива: {:?} -> {:?}", archive_path, dest_dir);
    fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;

    let is_zip = archive_path.extension().and_then(|s| s.to_str()) == Some("zip");

    if is_zip {
        // На Windows пробуем PowerShell Expand-Archive
        if cfg!(target_os = "windows") {
            let ps_cmd = format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                archive_path.to_str().unwrap(),
                dest_dir.to_str().unwrap()
            );
            let output = Command::new("powershell")
                .args(&["-NoProfile", "-Command", &ps_cmd])
                .output();

            if let Ok(out) = output {
                if out.status.success() {
                    return Ok(());
                }
            }
        }

        // Пробуем универсальный tar -xf
        let output = Command::new("tar")
            .args(&["-xf", archive_path.to_str().unwrap(), "-C", dest_dir.to_str().unwrap()])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                return Ok(());
            }
        }

        // Пробуем unzip для zip под Linux/Unix
        let output = Command::new("unzip")
            .args(&["-q", archive_path.to_str().unwrap(), "-d", dest_dir.to_str().unwrap()])
            .output();

        match output {
            Ok(out) if out.status.success() => return Ok(()),
            _ => {}
        }
    } else {
        // Для .tar.gz или .tar.xz
        let output = Command::new("tar")
            .args(&["-xf", archive_path.to_str().unwrap(), "-C", dest_dir.to_str().unwrap()])
            .output();

        match output {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => return Err(format!("tar failed: {}", String::from_utf8_lossy(&out.stderr))),
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
