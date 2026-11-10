# Wry 0.57.1: первый запуск WebView2 в Windows CI

Статус: Windows-only исследовательский прототип запустился на GitHub `windows-2025`, выполнил JavaScript локальной страницы и передал точный маркер в Rust. Пиксельная отрисовка и работа на пользовательском ПК не проверены.

## Среда и процедура

- 10 ноября 2026 года, GitHub Actions `windows-2025`, Wry 0.57.1, Tao 0.37.2, Rust 1.99.1. Код: [`prototypes/wry-windows-runtime`](../../prototypes/wry-windows-runtime/README.md). [CI job PR #29](https://github.com/realkaniewest/search/actions/runs/42742179954/job/129585665292).
- CI проверил формат, Clippy с `-D warnings`, сборку с закреплённым `Cargo.lock`, затем запустил бинарник. PowerShell ограничил ожидание процесса 90 секундами, job ограничен 15 минутами.
- Приложение создало окно Tao и WebView2, загрузило только встроенный HTML. JavaScript поменял `document.title`; Rust принимал только точное значение `Search Windows smoke ready`. Обработчик разрешений установлен в `Deny`, включён incognito. IPC и внешняя сеть не используются.
- В Windows-прототипе `with_additional_browser_args("")` заменяет аргументы Wry по умолчанию, содержащие отключение `msSmartScreenProtection`. Фактическая работа SmartScreen этим опытом не измерялась.

## Наблюдения

- Windows job завершился успешно. Журнал шага запуска содержит `TITLE Search Windows smoke ready` и `PASS: local HTML changed the document title via JavaScript`.
- Это подтверждает создание WebView2 и выполнение JavaScript в среде GitHub runner. Проверка точного маркера встроена в CI: отсутствие строки или ненулевой код процесса приводят к ошибке job.
- Сборка Wry на Windows и Ubuntu, macOS job и проверка файлов в этом же прогоне прошли. Linux runtime job проверяет отдельный сценарий.

## Границы

Запуск на runner не доказывает видимые пиксели, пользовательский ввод, доступность, медиа, реальные сайты, работу на физическом Windows-ПК, изоляцию профилей, удаление временных данных или полноту разрешений. По [документации Wry](https://docs.rs/wry/0.57.1/wry/struct.WebViewBuilder.html#method.with_incognito), incognito на Windows требует WebView2 Runtime 101.0.1210.39 или новее; версия и удаление данных здесь не проверялись. До архитектурного решения нужен отдельный сценарий работы на обычной учётной записи и проверка origin-aware разрешений. Время job не является скоростью браузера.
