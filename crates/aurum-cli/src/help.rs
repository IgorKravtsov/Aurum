//! `--help` text, a port of `help()` in `cli/src/main.js`.

use crate::commands::{COMMANDS, path_parameters};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP_HEAD: &str = r#"Aurum CLI {version}

Использование:
  aurum [глобальные флаги] <команда> [аргументы] [флаги]
  aurum request <METHOD> <PATH> [--query key=value] [--data JSON | --file PATH]
  aurum config set api-base-url <URL>
  aurum skills get core [--full]

Постоянная настройка сервера:
  aurum config set api-base-url https://server.example.com/api
  aurum config get api-base-url
  aurum config unset api-base-url
  aurum config path

Глобальные флаги:
  --api-base-url URL     Разово переопределить базовый URL API
  --url URL              Короткий псевдоним --api-base-url
  --user USER            HTTP Basic Auth user
  --password PASSWORD    HTTP Basic Auth password; безопаснее AURUM_PASSWORD
  --password-file PATH   Прочитать пароль из локального файла
  --header 'Name: value' Добавить HTTP-заголовок; можно повторять
  --timeout MS           Таймаут запроса, по умолчанию 30000
  --output PATH          Записать успешный ответ в файл
  --compact              Печатать JSON без отступов
  --json                 Структурированный вывод для skills
  --help                 Показать справку
  --version              Показать версию

Приоритет URL: --api-base-url → AURUM_API_BASE_URL → локальная конфигурация → http://127.0.0.1:3000/api.

Тело запроса:
  Любой флаг команды становится JSON-полем: --target-amount 5000
  Массивы/объекты передаются JSON: --tag-ids '[1,2]'
  --data '{"name":"Cash"}' или --file ./payload.json
  --field key=value можно повторять и совмещать с --file

Команды:
"#;

const HELP_TAIL: &str = r#"

Полный актуальный справочник для агентов:
  aurum skills get core --full"#;

fn signature(name: &str, path: &str) -> String {
    let parameters: Vec<String> = path_parameters(path).iter().map(|parameter| format!("<{parameter}>")).collect();
    format!("{name} {}", parameters.join(" ")).trim().to_string()
}

pub fn help() -> String {
    let width = COMMANDS.iter().map(|command| signature(command.name, command.path).chars().count()).max().unwrap_or(0);
    let lines: Vec<String> = COMMANDS
        .iter()
        .map(|command| format!("  {:<width$}  {}", signature(command.name, command.path), command.description))
        .collect();
    format!("{}{}{HELP_TAIL}", HELP_HEAD.replace("{version}", VERSION), lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_lists_every_command_with_its_path_parameters() {
        let text = help();
        assert!(text.starts_with(&format!("Aurum CLI {VERSION}\n\nИспользование:\n")));
        assert!(text.contains("  accounts update <account_id>"));
        assert!(text.contains("Проверить доступность API"));
        assert!(text.ends_with("aurum skills get core --full"));
        assert!(text.lines().filter(|line| line.starts_with("  ")).count() >= COMMANDS.len());
    }
}
