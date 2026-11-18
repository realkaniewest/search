# Wry: ограниченный опыт на публичных сайтах Mac

Отдельный исследовательский прототип Wry 0.57.1 и Tao 0.37.2. Он не меняет локальный тест в `prototypes/wry-comparison`.

```sh
cargo +1.99.1 run --locked --manifest-path prototypes/wry-public-sites/Cargo.toml -- rust
cargo +1.99.1 run --locked --manifest-path prototypes/wry-public-sites/Cargo.toml -- wikipedia
cargo +1.99.1 run --locked --manifest-path prototypes/wry-public-sites/Cargo.toml -- excalidraw
```

Программа принимает только три ключа, открывает закреплённые HTTPS-адреса в отдельном временном WebView и завершает работу при обнаружении ограниченного признака страницы в DOM: видимого по размерам заголовка Rust, поля поиска Wikipedia или холста Excalidraw. Верхняя навигация остаётся на разрешённых доменах выбранного сайта. Разрешения и новые окна отклоняются, скачивания запрещены. IPC и внутренние команды не создаются. Программа не сохраняет cookies и содержимое страницы в отчёт.

`PASS` означает только завершение загрузки и наличие DOM-признака. Он не доказывает, что страница полностью отрисована, ссылки и ввод работают, Wikipedia ищет, Excalidraw рисует и сохраняет, либо что сайт совместим с Wry на других ОС. Это не сравнение скорости с ServoShell. Сайты могут измениться, поэтому вместе с результатом записывайте дату и наблюдённые ошибки. Для пользовательских действий и снимка нужен отдельный сценарий.

## Проверка отказа TLS на macOS

```sh
cargo build --locked --manifest-path prototypes/wry-public-sites/Cargo.toml --example tls_boundary
python3 prototypes/wry-public-sites/check_tls_boundary.py \
  prototypes/wry-public-sites/target/debug/examples/tls_boundary
```

Требуются Python 3 и OpenSSL с поддержкой `req -addext`. Сценарий создаёт одноразовый самоподписанный сертификат для loopback и запускает отдельное окно с Wry. Сертификат не устанавливается в систему. Успех требует TLS-отказа от клиента, отсутствия HTTP-запроса и отсутствия завершившейся загрузки тестового документа. Таймаут без TLS-отказа считается провалом. Проверка относится к данному сценарию WKWebView, отдельно от режима навигации Search.
