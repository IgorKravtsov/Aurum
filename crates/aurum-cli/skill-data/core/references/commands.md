# Полный справочник Aurum CLI

## Глобальные параметры

| Флаг | Переменная окружения | Назначение |
|---|---|---|
| `--api-base-url URL` | `AURUM_API_BASE_URL` | Базовый URL API; по умолчанию `http://127.0.0.1:3000/api` |
| `--url URL` | `AURUM_URL` | Короткие псевдонимы основной настройки URL |
| `--user USER` | `AURUM_USER` | Пользователь HTTP Basic Auth |
| `--password PASSWORD` | `AURUM_PASSWORD` | Пароль HTTP Basic Auth |
| `--password-file PATH` | `AURUM_PASSWORD_FILE` | Локальный файл с паролем |
| `--header 'Name: value'` | — | Дополнительный заголовок; можно повторять |
| `--timeout MS` | `AURUM_TIMEOUT` | Таймаут HTTP-запроса; по умолчанию 30000 мс |
| `--output PATH` | — | Записать ответ в файл с правами `0600` |
| `--compact` | — | Компактный JSON |

Постоянная конфигурация хранится в `${XDG_CONFIG_HOME:-~/.config}/aurum/config.json` с правами `0600`. Путь можно переопределить через `AURUM_CONFIG_FILE`.

```text
aurum config set api-base-url <URL>
aurum config get api-base-url
aurum config unset api-base-url
aurum config path
```

Приоритет адреса: `--api-base-url` → `AURUM_API_BASE_URL` → локальная конфигурация → `http://127.0.0.1:3000/api`.

## Именованные команды

```text
health

accounts list [--include-archived true]
accounts create
accounts update <account_id>
accounts delete <account_id>

categories list [--kind income|expense]
categories create
categories update <category_id>
categories delete <category_id>

tags list
tags create
tags delete <tag_id>

transactions list [--year N] [--month N] [--start-date DATE] [--end-date DATE]
                  [--account-id ID] [--category-id ID] [--tag-id ID]
                  [--type income|expense|transfer] [--search TEXT]
                  [--sort date_desc|amount_desc|amount_asc] [--page N] [--page-size N]
transactions years
transactions create
transactions bulk
transactions update <transaction_id>
transactions delete <transaction_id>

recurring list
recurring create
recurring update <recurring_id>
recurring delete <recurring_id>
recurring post <recurring_id>

budgets list
budgets status [--year N] [--month N]
budgets create
budgets update <budget_id>
budgets delete <budget_id>

goals list
goals create
goals update <goal_id>
goals delete <goal_id>
goals contribute <goal_id>

assets list
assets create
assets update <asset_id>
assets delete <asset_id>
assets valuations <asset_id>
assets value <asset_id>

crypto portfolios [--include-archived true]
crypto portfolio-create
crypto portfolio-update <portfolio_id>
crypto portfolio-delete <portfolio_id>
crypto holdings [--portfolio-id ID]
crypto refresh [--portfolio-id ID]
crypto holding-create
crypto holding-update <asset_id>
crypto transactions <asset_id>
crypto transaction-add <asset_id>
crypto transaction-update <transaction_id>
crypto transaction-delete <transaction_id>
crypto search --q TEXT
crypto history [--range 7d|30d|90d|all] [--portfolio-id ID]
crypto performance [--portfolio-id ID]

dashboard summary [--year N] [--month N]
cash-flow get [--start-date DATE] [--end-date DATE]
net-worth summary [--range 30d|90d|1y|5y|all]
reports category-spending --category-id ID [--start-date DATE] [--end-date DATE]
reports category-ranking [--kind income|expense] [--start-date DATE] [--end-date DATE]
insights alerts
advice get
settings get
settings update
backup export
backup import
```

Все POST/PATCH-команды с телом принимают `--data JSON`, `--file PATH`, повторяемый `--field key=value` или поля как прямые флаги.

## Поля тел

- Счёт: `name`, `type`, `currency`, `color`; при обновлении также `is_archived`.
- Категория: `name`, `kind`, `icon`, `color`, `sort_order`, `parent_id` (`kind` задаётся только при создании).
- Тег: `name`.
- Операция: `account_id`, `category_id`, `transfer_account_id`, `type`, `amount`, `destination_amount`, `description`, `merchant`, `notes`, `date`, `tag_ids`, `splits`. Bulk-тело: `{"items":[...]}`.
- Регулярная операция: поля обычной операции без тегов/разбиений, плюс `frequency` (`weekly`, `monthly`, `yearly`), `anchor_date`; при обновлении также `is_active`.
- Бюджет: `category_id`, `monthly_limit`; при обновлении — `monthly_limit`.
- Цель: `name`, `target_amount`, `target_date`. Взнос: `amount`, `date`, `note`; отрицательная сумма означает снятие, ноль запрещён.
- Актив: `name`, `asset_class`, `currency`, `notes`, `capital_role`, `monthly_cash_flow`, `risk_level`; при создании также `value`, `as_of_date`. Оценка: `value`, `as_of_date`.
- Криптопортфель: `name`; при обновлении также `is_archived`.
- Новый криптоактив: `portfolio_id`, `coingecko_id`, `symbol`, `name`, `thumb_url`, `quantity`, `price_per_unit`, `date`, `note`, `risk_level`, `network`.
- Метаданные криптоактива: `network`. Криптооперация: `type` (`buy` или `sell`), `quantity`, `price_per_unit`, `date`, `note`.
- Настройки: `currency`, `negative_cash_flow_threshold_months`, `net_worth_decline_threshold_months`, `risky_allocation_threshold_percent`, `idle_cash_threshold_amount`, `idle_cash_threshold_days`.
- Импорт бэкапа принимает без изменений JSON, ранее полученный через `backup export`.

Точные ограничения схемы конкретного запущенного сервера доступны в его `/api/docs` или `/api/openapi.json`.

## Команды skills

```text
aurum skills
aurum skills list
aurum skills get core
aurum skills get core --full
aurum skills get --all
aurum skills path [core]
```

Все команды `skills` поддерживают `--json`.
