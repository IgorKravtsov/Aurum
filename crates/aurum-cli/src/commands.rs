//! Command registry, a port of `cli/src/commands.js`.

/// One CLI command mapped onto one REST endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// Words the user types, e.g. `accounts update`.
    pub name: &'static str,
    pub method: &'static str,
    /// Endpoint path relative to the API base URL; `{param}` segments are
    /// filled from the positional arguments after the command name.
    pub path: &'static str,
    /// Flags that go to the query string even on commands with a body.
    pub query: &'static [&'static str],
    pub body: bool,
    pub description: &'static str,
}

// This registry is intentionally declarative: adding a backend endpoint only
// requires one entry here, and the request builder, help, and skill reference
// all continue to use the same command model.
pub const COMMANDS: &[Command] = &[
    Command {
        name: "health",
        method: "GET",
        path: "/health",
        query: &[],
        body: false,
        description: "Проверить доступность API",
    },
    Command {
        name: "accounts list",
        method: "GET",
        path: "/accounts",
        query: &["include_archived"],
        body: false,
        description: "Список счетов",
    },
    Command {
        name: "accounts create",
        method: "POST",
        path: "/accounts",
        query: &[],
        body: true,
        description: "Создать счёт",
    },
    Command {
        name: "accounts update",
        method: "PATCH",
        path: "/accounts/{account_id}",
        query: &[],
        body: true,
        description: "Изменить счёт",
    },
    Command {
        name: "accounts delete",
        method: "DELETE",
        path: "/accounts/{account_id}",
        query: &[],
        body: false,
        description: "Удалить счёт и его операции",
    },
    Command {
        name: "categories list",
        method: "GET",
        path: "/categories",
        query: &["kind"],
        body: false,
        description: "Список категорий",
    },
    Command {
        name: "categories create",
        method: "POST",
        path: "/categories",
        query: &[],
        body: true,
        description: "Создать категорию",
    },
    Command {
        name: "categories update",
        method: "PATCH",
        path: "/categories/{category_id}",
        query: &[],
        body: true,
        description: "Изменить категорию",
    },
    Command {
        name: "categories delete",
        method: "DELETE",
        path: "/categories/{category_id}",
        query: &[],
        body: false,
        description: "Удалить категорию",
    },
    Command {
        name: "tags list", method: "GET", path: "/tags", query: &[], body: false, description: "Список тегов"
    },
    Command {
        name: "tags create", method: "POST", path: "/tags", query: &[], body: true, description: "Создать тег"
    },
    Command {
        name: "tags delete",
        method: "DELETE",
        path: "/tags/{tag_id}",
        query: &[],
        body: false,
        description: "Удалить тег",
    },
    Command {
        name: "transactions list",
        method: "GET",
        path: "/transactions",
        query: &[
            "year",
            "month",
            "start_date",
            "end_date",
            "account_id",
            "category_id",
            "tag_id",
            "type",
            "search",
            "sort",
            "page",
            "page_size",
        ],
        body: false,
        description: "Список и поиск операций",
    },
    Command {
        name: "transactions years",
        method: "GET",
        path: "/transactions/years",
        query: &[],
        body: false,
        description: "Годы с операциями",
    },
    Command {
        name: "transactions create",
        method: "POST",
        path: "/transactions",
        query: &[],
        body: true,
        description: "Создать операцию",
    },
    Command {
        name: "transactions bulk",
        method: "POST",
        path: "/transactions/bulk",
        query: &[],
        body: true,
        description: "Импортировать до 5000 операций",
    },
    Command {
        name: "transactions update",
        method: "PATCH",
        path: "/transactions/{transaction_id}",
        query: &[],
        body: true,
        description: "Изменить операцию",
    },
    Command {
        name: "transactions delete",
        method: "DELETE",
        path: "/transactions/{transaction_id}",
        query: &[],
        body: false,
        description: "Удалить операцию",
    },
    Command {
        name: "recurring list",
        method: "GET",
        path: "/recurring",
        query: &[],
        body: false,
        description: "Список регулярных операций",
    },
    Command {
        name: "recurring create",
        method: "POST",
        path: "/recurring",
        query: &[],
        body: true,
        description: "Создать регулярную операцию",
    },
    Command {
        name: "recurring update",
        method: "PATCH",
        path: "/recurring/{recurring_id}",
        query: &[],
        body: true,
        description: "Изменить регулярную операцию",
    },
    Command {
        name: "recurring delete",
        method: "DELETE",
        path: "/recurring/{recurring_id}",
        query: &[],
        body: false,
        description: "Удалить регулярную операцию",
    },
    Command {
        name: "recurring post",
        method: "POST",
        path: "/recurring/{recurring_id}/post",
        query: &[],
        body: false,
        description: "Провести регулярную операцию сейчас",
    },
    Command {
        name: "budgets list",
        method: "GET",
        path: "/budgets",
        query: &[],
        body: false,
        description: "Список бюджетов",
    },
    Command {
        name: "budgets status",
        method: "GET",
        path: "/budgets/status",
        query: &["year", "month"],
        body: false,
        description: "Исполнение бюджетов",
    },
    Command {
        name: "budgets create",
        method: "POST",
        path: "/budgets",
        query: &[],
        body: true,
        description: "Создать бюджет",
    },
    Command {
        name: "budgets update",
        method: "PATCH",
        path: "/budgets/{budget_id}",
        query: &[],
        body: true,
        description: "Изменить бюджет",
    },
    Command {
        name: "budgets delete",
        method: "DELETE",
        path: "/budgets/{budget_id}",
        query: &[],
        body: false,
        description: "Удалить бюджет",
    },
    Command {
        name: "goals list", method: "GET", path: "/goals", query: &[], body: false, description: "Список целей"
    },
    Command {
        name: "goals create",
        method: "POST",
        path: "/goals",
        query: &[],
        body: true,
        description: "Создать цель",
    },
    Command {
        name: "goals update",
        method: "PATCH",
        path: "/goals/{goal_id}",
        query: &[],
        body: true,
        description: "Изменить цель",
    },
    Command {
        name: "goals delete",
        method: "DELETE",
        path: "/goals/{goal_id}",
        query: &[],
        body: false,
        description: "Удалить цель",
    },
    Command {
        name: "goals contribute",
        method: "POST",
        path: "/goals/{goal_id}/contributions",
        query: &[],
        body: true,
        description: "Добавить взнос или снятие",
    },
    Command {
        name: "assets list",
        method: "GET",
        path: "/assets",
        query: &[],
        body: false,
        description: "Список активов",
    },
    Command {
        name: "assets create",
        method: "POST",
        path: "/assets",
        query: &[],
        body: true,
        description: "Создать актив",
    },
    Command {
        name: "assets update",
        method: "PATCH",
        path: "/assets/{asset_id}",
        query: &[],
        body: true,
        description: "Изменить актив",
    },
    Command {
        name: "assets delete",
        method: "DELETE",
        path: "/assets/{asset_id}",
        query: &[],
        body: false,
        description: "Удалить актив",
    },
    Command {
        name: "assets valuations",
        method: "GET",
        path: "/assets/{asset_id}/valuations",
        query: &[],
        body: false,
        description: "История оценок актива",
    },
    Command {
        name: "assets value",
        method: "POST",
        path: "/assets/{asset_id}/valuations",
        query: &[],
        body: true,
        description: "Записать оценку актива",
    },
    Command {
        name: "crypto portfolios",
        method: "GET",
        path: "/crypto/portfolios",
        query: &["include_archived"],
        body: false,
        description: "Список криптопортфелей",
    },
    Command {
        name: "crypto portfolio-create",
        method: "POST",
        path: "/crypto/portfolios",
        query: &[],
        body: true,
        description: "Создать криптопортфель",
    },
    Command {
        name: "crypto portfolio-update",
        method: "PATCH",
        path: "/crypto/portfolios/{portfolio_id}",
        query: &[],
        body: true,
        description: "Изменить криптопортфель",
    },
    Command {
        name: "crypto portfolio-delete",
        method: "DELETE",
        path: "/crypto/portfolios/{portfolio_id}",
        query: &[],
        body: false,
        description: "Удалить криптопортфель",
    },
    Command {
        name: "crypto holdings",
        method: "GET",
        path: "/crypto/holdings",
        query: &["portfolio_id"],
        body: false,
        description: "Криптоактивы и цены",
    },
    Command {
        name: "crypto refresh",
        method: "POST",
        path: "/crypto/refresh",
        query: &["portfolio_id"],
        body: false,
        description: "Принудительно обновить цены",
    },
    Command {
        name: "crypto holding-create",
        method: "POST",
        path: "/crypto/holdings",
        query: &[],
        body: true,
        description: "Добавить криптоактив",
    },
    Command {
        name: "crypto holding-update",
        method: "PATCH",
        path: "/crypto/holdings/{asset_id}",
        query: &[],
        body: true,
        description: "Изменить метаданные криптоактива",
    },
    Command {
        name: "crypto transactions",
        method: "GET",
        path: "/crypto/holdings/{asset_id}/transactions",
        query: &[],
        body: false,
        description: "Операции криптоактива",
    },
    Command {
        name: "crypto transaction-add",
        method: "POST",
        path: "/crypto/holdings/{asset_id}/transactions",
        query: &[],
        body: true,
        description: "Добавить покупку или продажу",
    },
    Command {
        name: "crypto transaction-update",
        method: "PATCH",
        path: "/crypto/transactions/{transaction_id}",
        query: &[],
        body: true,
        description: "Изменить криптооперацию",
    },
    Command {
        name: "crypto transaction-delete",
        method: "DELETE",
        path: "/crypto/transactions/{transaction_id}",
        query: &[],
        body: false,
        description: "Удалить криптооперацию",
    },
    Command {
        name: "crypto search",
        method: "GET",
        path: "/crypto/search",
        query: &["q"],
        body: false,
        description: "Найти монету в CoinGecko",
    },
    Command {
        name: "crypto history",
        method: "GET",
        path: "/crypto/history",
        query: &["range", "portfolio_id"],
        body: false,
        description: "История стоимости криптопортфеля",
    },
    Command {
        name: "crypto performance",
        method: "GET",
        path: "/crypto/performance/90d",
        query: &["portfolio_id"],
        body: false,
        description: "Доходность монет за 90 дней",
    },
    Command {
        name: "dashboard summary",
        method: "GET",
        path: "/dashboard/summary",
        query: &["year", "month"],
        body: false,
        description: "Сводка панели",
    },
    Command {
        name: "cash-flow get",
        method: "GET",
        path: "/cash-flow",
        query: &["start_date", "end_date"],
        body: false,
        description: "Денежный поток",
    },
    Command {
        name: "net-worth summary",
        method: "GET",
        path: "/net-worth/summary",
        query: &["range"],
        body: false,
        description: "Капитал и его история",
    },
    Command {
        name: "reports category-spending",
        method: "GET",
        path: "/reports/category-spending",
        query: &["category_id", "start_date", "end_date"],
        body: false,
        description: "Расходы выбранной категории",
    },
    Command {
        name: "reports category-ranking",
        method: "GET",
        path: "/reports/category-ranking",
        query: &["kind", "start_date", "end_date"],
        body: false,
        description: "Рейтинг категорий",
    },
    Command {
        name: "insights alerts",
        method: "GET",
        path: "/insights/alerts",
        query: &[],
        body: false,
        description: "Финансовые предупреждения",
    },
    Command {
        name: "advice get",
        method: "GET",
        path: "/advice",
        query: &[],
        body: false,
        description: "Финансовые рекомендации",
    },
    Command {
        name: "settings get",
        method: "GET",
        path: "/settings",
        query: &[],
        body: false,
        description: "Получить настройки",
    },
    Command {
        name: "settings update",
        method: "PATCH",
        path: "/settings",
        query: &[],
        body: true,
        description: "Изменить настройки",
    },
    Command {
        name: "backup export",
        method: "GET",
        path: "/backup/export",
        query: &[],
        body: false,
        description: "Экспортировать полный бэкап",
    },
    Command {
        name: "backup import",
        method: "POST",
        path: "/backup/import",
        query: &[],
        body: true,
        description: "Восстановить полный бэкап",
    },
];

/// `{name}` placeholders of a path, in order.
pub fn path_parameters(path: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find('}') {
            // `{}` has no name and is not a parameter; scan on after the brace.
            Some(close) if close > 0 => {
                names.push(after[..close].to_string());
                rest = &after[close + 1..];
            }
            _ => rest = after,
        }
    }
    names
}

/// Longest registered command whose words are a prefix of `positionals`.
pub fn find_command(positionals: &[String]) -> Option<&'static Command> {
    COMMANDS
        .iter()
        .filter(|command| {
            command
                .name
                .split(' ')
                .enumerate()
                .all(|(index, word)| positionals.get(index).is_some_and(|given| given == word))
        })
        .max_by_key(|command| command.name.split(' ').count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_parameters_lists_placeholders() {
        assert_eq!(path_parameters("/crypto/holdings/{asset_id}/transactions"), ["asset_id"]);
        assert_eq!(path_parameters("/accounts"), Vec::<String>::new());
        assert_eq!(path_parameters("/a/{}/{x}"), ["x"]);
    }

    #[test]
    fn find_command_prefers_the_longest_match() {
        let words = |text: &str| text.split(' ').map(str::to_string).collect::<Vec<_>>();
        assert_eq!(find_command(&words("crypto holding-update 17")).unwrap().path, "/crypto/holdings/{asset_id}");
        assert_eq!(find_command(&words("health")).unwrap().name, "health");
        assert!(find_command(&words("accounts")).is_none());
        assert!(find_command(&words("nope")).is_none());
    }

    #[test]
    fn registry_has_no_duplicate_names_or_endpoints() {
        let mut names: Vec<_> = COMMANDS.iter().map(|command| command.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), COMMANDS.len());
        let mut endpoints: Vec<_> = COMMANDS.iter().map(|command| (command.method, command.path)).collect();
        endpoints.sort_unstable();
        endpoints.dedup();
        assert_eq!(endpoints.len(), COMMANDS.len(), "two commands share one endpoint");
    }
}
