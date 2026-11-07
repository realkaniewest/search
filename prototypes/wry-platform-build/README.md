# Wry platform build probe

Отдельный исследовательский прототип для проверки компиляции и линковки Wry 0.57.1 на Windows и Linux. Он создаёт только Rust builder и не открывает окно или WebView. Результат сборки не доказывает работу сайтов, разрешений, профилей, загрузок или SmartScreen в запущенном приложении.

Windows: исходный код Wry 0.57.1 по умолчанию задаёт WebView2 аргумент `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`. Прототип явно вызывает `with_additional_browser_args("")`, который заменяет этот аргумент пустой строкой. Поскольку `build` WebView не вызывается, Windows runtime и фактическое состояние SmartScreen этим тестом не проверяются. В будущем UI-прототипе эту настройку нужно сохранить и проверить отдельно. [Исходный код Wry](https://docs.rs/crate/wry/0.57.1/source/src/webview2/mod.rs), [Windows API](https://docs.rs/wry/0.57.1/x86_64-pc-windows-msvc/wry/trait.WebViewBuilderExtWindows.html).

Linux: CI устанавливает `libwebkit2gtk-4.1-dev` по [README Wry](https://docs.rs/crate/wry/0.57.1/source/README.md), затем собирает бинарник без запуска. Графическая сессия, GTK event loop, X11 и Wayland не проверяются.

Локально для своей ОС: `cargo build --locked --manifest-path prototypes/wry-platform-build/Cargo.toml`. Версия Rust для CI закреплена в workflow.
