# Проверка origin разрешения в Wry на Linux

Исследовательский прототип для Ubuntu 24.04, Wry 0.57.1 и WebKitGTK 4.1. Он открывает верхнюю страницу, same-origin iframe, cross-origin iframe с `allow` и такой же iframe без `allow` на двух локальных портах `127.0.0.1`. Каждая страница дважды делает `navigator.permissions.query({name: 'camera'})` и дважды `getUserMedia({video: true})`, сообщая свой `location.origin`. Перед загрузкой страницы включаются штатные синтетические камера и микрофон WebKitGTK. Обработчик Wry отвечает `Deny` на все дошедшие до него запросы. Приложение не сохраняет страницу или личные данные и останавливает поток, если движок неожиданно его выдаст. Запускать только в CI без подключённых устройств.

Rust-обёртка WebKitGTK, используемая Wry 0.57.1, не предоставляет `connect_query_permission_state`: тип аргумента сигнала в ней помечен как неподдерживаемый. Поэтому только этот опыт подключает сигнал через C ABI. Это не код браузера Search и не выбранное решение для разрешений.

На Ubuntu с `libwebkit2gtk-4.1-dev`, `xvfb`, `xauth` и Rust 1.99.1:

```sh
cargo +1.99.1 build --locked --manifest-path prototypes/wry-linux-permission-origin/Cargo.toml
timeout 45s xvfb-run -a --server-args='-screen 0 1280x800x24' prototypes/wry-linux-permission-origin/target/debug/search-wry-linux-permission-origin --request-camera
```

Код 0 означает только, что все 16 отчётов страниц получены, хотя бы один обработчик Wry был вызван, поток не был выдан и диагностические строки записаны. Код 1 означает неполный опыт, отсутствие вызова обработчика или недоступный сигнал. Код 2 означает, что поток неожиданно был выдан. Отчёты привязаны к случайному маркеру запуска, точному локальному Host и, если присутствует, Origin. Тайм-аут ограничен 25 секундами внутри приложения и 45 секундами снаружи. `QUERY seq` и `WRY_PERMISSION_REQUEST seq` дают порядок нативных событий; записи ограничены первыми 64 событиями каждого типа. `DIAGNOSTIC` выводит число вызовов Wry и наличие обоих origin в нативных query-сигналах, но не превращает их в архитектурный pass/fail.

Даже код 0 **не доказывает**, что нативный сигнал с origin вызывается при каждом `getUserMedia()` из iframe и что его можно безопасно связать с обработчиком Wry. Результаты `navigator.permissions.query()` и callback Wry записаны отдельно. Совпадение их числа или времени само по себе не является доказательством связи. Для отрицательного iframe без `allow` ожидается отказ до доступа к камере, но прототип пока не связывает отдельный callback с конкретным iframe. Перед решением об архитектуре нужны проверка сохранения и отзыва разрешений, несколько профилей, геолокация и другие ОС.

Источники: [Wry 0.57.1 Linux handler](https://github.com/tauri-apps/wry/blob/wry-v0.57.1/src/webkitgtk/mod.rs), [обёртка WebKitGTK](https://github.com/tauri-apps/webkit2gtk-rs/blob/crate/src/auto/web_view.rs), [WebKitGTK query-permission-state](https://webkitgtk.org/reference/webkit2gtk/2.40.2/struct.PermissionStateQuery.html), [WebKitGTK mock capture devices](https://webkitgtk.org/reference/webkit2gtk/2.38.5/property.Settings.enable-mock-capture-devices.html), [Permissions Policy](https://www.w3.org/TR/permissions-policy/).
