# Wry: origin запроса разрешения на Linux

Статус: первый запуск Linux CI 13 ноября 2026 года. Задача [#41](https://github.com/realkaniewest/search/issues/41). Эта запись не подтверждает работу управления разрешениями в браузере Search.

## Проверенные сведения API

- В Wry 0.57.1 обработчик `permission-request` на Linux передаёт общему API вид разрешения без origin запрашивающего frame. [Исходный код Wry](https://github.com/tauri-apps/wry/blob/wry-v0.57.1/src/webkitgtk/mod.rs).
- `WebKitUserMediaPermissionRequest` сообщает, запрошены ли аудио и видео, но его публичный API не содержит origin. [WebKitGTK](https://webkitgtk.org/reference/webkit2gtk/2.41.3/class.UserMediaPermissionRequest.html).
- Отдельный `query-permission-state` передаёт `WebKitPermissionStateQuery` с `security_origin`. Документация не обещает, что этот сигнал приходит для каждого `getUserMedia()` или что он позволяет однозначно связать origin с последующим `permission-request`. [Сигнал](https://webkitgtk.org/reference/webkit2gtk/2.41.3/signal.WebView.query-permission-state.html), [тип запроса](https://webkitgtk.org/reference/webkit2gtk/2.40.2/struct.PermissionStateQuery.html).
- В используемом Wry Rust binding обработчик этого сигнала не сгенерирован: строка `connect_query_permission_state` закомментирована как неподдерживаемый `PermissionStateQuery`. Прототип подключил сигнал через низкоуровневый C ABI; для браузера это решение не принято. [Исходный код binding](https://github.com/tauri-apps/webkit2gtk-rs/blob/crate/src/auto/web_view.rs).

## Подготовленный опыт

1. [Прототип](../../prototypes/wry-linux-permission-origin/README.md) поднимает два локальных HTTP origin на `127.0.0.1` с разными портами. Он сравнивает верхнюю страницу, iframe того же origin и iframe другого origin с явным `allow="camera; microphone"`. [Permissions Policy](https://www.w3.org/TR/permissions-policy/).
2. Для каждой страницы он записывает JS `location.origin`, исход двух вызовов `navigator.permissions.query({name: 'camera'})` и одного `getUserMedia({video: true})`. Отдельно записывает вид разрешения в Wry и имя с `security_origin` в нативном сигнале. Обработчик Wry настроен отвечать `Deny` на запросы, которые до него доходят.
3. Код 0 означает только полный сбор диагностических отчётов без выданного потока. Он не доказывает, что origin из `query-permission-state` соответствует конкретному вызову `getUserMedia()` или что сигнал приходит перед каждым запросом.

Критерий для Search: перед каждым решением о доступе должен быть известен достоверный origin конкретного запроса. Отсутствие callback, отказ из-за `Permissions-Policy` или отсутствие устройства не доказывают, что этот критерий выполнен.

## Наблюдения

Первый запуск: [GitHub Actions, job `wry-linux-permission-diagnostic`](https://github.com/realkaniewest/search/actions/runs/43309326145/job/131500895477), Ubuntu 24.04 `x86_64`, WebKitGTK 4.1 версии `2.52.6-0ubuntu0.24.04.1`, Wry 0.57.1, Xvfb. Сборка и Clippy прошли. Девять отчётов страницы получены; верхняя страница и same-origin iframe сообщили один адрес, cross-origin iframe сообщил второй. Все три `getUserMedia()` завершились `OverconstrainedError`; поток не выдавался.

В журнале нативный `query-permission-state` записал origin верхней страницы, но ни одной записи с origin cross-origin iframe. Обработчик Wry для фактического разрешения не вызвался (`denied_callbacks=0`). Статус job зелёный, потому что диагностические страницы завершились, **а не потому, что привязка разрешения к origin доказана**. При отсутствии устройства ошибка `OverconstrainedError` могла возникнуть до запроса разрешения. Нельзя однозначно связать записанные нативные query с конкретными вызовами из iframe.

На Mac проверены формат Rust, согласованность lock-файла, проектные ссылки и отсутствие ошибок в diff. Дальше нужны iframe без `allow` как отрицательный контроль, повтор именно `getUserMedia()`, устройство или контролируемый медиа-источник для достижения обработчика без доступа к личной камере, порядок событий и проверка другого профиля. До этих проверок решение о нативном обработчике остаётся открытым.
