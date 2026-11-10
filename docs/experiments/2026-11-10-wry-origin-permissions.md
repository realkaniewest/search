# Wry 0.57.1: origin в запросах разрешений

Статус: аудит документации и исходного кода от 10 ноября 2026 года. Нативные обработчики Search не реализованы, реальные запросы из двух origin и iframe ещё не прогонялись. До проверки политика исследовательских окон остаётся `Deny`.

## Подтверждённые факты

| Платформа | Что предоставляет движок | Что сейчас передаёт Wry приложению |
| --- | --- | --- |
| macOS | [WKUIDelegate](https://developer.apple.com/documentation/webkit/wkuidelegate/webview%28_%3Arequestmediacapturepermissionfor%3Ainitiatedbyframe%3Atype%3Adecisionhandler%3A%29) получает `WKSecurityOrigin`, `WKFrameInfo` и вид media capture | [Wry 0.57.1](https://github.com/tauri-apps/wry/blob/wry-v0.57.1/src/wkwebview/class/wry_web_view_ui_delegate.rs) принимает origin/frame внутри собственного delegate, но общий `with_permission_handler` передаёт приложению только `PermissionKind` |
| Windows | [WebView2 Win32 PermissionRequested](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2permissionrequestedeventargs?view=webview2-1.0.3967.48) содержит `Uri` источника запроса и вид разрешения; [интерфейс Args3](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2permissionrequestedeventargs3?view=webview2-1.0.3967.48) позволяет не сохранять решение в профиле | [Wry 0.57.1](https://github.com/tauri-apps/wry/blob/wry-v0.57.1/src/webview2/mod.rs) читает вид разрешения и вызывает общий handler без `Uri` |
| Linux | [WebKitGTK `permission-request`](https://webkitgtk.org/reference/webkit2gtk/2.42.5/signal.WebView.permission-request.html) передаёт `WebKitPermissionRequest`. Публичные методы [UserMediaPermissionRequest](https://webkitgtk.org/reference/webkit2gtk/2.38.0/class.UserMediaPermissionRequest.html) описывают аудио и видео, но не origin. Отдельный [PermissionStateQuery](https://webkitgtk.org/reference/webkit2gtk/2.40.2/struct.PermissionStateQuery.html) содержит security origin для запроса состояния разрешения | [Wry 0.57.1](https://github.com/tauri-apps/wry/blob/wry-v0.57.1/src/webkitgtk/mod.rs) подключает `permission-request`, определяет вид запроса и передаёт общему handler только `PermissionKind` |

[Общий API Wry](https://docs.rs/wry/0.57.1/wry/struct.WebViewBuilder.html#method.with_permission_handler) не даёт Search построить собственный запрос разрешения с достоверным именем сайта. Адрес верхней страницы нельзя подставлять вместо источника: запрос может прийти из другого iframe. На Windows по [документации WebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2permissionrequestedeventargs3?view=webview2-1.0.3967.48) сохранённое решение может использоваться в следующих сессиях без нового события; модель хранения и отзыва нужно проектировать отдельно. При реализации нужно проверить доступность `Args3` в выбранной версии runtime.

На macOS [реализация Wry](https://github.com/tauri-apps/wry/blob/wry-v0.57.1/src/wkwebview/class/wry_web_view_ui_delegate.rs) при отсутствии `permission_handler` возвращает `Grant`. Поэтому `Deny` надо явно задавать и проверять для каждого WebView, включая новые окна.

## Гипотезы, которые ещё не доказаны

- На macOS и Windows возможен origin-aware путь через изменение Wry или поддержанное им расширение нативного обработчика. Нельзя просто заменить `WKWebView.uiDelegate` или зарегистрировать второй WebView2 handler и считать задачу решённой: их порядок, влияние на новые окна, загрузки и системные диалоги надо проверить.
- На Linux `query-permission-state` может помочь с origin-aware решением о состоянии разрешения, но из документации выше не следует, что он срабатывает для каждого `getUserMedia()` и даёт origin именно запрашивающего iframe. Нужен опыт с двумя origin. Взять только `webkit_web_view_get_uri()` недостаточно для cross-origin iframe.
- `PermissionResponse::Default` оставляет решение движку. Это не обещает системный диалог: [WebKitGTK UserMediaPermissionRequest](https://webkitgtk.org/reference/webkit2gtk/2.38.0/class.UserMediaPermissionRequest.html) описывает отказ по умолчанию для необработанного media-запроса. Нужно проверить фактическое поведение на каждой ОС, адрес сайта в интерфейсе и хранение ответа. Такой ответ нельзя считать готовым интерфейсом Search.

## Следующие опыты до ADR

1. Сделать локальные страницы `127.0.0.1` на двух портах: верхняя страница, тот же origin во frame и cross-origin frame с явным `allow` для камеры/микрофона. Проверить `Permissions-Policy` и составить матрицу top-level/same-origin/cross-origin при `Deny` и тестовом `Grant`. Записывать возникновение callback, ожидаемые origin и вид запроса, без личных данных. Отсутствие callback не считать успешной изоляцией.
2. Для Windows проверить, можно ли добавить origin-aware обработчик в ту же цепочку WebView2, которой пользуется Wry. Сравнить `Uri` верхней страницы и iframe; проверить `SavesInProfile=false`, повторный запрос и отзыв решения. Не менять аргументы SmartScreen.
3. Для macOS проверить origin и frame внутри делегата Wry на локальных страницах. Предыдущая попытка была остановлена системным отказом камеры до callback; без обхода системного разрешения результат оставить «не проверено». Отдельно проверить, не ломаются ли окна и выбор файлов при изменении delegate.
4. Для Linux проверить, возникает ли `query-permission-state` до `permission-request` для тех же сценариев и совпадает ли его origin с источником запроса в iframe. Если достоверного origin нет, разрешать устройствам доступ через этот путь нельзя.
5. На каждой ОС проверить сохранённые решения, отзыв доступа и изоляцию профилей. Критерий: интерфейс сообщает запрашивающий origin, решение применяется только к нему и не переносится в другой профиль.

## Вывод

Wry остаётся рабочим кандидатом для запуска страниц на трёх ОС, но собственное безопасное управление разрешениями сайтов пока не доказано. Это ограничение влияет на выбор архитектуры: до ADR нужен проверенный origin-aware путь либо пересмотр движка. Нельзя выдавать общую политику `Deny` исследовательских окон за готовую функцию браузера.
