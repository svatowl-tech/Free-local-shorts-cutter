# 🎬 Video Cutter Pro — Умный ИИ-редактор видео

Кроссплатформенное десктопное приложение для интеллектуальной нарезки, монтажа и анализа видеоматериалов с использованием локальных нейросетей (Whisper, LLaMA-Vision) и высокопроизводительного мультимедийного движка FFmpeg.

[![Release](https://img.shields.io/github/v/release/OWNER/REPO?label=Версия&color=blue)](https://github.com/OWNER/REPO/releases)
[![Build Status](https://img.shields.io/github/actions/workflow/status/OWNER/REPO/release.yml?branch=main&label=Сборка%20всех%20платформ)](https://github.com/OWNER/REPO/actions)
[![Status](https://img.shields.io/badge/Статус-В%20активной%20разработке%20(WIP)-orange.svg)](https://github.com/OWNER/REPO)
[![License](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

> [!WARNING]
> **Внимание: Проект находится в стадии активной разработки (WIP / Beta)!**
> 
> Приложение активно дорабатывается и тестируется. Вы можете столкнуться с ошибками, недоработками, временной нестабильностью некоторых функций или изменением интерфейса.
> Если вы обнаружили баг или у вас есть предложение по улучшению — пожалуйста, создайте [Issue](https://github.com/OWNER/REPO/issues) в репозитории. Обратная связь очень важна!

---

## ✨ Ключевые возможности

- ⚡ **Ультрабыстрый экспорт без перекодирования**: Резка по ключевым кадрам (Stream Copy) со скоростью чтения накопителя.
- 🎯 **Высокоточная покадровая нарезка**: Покадровый монтаж с повторным кодированием только стыковочных участков.
- 🎙️ **Локальное распознавание речи (Whisper.cpp)**: Автоматическая транскрибация, субтитры и нарезка по паузам/фразам без интернета.
- 👁️ **Анализ сцен и компьютерное зрение (LLaMA-Vision / LLaVA)**: Поиск объектов, лиц и динамических сцен.
- 📦 **Два формата распространения**: Классический инсталлятор для системы и 100% автономная **Portable** версия.
- 🌐 **Кроссплатформенность**: Нативная поддержка **Windows**, **Linux** и **macOS** (Apple Silicon + Intel).

---

## 📥 Скачать (Releases)

Все готовые сборки доступны на странице **[GitHub Releases](https://github.com/OWNER/REPO/releases)**.

| Операционная система | Установочная версия (Installer) | Портативная версия (Portable) | Описание |
| :--- | :--- | :--- | :--- |
| **Windows 10 / 11 (x64)** | `.exe` (NSIS Setup)<br>`.msi` | `.zip` | В Portable-версии все sidecar-бинарники и настройки лежат рядом в папке, не затрагивая системный реестр. |
| **Linux (x64)** | `.deb` (Ubuntu, Debian, Mint) | `.AppImage`<br>`.tar.gz` | AppImage запускается сразу в один клик без установки зависимостей в систему. |
| **macOS (Apple Silicon M1/M2/M3)** | `.dmg` | `.zip` (.app бандл) | Оптимизировано под архитектуру ARM64. |
| **macOS (Intel x64)** | `.dmg` | `.zip` (.app бандл) | Для компьютеров Mac на базе процессоров Intel. |

---

## 🚀 Автоматический релиз и версионирование (CI/CD)

В репозиторий встроен полностью автоматизированный цикл сборки и публикаций через **GitHub Actions**:

### Варианты запуска нового релиза:

1. **Через веб-интерфейс GitHub (Ручной запуск с авто-инкрементом)**:
   - Перейдите во вкладку **Actions** в репозитории на GitHub.
   - Выберите пайплайн **"Automated Release & Multi-Platform Build"**.
   - Нажмите **"Run workflow"**.
   - Выберите тип обновления версии:
     - `patch` (например `2.0.0` -> `2.0.1`) — по умолчанию
     - `minor` (например `2.0.0` -> `2.1.0`)
     - `major` (например `2.0.0` -> `3.0.0`)
     - `custom` (ввести произвольную версию, например `2.5.0`)
     - `none` (собрать релиз с текущей версией)
   - Действие автоматически обновит `package.json`, `tauri.conf.json` и `Cargo.toml`, создаст Git-тег, запустит параллельную сборку на 4 раннерах (Windows, Linux, macOS ARM64, macOS Intel), упакует инсталляторы и Portable архивы, и прикрепит их к GitHub Release.

2. **Через Git-тег**:
   ```bash
   git tag v2.0.1
   git push origin v2.0.1
   ```
   Пайплайн автоматически подхватит тег и соберёт дистрибутивы под эту версию.

3. **Через коммит в ветку `main`**:
   Если сообщение коммита содержит `[release]` (например, `git commit -m "feat: added new cuts [release]"`), запустится автоматический patch-релиз.

---

## 🛠️ Локальная разработка и сборка

### Требования к окружению:
- **Node.js**: 18+ (рекомендуется 20 LTS)
- **Rust**: актуальная стабильная версия (`rustup update stable`)
- Системные пакеты (для Linux): `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `libasound2-dev`

### Команды проекта:

```bash
# Установка зависимостей
npm install

# Запуск в режиме веб-разработки (интерфейс в браузере)
npm run dev

# Запуск десктопного приложения в режиме разработки Tauri
npm run tauri:dev

# Генерация 100% комплекта иконок для всех платформ (Windows ICO/MSI, macOS ICNS, Linux, Android, iOS):
npm run generate:icons

# Полная сборка всех платформ и генерация portable версий локально
npm run build:all

# Только генерация portable и сбор инсталляторов в папку dist-releases:
npm run package:portable

# Инкремент версии вручную (patch / minor / major)
npm run version:bump patch
```

---

## 📁 Структура проекта

```
├── .github/workflows/       # CI/CD: автоматическая сборка и релизы на GitHub
│   ├── release.yml          # Матричный релиз всех платформ + Portable + авто-теги
│   └── ci.yml               # Быстрая проверка PR и коммитов
├── scripts/
│   ├── bump-version.js      # Скрипт синхронизации семантической версии
│   ├── download-resources.cjs # Загрузчик sidecar-бинарников (FFmpeg, Whisper, LLaMA)
│   ├── generate-icons.js    # Генератор полного набора иконок (ICO, ICNS, PNG, Mipmaps)
│   └── package-portable.js  # Сборщик и упаковщик Portable версий и инсталляторов
├── src/                     # SvelteKit интерфейс приложения (UI)
├── src-tauri/               # Бэкенд Tauri 2 (Rust)
│   ├── Cargo.toml           # Манифест зависимостей Rust
│   ├── tauri.conf.json      # Конфигурация приложения и бандлеров Tauri 2
│   ├── build.rs             # Системный скрипт сборки Rust
│   ├── resources/           # Папка для sidecar исполняемых файлов
│   └── models/              # Папка для локальных GGUF-моделей ИИ
└── dist-releases/           # Готовые собранные установщики и ZIP/tar.gz архивы
```

---

## 📄 Лицензия

Распространяется под лицензией MIT.
