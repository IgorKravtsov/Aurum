# Aurum CLI

Node.js-клиент для всех endpoint'ов Aurum API. CLI устанавливается на хосте и обращается к экземпляру Aurum, запущенному через Docker или другим способом; в runtime-образы проекта пакет не включается.

## Установка

Требуется Node.js 20 или новее.

```bash
npm install -g ./cli
aurum --version
aurum health
```

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
