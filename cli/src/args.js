const GLOBAL_VALUE_FLAGS = new Set([
  "url",
  "api_base_url",
  "user",
  "password",
  "password_file",
  "output",
  "timeout",
  "header",
]);
const GLOBAL_BOOLEAN_FLAGS = new Set(["help", "version", "json", "compact"]);
const RAW_VALUE_FLAGS = new Set(["data", "file", "field", "query"]);

export function flagName(value) {
  return value.replace(/^--/, "").replaceAll("-", "_");
}

function looksLikeFlag(value) {
  return value.startsWith("--") && !/^--?-?\d/.test(value);
}

export function parseScalar(value) {
  if (value === "true") return true;
  if (value === "false") return false;
  if (value === "null") return null;
  if (/^-?(?:0|[1-9]\d*)(?:\.\d+)?$/.test(value)) return Number(value);
  if ((value.startsWith("[") && value.endsWith("]")) || (value.startsWith("{") && value.endsWith("}"))) {
    try {
      return JSON.parse(value);
    } catch {
      throw new Error(`Некорректный JSON во флаге: ${value}`);
    }
  }
  return value;
}

function assignFlag(target, name, value) {
  if (Object.hasOwn(target, name)) {
    target[name] = Array.isArray(target[name]) ? [...target[name], value] : [target[name], value];
  } else {
    target[name] = value;
  }
}

export function parseArguments(argv) {
  const positionals = [];
  const flags = {};
  const globals = {};
  let passthrough = false;

  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === "--") {
      passthrough = true;
      continue;
    }
    if (passthrough || !looksLikeFlag(token)) {
      positionals.push(token);
      continue;
    }

    const equalsAt = token.indexOf("=");
    const rawName = equalsAt === -1 ? token : token.slice(0, equalsAt);
    let name = flagName(rawName);
    if (name.startsWith("no_")) {
      assignFlag(flags, name.slice(3), false);
      continue;
    }
    if (GLOBAL_BOOLEAN_FLAGS.has(name)) {
      globals[name] = true;
      continue;
    }

    let value = equalsAt === -1 ? undefined : token.slice(equalsAt + 1);
    if (value === undefined) {
      const next = argv[index + 1];
      if (next === undefined || looksLikeFlag(next)) {
        value = true;
      } else {
        value = next;
        index += 1;
      }
    }

    if (GLOBAL_VALUE_FLAGS.has(name)) {
      assignFlag(globals, name, value);
    } else if (RAW_VALUE_FLAGS.has(name)) {
      assignFlag(flags, name, value);
    } else {
      assignFlag(flags, name, parseScalar(String(value)));
    }
  }

  return { positionals, flags, globals };
}

export function splitKeyValue(value, flag) {
  const separator = String(value).indexOf("=");
  if (separator < 1) throw new Error(`${flag} ожидает значение в формате key=value`);
  return [String(value).slice(0, separator).replaceAll("-", "_"), parseScalar(String(value).slice(separator + 1))];
}
