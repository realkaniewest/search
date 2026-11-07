# Wry 0.57.1: сборка на Windows и Linux CI

Статус: прототип собран с линковкой на GitHub runner’ах Windows и Linux, но бинарник не запускался.

## Среда и действия

- 7 ноября 2026 года, Wry 0.57.1, Rust 1.99.1, закреплённый `Cargo.lock`. Исходники: [`prototypes/wry-platform-build`](../../prototypes/wry-platform-build/README.md).
- CI [PR #19](https://github.com/realkaniewest/search/pull/19): `windows-2025` и `ubuntu-24.04`; отдельная macOS-проверка проекта тоже прошла. Ubuntu установил `libwebkit2gtk-4.1-dev` по [README Wry](https://docs.rs/crate/wry/0.57.1/source/README.md).
- На обеих ОС выполнены `cargo +1.99.1 fmt --check`, `cargo +1.99.1 clippy --locked -- -D warnings` и `cargo +1.99.1 build --locked` для нового прототипа.

## Результат

| ОС | Результат CI | Что подтверждено |
| --- | --- | --- |
| Windows 2025 runner | Прошёл, 1 мин 29 с | Закреплённый пример с Windows API Wry скомпилирован и слинкован. |
| Ubuntu 24.04 runner | Прошёл, 2 мин | Закреплённый пример с Linux-зависимостями скомпилирован и слинкован. |
| macOS 27.0.1, Apple Silicon, локально | Сборка прошла | Тот же пример собран на Mac. |
| macos-26 runner | Проектный CI прошёл, 2 мин 54 с | Существующие macOS Wry-прототипы прошли проверки CI; новый пример в этом job не собирался. |

В Windows-ветке пример явно вызывает `with_additional_browser_args("")`. По [исходному коду Wry 0.57.1](https://docs.rs/crate/wry/0.57.1/source/src/webview2/mod.rs) это заменяет аргумент по умолчанию, который содержит `msSmartScreenProtection` в списке отключаемых функций. Проверена компиляция такого вызова. WebView2 environment не создавался, поэтому работа SmartScreen при запуске **не проверена**. Пустая строка также заменяет другие аргументы Wry по умолчанию; в этом примере autoplay и proxy не заданы.

## Границы

Бинарник не запускался на Windows и Linux: ни окно, ни сайт, ни GTK event loop, ни X11/Wayland, ни WebView2 runtime, ни системные разрешения и профили не проверены. Время CI включает подготовку runner’а и зависимости; это не измерение скорости браузера. Для закрытия пункта roadmap о тестовых машинах нужны реальные UI-прогоны на обеих ОС. Выбор архитектуры не сделан.
