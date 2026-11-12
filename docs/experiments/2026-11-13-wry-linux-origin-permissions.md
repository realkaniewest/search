# Wry: origin запроса разрешения на Linux

Статус: диагностический прототип подготовлен 13 ноября 2026 года; запуск Linux CI ещё не проверен. Задача [#41](https://github.com/realkaniewest/search/issues/41). Эта запись не подтверждает работу управления разрешениями в браузере Search.

## Проверенные сведения API

- В Wry 0.57.1 обработчик `permission-request` на Linux передаёт общему API вид разрешения без origin запрашивающего frame. [Исходный код Wry](https://github.com/tauri-apps/wry/blob/wry-v0.57.1/src/webkitgtk/mod.rs).
- `WebKitUserMediaPermissionRequest` сообщает, запрошены ли аудио и видео, но его публичный API не содержит origin. [WebKitGTK](https://webkitgtk.org/reference/webkit2gtk/2.41.3/class.UserMediaPermissionRequest.html).
- Отдельный `query-permission-state` передаёт `WebKitPermissionStateQuery` с `security_origin`. Документация не обещает, что этот сигнал приходит для каждого `getUserMedia()` или что он позволяет однозначно связать origin с последующим `permission-request`. [Сигнал](https://webkitgtk.org/reference/webkit2gtk/2.41.3/signal.WebView.query-permission-state.html), [тип запроса](https://webkitgtk.org/reference/webkit2gtk/2.40.2/struct.PermissionStateQuery.html).
- В используемом Wry Rust binding обработчик этого сигнала не сгенерирован: строка `connect_query_permission_state` закомментирована как неподдерживаемый `PermissionStateQuery`. Для опыта понадобится аккуратный низкоуровневый вызов либо отдельное исправление binding. [Исходный код binding](https://github.com/tauri-apps/webkit2gtk-rs/blob/crate/src/auto/web_view.rs).

## Подготовленный опыт

1. [Прототип](../../prototypes/wry-linux-permission-origin/README.md) поднимает два локальных HTTP origin на `127.0.0.1` с разными портами. Он сравнивает верхнюю страницу, iframe того же origin и iframe другого origin с явным `allow="camera; microphone"`. [Permissions Policy](https://www.w3.org/TR/permissions-policy/).
2. Для каждой страницы он записывает JS `location.origin`, исход двух вызовов `navigator.permissions.query({name: 'camera'})` и одного `getUserMedia({video: true})`. Отдельно записывает вид разрешения в Wry и имя с `security_origin` в нативном сигнале. Wry отвечает `Deny` на любые запросы.
3. Код 0 означает только полный сбор диагностических отчётов без выданного потока. Он не доказывает, что origin из `query-permission-state` соответствует конкретному вызову `getUserMedia()` или что сигнал приходит перед каждым запросом.

Критерий для Search: перед каждым решением о доступе должен быть известен достоверный origin конкретного запроса. Отсутствие callback, отказ из-за `Permissions-Policy` или отсутствие устройства не доказывают, что этот критерий выполнен.

## Наблюдения

Linux CI ещё не запускался. На Mac проверены формат Rust, согласованность lock-файла, проектные ссылки и отсутствие ошибок в diff. Требуются результат Linux CI, iframe без `allow` как отрицательный контроль, повтор именно `getUserMedia()`, порядок нативных событий, версии WebKitGTK и проверка другого профиля. До этих проверок решение о нативном обработчике остаётся открытым.
