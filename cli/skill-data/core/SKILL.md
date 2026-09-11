---
name: core
description: Актуальное руководство по Aurum CLI для работы со счетами, операциями, бюджетами, целями, активами, криптопортфелями, отчётами, настройками и резервными копиями через Aurum API.
allowed-tools: Bash(aurum:*)
---

# Aurum CLI

`aurum` — Node.js-клиент для REST API личного финансового трекера Aurum. Инструкции поставляются вместе с CLI и соответствуют установленной версии.

Перед работой проверьте доступность экземпляра:

```bash
aurum health
aurum settings get
```

По умолчанию CLI обращается к `http://127.0.0.1:3000/api`. Чтобы один раз сохранить другой адрес для всех следующих запросов:

```bash
aurum config set api-base-url https://finance.example.com/api
```

Проверить, сбросить или найти локальную конфигурацию можно командами `aurum config get api-base-url`, `aurum config unset api-base-url` и `aurum config path`. Для разового переопределения используйте `--api-base-url`; для окружения — `AURUM_API_BASE_URL`. Приоритет: флаг → переменная окружения → сохранённая конфигурация → локальный адрес по умолчанию. `--url` и `AURUM_URL` сохранены как короткие псевдонимы.

## Аутентификация

Если в Aurum включён HTTP Basic Auth, используйте `AURUM_USER` и `AURUM_PASSWORD`. Не записывайте пароль в репозиторий и по возможности не передавайте его через `--password`, поскольку аргументы процесса могут быть видны другим локальным пользователям. Поддерживается локальный файл с паролем через `--password-file` или `AURUM_PASSWORD_FILE`.

```bash
export AURUM_API_BASE_URL=https://finance.example.com/api
export AURUM_USER=myuser
export AURUM_PASSWORD_FILE="$PWD/.aurum-password"
aurum accounts list
```

## Передача данных

Поля JSON-тела передаются обычными флагами в kebab-case. CLI преобразует их в snake_case и распознаёт числа, `true`, `false`, `null`, JSON-массивы и JSON-объекты.

```bash
aurum accounts create --name "Main card" --type debit_card --currency EUR
aurum goals create --name "Emergency fund" --target-amount 10000 --target-date 2027-12-31
aurum transactions create \
  --account-id 1 --type expense --amount 42.50 \
  --description "Groceries" --date 2026-09-12 --tag-ids '[1,2]'
```

Для крупных или вложенных документов используйте `--file`, для готового JSON — `--data`. Отдельные `--field key=value` переопределяют поля документа.

```bash
aurum transactions bulk --file ./transactions.json
aurum backup import --file ./aurum-backup.json
aurum backup export --output ./aurum-backup.json
```

PATCH отправляет только указанные поля. Например, `--parent-id null` явно очищает родителя категории, а отсутствие флага не меняет его.

## Чтение и фильтры

Флаги GET-команд становятся query-параметрами:

```bash
aurum transactions list --year 2026 --month 9 --page-size 100
aurum transactions list --search "market" --sort amount_desc
aurum cash-flow get --start-date 2026-01-01 --end-date 2026-12-31
aurum crypto history --range 90d --portfolio-id 2
```

Успешный ответ печатается как форматированный JSON. `--compact` убирает отступы, а `--output PATH` записывает ответ в файл с правами `0600`.

## Произвольный endpoint

Если сервер новее установленной CLI или нужен endpoint без именованной команды, используйте низкоуровневый запрос:

```bash
aurum request GET /future-endpoint --query page=2
aurum request POST /future-endpoint --data '{"enabled":true}'
```

## Безопасность изменений

Команды `delete`, `backup import` и другие изменяющие запросы выполняются сразу, без интерактивного подтверждения, чтобы CLI оставалась пригодной для автоматизации. Перед их запуском проверьте URL, идентификаторы и входной файл; перед массовым или разрушительным изменением сделайте `aurum backup export --output PATH`.

## Полный справочник

Запустите `aurum skills get core --full`, чтобы добавить перечень всех именованных команд, форматы тел и допустимые значения. Обычная справка доступна через `aurum --help`.
