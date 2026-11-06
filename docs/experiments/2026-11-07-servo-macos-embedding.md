# Servo 0.7.0: встраивание в окно macOS

Статус: исходники Servo собраны, официальный минимальный пример встраивания запущен на Mac. Локальная HTML-страница отрисовалась в отдельном окне. Это узкая проверка API, не прототип браузера Search и не выбор архитектуры.

## Подтверждённые факты и среда

- 7 ноября 2026 года, macOS 27.0.1, Apple Silicon. Проверка на Windows и Linux не проводилась.
- Исходники [Servo v0.7.0](https://github.com/servo/servo/tree/v0.7.0) клонированы по тегу с коммитом `aac43a3f31a259f04a574f5ec4e959c943ad7cc7` в `/private/tmp/search-servo-v0.7.0-src`. Исходники не добавлены в Search.
- Собран [официальный пример `winit_minimal.rs`](https://github.com/servo/servo/blob/v0.7.0/components/servo/examples/winit_minimal.rs) командой `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=/private/tmp/search-servo-v0.7.0-target cargo build --locked -p servo --example winit_minimal`. Сборка завершилась с кодом 0 за 2 минуты 49 секунд в профиле `dev`. Это время первой сборки с загрузкой зависимостей, не показатель скорости браузера.
- `./mach bootstrap --yes --skip-lints --skip-nextest` остановился на системной установке GStreamer: `sudo` потребовал интерактивный пароль. Затем `./mach bootstrap --yes --skip-platform --skip-lints --skip-nextest` подготовил закреплённый Rust 1.97.1. Прямая сборка Cargo использовала `servo-media-dummy`; медиа не проверялось. [Servo Book](https://book.servo.org/embedding/overview.html) отдельно описывает различие между сборкой через Cargo и `mach` для медиа.

## Проверки

| Сценарий | Наблюдение |
| --- | --- |
| Исходный пример с `https://demo.servo.org/experiments/twgl-tunnel/` | Окно `winit window` открылось, но область страницы осталась белой. В логе есть `fail to evaluate module`. Причина не установлена. |
| Тот же пример с заменой только URL на локальный `data:text/html` | Окно показало заголовок `Servo embedded OK` и строку `Search prototype` на заданном фоне. Результат проверен снимком экрана. Изменение URL сделано только во временном клоне Servo. |
| Завершение | Оба процесса остановлены `Ctrl-C` с кодом 130. Штатное закрытие окна не проверялось. |

Для повторения второй проверки в `components/servo/examples/winit_minimal.rs` нужно заменить строку URL на `data:text/html,%3Chtml%3E%3Cbody%20style%3D%22font%3A32px%20sans-serif%3Bbackground%3A%23eef5ff%22%3E%3Ch1%3EServo%20embedded%20OK%3C%2Fh1%3E%3Cp%3ESearch%20prototype%3C%2Fp%3E%3C%2Fbody%3E%3C%2Fhtml%3E`, пересобрать той же командой и запустить `/private/tmp/search-servo-v0.7.0-target/debug/examples/winit_minimal`. Тестовый URL не требует сети. Снимок экрана хранится только во временной папке `/private/tmp/search-servo-embed-local.png` и не входит в репозиторий.

## Что это значит для Search

Подтверждено: Servo 0.7.0 можно собрать из закреплённых исходников и встроить в окно `winit` на этом Mac; простая HTML-страница отрисовывается. Не подтверждено: работа исходной демонстрационной страницы, ввод с клавиатуры и мыши, навигация, безопасность, профили, приватные данные, разрешения, медиа, реальные сайты и работа на других ОС.

Это пример с одним окном и без браузерного интерфейса. До ADR нужны одинаковые сценарии сравнения с Wry и разбор стоимости поддержки. Ускорение не измерялось.
