# Aurum CLI

Rust-клиент (бинарник `aurum`) для всех endpoint'ов Aurum API. CLI устанавливается на хосте и обращается к экземпляру Aurum, запущенному через Docker или другим способом; в runtime-образы проекта он не включается. Это один исполняемый файл: Node.js и другие рантаймы не нужны.

Крейт `aurum-cli` отвечает за разбор аргументов, конфигурацию и вывод, а HTTP-запросы делает переиспользуемая библиотека `aurum-client` (она же будет работать в мобильных приложениях). Конфигурация и пароли читаются только в CLI; клиент ничего не пишет на диск.

## Установка

Готовые бинарники для Linux и macOS (x86_64 и aarch64) публикуются в GitHub Releases вместе с тегом `vX.Y.Z` (архив `aurum-<target>.tar.gz` с файлом `aurum` и контрольной суммой `.sha256`). Распакуйте его в каталог из `PATH`, например:

```bash
curl -fsSL -o aurum.tar.gz https://github.com/IgorKravtsov/Aurum/releases/latest/download/aurum-x86_64-unknown-linux-gnu.tar.gz
tar -xzf aurum.tar.gz -C ~/.local/bin aurum
```

Либо соберите из исходников (нужен Rust 1.88 или новее):

```bash
cargo install --locked --path crates/aurum-cli
aurum --version
aurum health
```

Установить или обновить CLI и discovery-скилл на SSH-хосте (по умолчанию `pi-srv`) без `sudo` можно скриптом `./install-cli-pi-srv.sh [SSH_HOST]`: он берёт готовый бинарник из релиза, а если его нет — собирает `aurum` (локально или на сервере через `cargo install`).

Для установки короткого discovery-скилла в `~/.agents/skills` и `~/.claude/skills`:

```bash
./install-skill.sh
```

## Быстрый старт

```bash
aurum accounts list
aurum transactions list --year 2026 --month 9
aurum accounts create --name "Cash" --type cash --currency USD
aurum backup export --output ./aurum-backup.json
```

Для другого сервера достаточно один раз сохранить базовый URL:

```bash
aurum config set api-base-url https://finance.example.com/api
aurum settings get
```

Настройка хранится в `${XDG_CONFIG_HOME:-~/.config}/aurum/config.json`. Её можно проверить, сбросить или найти командами:

```bash
aurum config get api-base-url
aurum config unset api-base-url
aurum config path
```

Basic Auth настраивается без сохранения чувствительных данных в проекте:

```bash
export AURUM_USER=myuser
export AURUM_PASSWORD_FILE=/secure/local/path/aurum-password
aurum settings get
```

Переменная окружения и флаг имеют приоритет над сохранённым адресом:

```bash
export AURUM_API_BASE_URL=https://temporary.example.com/api
aurum --api-base-url https://finance.example.com/api accounts list
```

Полная справка:

```bash
aurum --help
aurum skills get core --full
```

Справочник для агентов встроен в бинарник (`crates/aurum-cli/skill-data`), поэтому всегда соответствует установленной версии. `aurum skills path [name]` распаковывает его в `${XDG_CACHE_HOME:-~/.cache}/aurum/skill-data` и печатает путь.

## Разработка

```bash
cargo test -p aurum-cli -p aurum-client
cargo clippy -p aurum-cli -p aurum-client --all-targets -- -D warnings
```

Тесты запускают настоящий бинарник `aurum` против локального mock-сервера, отдельная база или backend не нужны. Выпуск бинарников: тег `vX.Y.Z` (совпадающий с версией в `Cargo.toml`) запускает `.github/workflows/release-cli.yml`.
