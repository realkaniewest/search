# Первая платформенная матрица

Статус: план для прототипа Wry, 6 ноября 2026 года. Минимальные версии ОС Search пока не утверждены. Выводы о сборке не заменяют проверку поведения браузера.

| Платформа | Движок и инструменты | Что проверить на прототипе | Текущее состояние |
| --- | --- | --- | --- |
| macOS, Apple Silicon | Wry использует WKWebView. Для разработки установлены Xcode Command Line Tools и Rust. [Wry](https://github.com/tauri-apps/wry/blob/dev/README.md), [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) | Навигация, разрешения, загрузки, профили, упаковка; отдельно проверить целевые версии macOS. | [Узкий Wry smoke-прототип](experiments/2026-11-06-wry-macos-smoke.md) собран и вручную запущен: окно и переход на сайт Rust работают. Остальные сценарии не проверены. |
| Windows | Wry использует WebView2. Нужны MSVC C++ Build Tools и WebView2 Runtime. Runtime нужно проверять или доставлять при распространении. [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), [Microsoft](https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution) | Наличие Runtime, защита SmartScreen, профили, разрешения и возможность поддержки Windows 10/11. | ПК с Windows есть; доступ и характеристики ещё не проверены. |
| Linux | Wry использует GTK и WebKitGTK. Для Debian/Ubuntu нужны пакеты семейства `libwebkit2gtk-4.1-dev`; набор зависит от дистрибутива. [Wry](https://github.com/tauri-apps/wry/blob/dev/README.md) | Сборка, X11/Wayland, работа цикла GTK, мультимедиа, разрешения, профили и пакетирование. | Linux VM на Mac запланирована при необходимости, но ещё не создана. |

У Wry обычный путь `build(&window)` ориентирован на X11; для Wayland документация указывает GTK-путь `build_gtk`. Это отдельная проверка, а не обещание одинаковой поддержки. [Документация Wry](https://docs.rs/wry/latest/wry/struct.WebViewBuilder.html).

[GitHub hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) доступны для macOS, Windows и Linux. Для узкого Wry-прототипа уже работает macOS-сборка в CI; Windows и Linux добавим после подготовки прототипов для этих ОС. Ручные UI-сценарии и измерения производительности останутся отдельными проверками на известных устройствах.

## Что нужно зафиксировать перед ADR

- Какие версии macOS, Windows и дистрибутивы Linux поддерживаем сначала.
- Есть ли доступ к Windows и Linux для ручного тестирования, включая Wayland.
- Сборка и запуск Wry на каждой ОС; конкретные версии WebView2/WebKitGTK.
- Результаты сценариев из `docs/prototype-plan.md` с указанием «прошёл», «не прошёл», «не проверен».
