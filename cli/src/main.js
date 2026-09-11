import { chmod, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { parseArguments } from "./args.js";
import { buildGenericRequest, buildRequest, executeRequest } from "./client.js";
import { COMMANDS, findCommand, pathParameters } from "./commands.js";
import { runConfig } from "./config.js";
import { runSkills } from "./skills.js";

const { version: VERSION } = createRequire(import.meta.url)("../package.json");

function help() {
  const width = Math.max(...COMMANDS.map(({ name, path }) => `${name} ${pathParameters(path).map((parameter) => `<${parameter}>`).join(" ")}`.trim().length));
  const lines = COMMANDS.map(({ name, path, description }) => {
    const signature = `${name} ${pathParameters(path).map((parameter) => `<${parameter}>`).join(" ")}`.trim();
    return `  ${signature.padEnd(width)}  ${description}`;
  });
  return `Aurum CLI ${VERSION}\n\nИспользование:\n  aurum [глобальные флаги] <команда> [аргументы] [флаги]\n  aurum request <METHOD> <PATH> [--query key=value] [--data JSON | --file PATH]\n  aurum config set api-base-url <URL>\n  aurum skills get core [--full]\n\nПостоянная настройка сервера:\n  aurum config set api-base-url https://server.example.com/api\n  aurum config get api-base-url\n  aurum config unset api-base-url\n  aurum config path\n\nГлобальные флаги:\n  --api-base-url URL     Разово переопределить базовый URL API\n  --url URL              Короткий псевдоним --api-base-url\n  --user USER            HTTP Basic Auth user\n  --password PASSWORD    HTTP Basic Auth password; безопаснее AURUM_PASSWORD\n  --password-file PATH   Прочитать пароль из локального файла\n  --header 'Name: value' Добавить HTTP-заголовок; можно повторять\n  --timeout MS           Таймаут запроса, по умолчанию 30000\n  --output PATH          Записать успешный ответ в файл\n  --compact              Печатать JSON без отступов\n  --json                 Структурированный вывод для skills\n  --help                 Показать справку\n  --version              Показать версию\n\nПриоритет URL: --api-base-url → AURUM_API_BASE_URL → локальная конфигурация → http://127.0.0.1:3000/api.\n\nТело запроса:\n  Любой флаг команды становится JSON-полем: --target-amount 5000\n  Массивы/объекты передаются JSON: --tag-ids '[1,2]'\n  --data '{"name":"Cash"}' или --file ./payload.json\n  --field key=value можно повторять и совмещать с --file\n\nКоманды:\n${lines.join("\n")}\n\nПолный актуальный справочник для агентов:\n  aurum skills get core --full`;
}

function render(value, compact) {
  return typeof value === "string" ? value : JSON.stringify(value, null, compact ? 0 : 2);
}

export async function main(argv, dependencies = {}) {
  const parsed = parseArguments(argv);
  if (parsed.globals.version) {
    console.log(VERSION);
    return;
  }
  if (parsed.globals.help || parsed.positionals.length === 0) {
    console.log(help());
    return;
  }
  if (parsed.positionals[0] === "skills") {
    const output = await runSkills(parsed.positionals, { ...parsed.globals, ...parsed.flags });
    console.log(render(output, parsed.globals.compact));
    return;
  }
  if (parsed.positionals[0] === "config") {
    const output = await runConfig(parsed.positionals);
    console.log(render(output, parsed.globals.compact));
    return;
  }

  let request;
  if (parsed.positionals[0] === "request") {
    request = await buildGenericRequest(parsed.positionals, parsed.flags, parsed.globals);
  } else {
    const command = findCommand(parsed.positionals);
    if (!command) throw new Error(`Неизвестная команда: ${parsed.positionals.join(" ")}\nЗапустите aurum --help`);
    request = await buildRequest(command, parsed.positionals, parsed.flags, parsed.globals);
  }
  const result = await executeRequest(request, parsed.globals, dependencies.fetchImplementation);
  const output = `${render(result, parsed.globals.compact)}\n`;
  if (parsed.globals.output) {
    await writeFile(String(parsed.globals.output), output, { mode: 0o600 });
    await chmod(String(parsed.globals.output), 0o600);
  } else {
    process.stdout.write(output);
  }
}

export { help, VERSION };
