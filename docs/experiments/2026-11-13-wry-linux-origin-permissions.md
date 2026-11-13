# Wry: origin запроса разрешения на Linux

Статус: два успешных диагностических запуска Linux CI 13 ноября 2026 года. Задачи [#41](https://github.com/realkaniewest/search/issues/41) и [#43](https://github.com/realkaniewest/search/issues/43). Эта запись не подтверждает работу управления разрешениями в браузере Search.

## Проверенные сведения API

- В Wry 0.57.1 обработчик `permission-request` на Linux передаёт общему API вид разрешения без origin запрашивающего frame. [Исходный код Wry](https://github.com/tauri-apps/wry/blob/wry-v0.57.1/src/webkitgtk/mod.rs).
- `WebKitUserMediaPermissionRequest` сообщает, запрошены ли аудио и видео, но его публичный API не содержит origin. [WebKitGTK](https://webkitgtk.org/reference/webkit2gtk/2.41.3/class.UserMediaPermissionRequest.html).
- Отдельный `query-permission-state` передаёт `WebKitPermissionStateQuery` с `security_origin`. Документация не обещает, что этот сигнал приходит для каждого `getUserMedia()` или что он позволяет однозначно связать origin с последующим `permission-request`. [Сигнал](https://webkitgtk.org/reference/webkit2gtk/2.41.3/signal.WebView.query-permission-state.html), [тип запроса](https://webkitgtk.org/reference/webkit2gtk/2.40.2/struct.PermissionStateQuery.html).
- В используемом Wry Rust binding обработчик этого сигнала не сгенерирован: строка `connect_query_permission_state` закомментирована как неподдерживаемый `PermissionStateQuery`. Прототип подключил сигнал через низкоуровневый C ABI; для браузера это решение не принято. [Исходный код binding](https://github.com/tauri-apps/webkit2gtk-rs/blob/crate/src/auto/web_view.rs).

## Подготовленный опыт

1. [Прототип](../../prototypes/wry-linux-permission-origin/README.md) поднимает два локальных HTTP origin на `127.0.0.1` с разными портами. Он сравнивает верхнюю страницу, iframe того же origin, iframe другого origin с явным `allow="camera; microphone"` и iframe без `allow`. [Permissions Policy](https://www.w3.org/TR/permissions-policy/).
2. Для каждой страницы он записывает JS `location.origin`, исход двух вызовов `navigator.permissions.query({name: 'camera'})` и двух `getUserMedia({video: true})`. Отдельно записывает вид разрешения в Wry и имя с `security_origin` в нативном сигнале. Обработчик Wry настроен отвечать `Deny` на запросы, которые до него доходят.
3. Код 0 означает только полный сбор диагностических отчётов, хотя бы один callback Wry и отсутствие выданного потока. Он не доказывает, что origin из `query-permission-state` соответствует конкретному вызову `getUserMedia()` или что сигнал приходит перед каждым запросом.

Критерий для Search: перед каждым решением о доступе должен быть известен достоверный origin конкретного запроса. Отсутствие callback, отказ из-за `Permissions-Policy` или отсутствие устройства не доказывают, что этот критерий выполнен.

## Наблюдения

Первый запуск: [GitHub Actions, job `wry-linux-permission-diagnostic`](https://github.com/realkaniewest/search/actions/runs/43309326145/job/131500895477), Ubuntu 24.04 `x86_64`, WebKitGTK 4.1 версии `2.52.6-0ubuntu0.24.04.1`, Wry 0.57.1, Xvfb. Сборка и Clippy прошли. Девять отчётов страницы получены; верхняя страница и same-origin iframe сообщили один адрес, cross-origin iframe сообщил второй. Все три `getUserMedia()` завершились `OverconstrainedError`; поток не выдавался.

В журнале нативный `query-permission-state` записал origin верхней страницы, но ни одной записи с origin cross-origin iframe. Обработчик Wry для фактического разрешения не вызвался (`denied_callbacks=0`). Статус job зелёный, потому что диагностические страницы завершились, **а не потому, что привязка разрешения к origin доказана**. При отсутствии устройства ошибка `OverconstrainedError` могла возникнуть до запроса разрешения. Нельзя однозначно связать записанные нативные query с конкретными вызовами из iframe.

Повторный опыт: [GitHub Actions, job `wry-linux-permission-diagnostic`](https://github.com/realkaniewest/search/actions/runs/43444115317/job/131948974141) с той же Ubuntu и WebKitGTK 2.52.6. Перед загрузкой страниц включены [штатные синтетические устройства WebKitGTK](https://webkitgtk.org/reference/webkit2gtk/2.38.5/property.Settings.enable-mock-capture-devices.html) и media stream. Верхняя страница, same-origin iframe, cross-origin iframe с `allow` и cross-origin iframe без `allow` выполнили по два `permissions.query()` и `getUserMedia()`. Получены все 16 отчётов. В iframe без `allow` query вернул `denied`; все восемь `getUserMedia()` вернули `NotAllowedError`. Поток не выдавался. Обработчик Wry получил два запроса `Camera` и оба отклонил.

Нативные `query-permission-state` в этом запуске снова содержали только origin верхней страницы: `top_origin_seen=true`, `cross_origin_seen=false`. Порядок событий записан общим счётчиком, но отдельные вызовы Wry нельзя достоверно сопоставить с iframe. Два callback могли относиться к верхней странице или к вложенным страницам; результат не доказывает, что Wry способен показать пользователю origin каждого запроса. Отказ iframe без `allow` наблюдался на уровне страницы; прототип не доказал, что его причина только в `Permissions-Policy`.

На Mac проверены формат Rust, согласованность lock-файла, проектные ссылки и отсутствие ошибок в diff. Дальше нужен изолированный cross-origin сценарий без одновременного запроса из верхней страницы, затем проверка сохранения и отзыва разрешений и другого профиля. До этих проверок решение о нативном обработчике остаётся открытым.
