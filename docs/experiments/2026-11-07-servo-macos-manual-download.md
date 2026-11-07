# Servo 0.7.0: скачивание после обычного клика на macOS

Статус: один UI-прогон официального ServoShell на сохранённом fixture Search. В этом сценарии обычный клик не привёл к наблюдаемому сохранению файла. Это узкий результат для ServoShell v0.7.0 на Mac.

## Сценарий

- 7 ноября 2026 года, macOS 27.0.1, Apple Silicon, официальный [Servo v0.7.0](https://github.com/servo/servo/releases/tag/v0.7.0), коммит `aac43a3f31a259f04a574f5ec4e959c943ad7cc7`. Использован ранее проверенный образ из [первого опыта](2026-11-06-servo-macos-release-smoke.md).
- Запущен [`prototypes/engine-fixtures/download_server.py`](../../prototypes/engine-fixtures/download_server.py). ServoShell открыт с отдельным временным `--config-dir=/private/tmp/search-servo-manual-download-profile` на `http://127.0.0.1:43129/manual`.
- Страница `/manual` показывает ссылку `Download test file` и не нажимает её скриптом. Ответ `/download` тот же, что в [автоматическом опыте](2026-11-07-servo-macos-download.md): HTTP 200, `Content-Type: application/octet-stream`, `Content-Disposition: attachment; filename="search-servo-v070-probe.txt"`, 28 байт тестового содержимого.

## Наблюдение

Страница и ссылка отобразились в окне. Через доступное управление окном выполнен один настоящий клик по ссылке; перед ним сервер записал только `GET /manual` 200, после него `GET /download` 200. Адресная строка перешла на `/download`. На странице показано `Unknown content type (application/octet-stream)`. В логах ServoShell есть `unknown completed load PageSource("http://127.0.0.1:43129/download")`.

Файл `search-servo-v070-probe.txt` после клика не найден в `~/Downloads` и во временном каталоге конфигурации. Поиск по всему диску не проводился. Процесс ServoShell и сервер остановлены, образ отключён. Скриншот интерфейса просмотрен во время опыта, в репозиторий не добавлен.

## Вывод и ограничения

В проверенном сценарии реальный клик дал тот же видимый результат, что автоматическое нажатие: запрос файла был, сохранённый файл в проверенных местах не появился. Автоматическое нажатие не объясняет наблюдавшийся сбой само по себе. Это не доказывает, что Servo вообще не умеет скачивать файлы: не проверены другой MIME-тип, другие места сохранения, новые версии Servo, встраивание движка, Windows и Linux. Для архитектуры Search сценарий скачивания через Servo остаётся открытой задачей.
