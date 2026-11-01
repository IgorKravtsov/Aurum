const command = (name, method, path, options = {}) => ({
  name,
  method,
  path,
  query: [],
  body: false,
  description: "",
  ...options,
});

// This registry is intentionally declarative: adding a backend endpoint only
// requires one entry here, and the request builder, help, and skill reference
// all continue to use the same command model.
export const COMMANDS = [
  command("health", "GET", "/health", { description: "Проверить доступность API" }),

  command("accounts list", "GET", "/accounts", { query: ["include_archived"], description: "Список счетов" }),
  command("accounts create", "POST", "/accounts", { body: true, description: "Создать счёт" }),
  command("accounts update", "PATCH", "/accounts/{account_id}", { body: true, description: "Изменить счёт" }),
  command("accounts delete", "DELETE", "/accounts/{account_id}", { description: "Удалить счёт и его операции" }),

  command("categories list", "GET", "/categories", { query: ["kind"], description: "Список категорий" }),
  command("categories create", "POST", "/categories", { body: true, description: "Создать категорию" }),
  command("categories update", "PATCH", "/categories/{category_id}", { body: true, description: "Изменить категорию" }),
  command("categories delete", "DELETE", "/categories/{category_id}", { description: "Удалить категорию" }),

  command("tags list", "GET", "/tags", { description: "Список тегов" }),
  command("tags create", "POST", "/tags", { body: true, description: "Создать тег" }),
  command("tags delete", "DELETE", "/tags/{tag_id}", { description: "Удалить тег" }),

  command("transactions list", "GET", "/transactions", {
    query: ["year", "month", "start_date", "end_date", "account_id", "category_id", "tag_id", "type", "search", "sort", "page", "page_size"],
    description: "Список и поиск операций",
  }),
  command("transactions years", "GET", "/transactions/years", { description: "Годы с операциями" }),
  command("transactions create", "POST", "/transactions", { body: true, description: "Создать операцию" }),
  command("transactions bulk", "POST", "/transactions/bulk", { body: true, description: "Импортировать до 5000 операций" }),
  command("transactions update", "PATCH", "/transactions/{transaction_id}", { body: true, description: "Изменить операцию" }),
  command("transactions delete", "DELETE", "/transactions/{transaction_id}", { description: "Удалить операцию" }),

  command("recurring list", "GET", "/recurring", { description: "Список регулярных операций" }),
  command("recurring create", "POST", "/recurring", { body: true, description: "Создать регулярную операцию" }),
  command("recurring update", "PATCH", "/recurring/{recurring_id}", { body: true, description: "Изменить регулярную операцию" }),
  command("recurring delete", "DELETE", "/recurring/{recurring_id}", { description: "Удалить регулярную операцию" }),
  command("recurring post", "POST", "/recurring/{recurring_id}/post", { description: "Провести регулярную операцию сейчас" }),

  command("budgets list", "GET", "/budgets", { description: "Список бюджетов" }),
  command("budgets status", "GET", "/budgets/status", { query: ["year", "month"], description: "Исполнение бюджетов" }),
  command("budgets create", "POST", "/budgets", { body: true, description: "Создать бюджет" }),
  command("budgets update", "PATCH", "/budgets/{budget_id}", { body: true, description: "Изменить бюджет" }),
  command("budgets delete", "DELETE", "/budgets/{budget_id}", { description: "Удалить бюджет" }),

  command("goals list", "GET", "/goals", { description: "Список целей" }),
  command("goals create", "POST", "/goals", { body: true, description: "Создать цель" }),
  command("goals update", "PATCH", "/goals/{goal_id}", { body: true, description: "Изменить цель" }),
  command("goals delete", "DELETE", "/goals/{goal_id}", { description: "Удалить цель" }),
  command("goals contribute", "POST", "/goals/{goal_id}/contributions", { body: true, description: "Добавить взнос или снятие" }),

  command("assets list", "GET", "/assets", { description: "Список активов" }),
  command("assets create", "POST", "/assets", { body: true, description: "Создать актив" }),
  command("assets update", "PATCH", "/assets/{asset_id}", { body: true, description: "Изменить актив" }),
  command("assets delete", "DELETE", "/assets/{asset_id}", { description: "Удалить актив" }),
  command("assets valuations", "GET", "/assets/{asset_id}/valuations", { description: "История оценок актива" }),
  command("assets value", "POST", "/assets/{asset_id}/valuations", { body: true, description: "Записать оценку актива" }),

  command("crypto portfolios", "GET", "/crypto/portfolios", { query: ["include_archived"], description: "Список криптопортфелей" }),
  command("crypto portfolio-create", "POST", "/crypto/portfolios", { body: true, description: "Создать криптопортфель" }),
  command("crypto portfolio-update", "PATCH", "/crypto/portfolios/{portfolio_id}", { body: true, description: "Изменить криптопортфель" }),
  command("crypto portfolio-delete", "DELETE", "/crypto/portfolios/{portfolio_id}", { description: "Удалить криптопортфель" }),
  command("crypto holdings", "GET", "/crypto/holdings", { query: ["portfolio_id"], description: "Криптоактивы и цены" }),
  command("crypto refresh", "POST", "/crypto/refresh", { query: ["portfolio_id"], description: "Принудительно обновить цены" }),
  command("crypto holding-create", "POST", "/crypto/holdings", { body: true, description: "Добавить криптоактив" }),
  command("crypto holding-update", "PATCH", "/crypto/holdings/{asset_id}", { body: true, description: "Изменить метаданные криптоактива" }),
  command("crypto transactions", "GET", "/crypto/holdings/{asset_id}/transactions", { description: "Операции криптоактива" }),
  command("crypto transaction-add", "POST", "/crypto/holdings/{asset_id}/transactions", { body: true, description: "Добавить покупку или продажу" }),
  command("crypto transaction-update", "PATCH", "/crypto/transactions/{transaction_id}", { body: true, description: "Изменить криптооперацию" }),
  command("crypto transaction-delete", "DELETE", "/crypto/transactions/{transaction_id}", { description: "Удалить криптооперацию" }),
  command("crypto search", "GET", "/crypto/search", { query: ["q"], description: "Найти монету в CoinGecko" }),
  command("crypto history", "GET", "/crypto/history", { query: ["range", "portfolio_id"], description: "История стоимости криптопортфеля" }),
  command("crypto performance", "GET", "/crypto/performance/90d", { query: ["portfolio_id"], description: "Доходность монет за 90 дней" }),

  command("dashboard summary", "GET", "/dashboard/summary", { query: ["year", "month"], description: "Сводка панели" }),
  command("cash-flow get", "GET", "/cash-flow", { query: ["start_date", "end_date"], description: "Денежный поток" }),
  command("net-worth summary", "GET", "/net-worth/summary", { query: ["range"], description: "Капитал и его история" }),
  command("reports category-spending", "GET", "/reports/category-spending", { query: ["category_id", "start_date", "end_date"], description: "Расходы выбранной категории" }),
  command("reports category-ranking", "GET", "/reports/category-ranking", { query: ["kind", "start_date", "end_date"], description: "Рейтинг категорий" }),
  command("reports item-spending", "GET", "/reports/item-spending", {
    query: ["start_date", "end_date", "search", "account_id", "limit"],
    description: "Расходы по товарам из позиций чеков",
  }),
  command("insights alerts", "GET", "/insights/alerts", { description: "Финансовые предупреждения" }),
  command("advice get", "GET", "/advice", { description: "Финансовые рекомендации" }),
  command("settings get", "GET", "/settings", { description: "Получить настройки" }),
  command("settings update", "PATCH", "/settings", { body: true, description: "Изменить настройки" }),
  command("backup export", "GET", "/backup/export", { description: "Экспортировать полный бэкап" }),
  command("backup import", "POST", "/backup/import", { body: true, description: "Восстановить полный бэкап" }),
];

export function pathParameters(path) {
  return [...path.matchAll(/\{([^}]+)\}/g)].map((match) => match[1]);
}

export function findCommand(positionals) {
  const candidates = COMMANDS
    .filter(({ name }) => {
      const words = name.split(" ");
      return words.every((word, index) => positionals[index] === word);
    })
    .sort((left, right) => right.name.split(" ").length - left.name.split(" ").length);
  return candidates[0] ?? null;
}
