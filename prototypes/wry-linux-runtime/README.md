# Wry Linux runtime smoke

Изолированный исследовательский прототип для Ubuntu 24.04 под Xvfb. Он создаёт окно Tao и WebKitGTK через `build_gtk`, загружает встроенный HTML и ждёт, пока JavaScript изменит заголовок страницы. Только точный заголовок `Search Linux smoke ready` завершает процесс с кодом 0; внутренний таймаут 20 секунд и преждевременное закрытие окна дают код 1. Внешняя сеть не используется.

Сборка и запуск в CI закреплены в `.github/workflows/project-checks.yml`. Локально на Linux нужны `libwebkit2gtk-4.1-dev`, `xvfb` и `xauth`; затем `cargo build --locked --manifest-path prototypes/wry-linux-runtime/Cargo.toml` и `xvfb-run -a prototypes/wry-linux-runtime/target/debug/search-wry-linux-runtime`.

В тесте нет `with_ipc_handler`: веб-странице не выдаётся мост к внутренним командам. Обработчик разрешений Wry настроен на отказ. Этот сценарий не доказывает полноту его покрытия, изоляцию профиля, пиксельную отрисовку, работу клавиатуры, загрузки, Wayland или запуск на пользовательском Linux. [Официальный пример Wry](https://docs.rs/crate/wry/0.57.1/source/examples/simple.rs), [требования Linux](https://docs.rs/crate/wry/0.57.1/source/README.md).
