import { chmod, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

export function configPath(environment = process.env) {
  if (environment.AURUM_CONFIG_FILE) return environment.AURUM_CONFIG_FILE;
  const configHome = environment.XDG_CONFIG_HOME || join(homedir(), ".config");
  return join(configHome, "aurum", "config.json");
}

export async function readConfig(path = configPath()) {
  try {
    const parsed = JSON.parse(await readFile(path, "utf8"));
    if (parsed === null || Array.isArray(parsed) || typeof parsed !== "object") {
      throw new Error("корневое значение должно быть JSON-объектом");
    }
    return parsed;
  } catch (error) {
    if (error.code === "ENOENT") return {};
    throw new Error(`Не удалось прочитать конфигурацию ${path}: ${error.message}`);
  }
}

export function normalizeApiBaseUrl(value) {
  let url;
  try {
    url = new URL(String(value));
  } catch {
    throw new Error(`Некорректный api-base-url: ${value}`);
  }
  if (!["http:", "https:"].includes(url.protocol)) {
    throw new Error("api-base-url должен использовать http:// или https://");
  }
  if (url.username || url.password) {
    throw new Error("Не помещайте логин или пароль в api-base-url; используйте настройки Basic Auth");
  }
  url.hash = "";
  url.search = "";
  return url.href.replace(/\/+$/, "");
}

export async function writeConfig(config, path = configPath()) {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  const temporaryPath = `${path}.${process.pid}.tmp`;
  await writeFile(temporaryPath, `${JSON.stringify(config, null, 2)}\n`, { mode: 0o600 });
  await rename(temporaryPath, path);
  await chmod(path, 0o600);
}

export async function resolveApiBaseUrl(globals, environment = process.env) {
  const explicit = globals.api_base_url
    ?? globals.url
    ?? environment.AURUM_API_BASE_URL
    ?? environment.AURUM_URL;
  if (explicit) return normalizeApiBaseUrl(explicit);
  const config = await readConfig(configPath(environment));
  return normalizeApiBaseUrl(config.api_base_url ?? "http://127.0.0.1:3000/api");
}

export async function runConfig(positionals, environment = process.env) {
  const action = positionals[1];
  const key = positionals[2];
  const path = configPath(environment);
  if (action === "path" && key === undefined) return path;
  if (!["get", "set", "unset"].includes(action) || key !== "api-base-url") {
    throw new Error("Использование: aurum config <get|set|unset> api-base-url [URL]\n              aurum config path");
  }

  const config = await readConfig(path);
  if (action === "get") {
    if (positionals.length > 3) throw new Error("Команда config get не принимает значение");
    return config.api_base_url ?? "http://127.0.0.1:3000/api";
  }
  if (action === "unset") {
    if (positionals.length > 3) throw new Error("Команда config unset не принимает значение");
    delete config.api_base_url;
    await writeConfig(config, path);
    return { api_base_url: "http://127.0.0.1:3000/api", source: "default", path };
  }

  const value = positionals[3];
  if (!value || positionals.length > 4) {
    throw new Error("Использование: aurum config set api-base-url <URL>");
  }
  config.api_base_url = normalizeApiBaseUrl(value);
  await writeConfig(config, path);
  return { api_base_url: config.api_base_url, source: "config", path };
}
