import { readFile } from "node:fs/promises";
import { parseScalar, splitKeyValue } from "./args.js";
import { pathParameters } from "./commands.js";
import { resolveApiBaseUrl } from "./config.js";

function valuesOf(value) {
  return value === undefined ? [] : Array.isArray(value) ? value : [value];
}

async function jsonFromFile(path) {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch (error) {
    throw new Error(`Не удалось прочитать JSON из ${path}: ${error.message}`);
  }
}

function appendQuery(searchParams, name, value) {
  for (const item of valuesOf(value)) {
    if (item === null || item === undefined) continue;
    searchParams.append(name, typeof item === "object" ? JSON.stringify(item) : String(item));
  }
}

export async function buildRequest(command, positionals, flags, globals) {
  const commandWords = command.name.split(" ").length;
  const argumentsAfterCommand = positionals.slice(commandWords);
  let path = command.path;
  const pathParams = pathParameters(path);
  if (argumentsAfterCommand.length < pathParams.length) {
    throw new Error(`Команда ${command.name} ожидает: ${pathParams.map((name) => `<${name}>`).join(" ")}`);
  }
  pathParams.forEach((name, index) => {
    path = path.replace(`{${name}}`, encodeURIComponent(argumentsAfterCommand[index]));
  });
  if (argumentsAfterCommand.length > pathParams.length) {
    throw new Error(`Лишние аргументы: ${argumentsAfterCommand.slice(pathParams.length).join(" ")}`);
  }

  const base = await resolveApiBaseUrl(globals);
  const url = new URL(`${base}${path.startsWith("/") ? path : `/${path}`}`);
  const bodyFields = {};
  for (const [name, value] of Object.entries(flags)) {
    if (["data", "file", "field", "query"].includes(name)) continue;
    if (!command.body || command.query.includes(name)) appendQuery(url.searchParams, name, value);
    else bodyFields[name] = value;
  }
  for (const value of valuesOf(flags.query)) {
    const [name, parsed] = splitKeyValue(value, "--query");
    appendQuery(url.searchParams, name, parsed);
  }

  let body;
  if (command.body) {
    if (flags.data !== undefined && flags.file !== undefined) {
      throw new Error("Используйте только один из флагов --data и --file");
    }
    if (Array.isArray(flags.data) || Array.isArray(flags.file)) {
      throw new Error("Флаги --data и --file нельзя повторять");
    }
    let document = {};
    if (flags.file !== undefined) document = await jsonFromFile(String(flags.file));
    if (flags.data !== undefined) {
      try {
        document = JSON.parse(String(flags.data));
      } catch (error) {
        throw new Error(`Некорректный JSON в --data: ${error.message}`);
      }
    }
    if (document === null || Array.isArray(document) || typeof document !== "object") {
      throw new Error("Тело запроса должно быть JSON-объектом");
    }
    for (const value of valuesOf(flags.field)) {
      const [name, parsed] = splitKeyValue(value, "--field");
      bodyFields[name] = parsed;
    }
    body = { ...document, ...bodyFields };
  } else if (flags.data !== undefined || flags.file !== undefined || flags.field !== undefined) {
    throw new Error(`Команда ${command.name} не принимает тело запроса`);
  }

  return { method: command.method, url, body };
}

export async function buildGenericRequest(positionals, flags, globals) {
  const [, methodValue, path] = positionals;
  if (!methodValue || !path) throw new Error("Использование: aurum request <METHOD> <PATH> [флаги]");
  const method = methodValue.toUpperCase();
  if (!/^[A-Z]+$/.test(method)) throw new Error(`Некорректный HTTP-метод: ${methodValue}`);
  const command = {
    name: "request",
    method,
    path,
    query: [],
    body: !["GET", "HEAD", "DELETE"].includes(method),
  };
  return buildRequest(command, ["request"], flags, globals);
}

async function readPassword(globals) {
  const passwordFile = globals.password_file ?? process.env.AURUM_PASSWORD_FILE;
  if (passwordFile) return (await readFile(String(passwordFile), "utf8")).replace(/[\r\n]+$/, "");
  return globals.password ?? process.env.AURUM_PASSWORD;
}

export async function executeRequest(request, globals, fetchImplementation = fetch) {
  const headers = { Accept: "application/json" };
  if (request.body !== undefined) headers["Content-Type"] = "application/json";

  for (const entry of valuesOf(globals.header)) {
    const separator = String(entry).indexOf(":");
    if (separator < 1) throw new Error("--header ожидает значение в формате 'Name: value'");
    headers[String(entry).slice(0, separator).trim()] = String(entry).slice(separator + 1).trim();
  }

  const user = globals.user ?? process.env.AURUM_USER;
  const password = await readPassword(globals);
  if ((user && password === undefined) || (!user && password !== undefined)) {
    throw new Error("Для Basic Auth нужны и имя пользователя, и пароль");
  }
  if (user) headers.Authorization = `Basic ${Buffer.from(`${user}:${password}`, "utf8").toString("base64")}`;

  const timeout = Number(globals.timeout ?? process.env.AURUM_TIMEOUT ?? 30_000);
  if (!Number.isFinite(timeout) || timeout <= 0) throw new Error("--timeout должен быть положительным числом миллисекунд");

  const response = await fetchImplementation(request.url, {
    method: request.method,
    headers,
    body: request.body === undefined ? undefined : JSON.stringify(request.body),
    signal: AbortSignal.timeout(timeout),
  });
  const text = await response.text();
  let payload = text;
  if (text) {
    try {
      payload = JSON.parse(text);
    } catch {
      // Preserve non-JSON proxy errors: they are more useful than a parsing failure.
    }
  } else {
    payload = { ok: true, status: response.status };
  }
  if (!response.ok) {
    const detail = typeof payload === "object" ? JSON.stringify(payload, null, 2) : payload;
    throw new Error(`Aurum API вернул ${response.status} ${response.statusText}\n${detail}`);
  }
  return payload;
}

export function parseGenericField(value) {
  return parseScalar(value);
}
