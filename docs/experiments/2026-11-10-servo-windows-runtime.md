# ServoShell v0.8.0: Windows runtime probe

Статус: выполнен [CI-опыт на Windows 2025](https://github.com/realkaniewest/search/actions/runs/42796677414/job/129763521066) для [задачи #35](https://github.com/realkaniewest/search/issues/35). Это официальный ServoShell, не встраивание движка в Search.

## Зафиксированные входные данные

- [Servo v0.8.0](https://github.com/servo/servo/releases/tag/v0.8.0), коммит `4e136d8ef31b7c4748fbfac606ca463f3efd7537`, ZIP `servo-x86_64-windows-msvc.zip`, опубликованный SHA-256 `0fca2f4d7e6107304631f7b9031edf0df2c3d88e0570cd19cde2ef90379bee8c`. Первый CI-прогон отверг неверно записанный digest до распаковки и запуска; после исправления журнал показал `SHA256: official Servo ZIP matches pinned digest`.
- [Локальная страница](../../prototypes/servo-windows-runtime/probe.html) подаётся с `127.0.0.1:43132`. JavaScript меняет серый прямоугольник на цвет `#14b8a6`.
- [Сценарий](../../prototypes/servo-windows-runtime/run.ps1) проверяет SHA-256 до распаковки и запуска, наличие `servoshell.exe`, аргумент временного профиля, GET от Servo и пиксели цвета после JavaScript. Временный каталог намеренно содержит пробел, а пути передаются процессам отдельными аргументами через `ProcessStartInfo.ArgumentList`. Успешный PNG подтверждает передачу пути вывода с пробелом; фактическая изоляция профиля отдельно не проверялась. Процесс ограничен 90 секундами, job 15 минутами.

## Матрица результата

| Проверка | Результат |
| --- | --- |
| Контрольная сумма ZIP и наличие `servoshell.exe` | Проходит в CI job `129763521066` |
| Запуск Windows-бинарника с `--headless --exit --output` | Проходит в CI job `129763521066` |
| HTTP GET и JavaScript-отрисовка в PNG | Проходит: `7000` проверенных пикселей в PNG `1024×740` |
| UI на физическом ПК, ввод, реальные сайты, загрузки, медиа, профили | Не проверено |
| Сборка Servo из исходников и встраивание в Search | Не проверено |

В [журнале CI](https://github.com/realkaniewest/search/actions/runs/42796677414/job/129763521066) после проверки SHA записано `PASS: ServoShell requested local page and rendered 7000 sampled JavaScript-colored pixels in 1024x740 PNG`. PNG используется только для машинной проверки; в репозиторий не сохраняется. Локальный сервер пишет в лог путь без query string, чтобы запрос с параметрами не записал возможный токен. Скрипт пытается остановить дерево процессов и удалить временные файлы при обычном выходе, но авария runner или принудительное завершение могут помешать очистке. Этот CI подтвердил только узкий headless-сценарий на виртуальной Windows. Скорость не измерялась.
