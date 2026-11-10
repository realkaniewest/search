# Wry Linux runtime smoke

Изолированный исследовательский прототип для Ubuntu 24.04 под Xvfb. Он создаёт окно Tao и WebKitGTK через `build_gtk`, загружает встроенный HTML и ждёт, пока JavaScript изменит заголовок страницы. Только точный заголовок `Search Linux smoke ready` завершает процесс с кодом 0; внутренний таймаут 20 секунд и преждевременное закрытие окна дают код 1. Внешняя сеть не используется.

Сборка и запуск в CI закреплены в `.github/workflows/project-checks.yml`. Локально на Linux нужны `libwebkit2gtk-4.1-dev`, `xvfb`, `xauth`, `xdotool` и `imagemagick`; затем `cargo build --locked --manifest-path prototypes/wry-linux-runtime/Cargo.toml` и `xvfb-run -a prototypes/wry-linux-runtime/target/debug/search-wry-linux-runtime`.

Дополнительный opt-in сценарий запускается через `xvfb-run -a bash prototypes/wry-linux-runtime/run_visual_input.sh "$PWD/prototypes/wry-linux-runtime/target/debug/search-wry-linux-runtime"`. Он сохраняет окно открытым, снимает PNG через ImageMagick, проверяет большую одноцветную область и отправляет клик с набором текста через X11. Это проверяет наличие пикселей страницы и получение ввода в Xvfb, но не качество интерфейса на физическом дисплее.

В тесте нет `with_ipc_handler`: веб-странице не выдаётся мост к внутренним командам. Обработчик разрешений Wry настроен на отказ. Эти сценарии не доказывают полноту его покрытия, изоляцию профиля, доступность, качество отрисовки и ввода на реальном дисплее, загрузки, Wayland или запуск на пользовательском Linux. [Официальный пример Wry](https://docs.rs/crate/wry/0.57.1/source/examples/simple.rs), [требования Linux](https://docs.rs/crate/wry/0.57.1/source/README.md).
