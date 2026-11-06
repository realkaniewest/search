# Локальные страницы для сравнения движков

Запуск из корня проекта:

```sh
python3 -m http.server 43128 --bind 127.0.0.1 --directory prototypes/engine-fixtures
```

Откройте `http://127.0.0.1:43128/basic.html`. Страница не загружает внешние ресурсы и не содержит личных данных. После нажатия кнопки заголовок вкладки меняется с `Search fixture: ready` на `Search fixture: clicked`.
