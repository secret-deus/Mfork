// Process-name routing rules: pure helpers shared by the Rules page section.
//
// One saved rule becomes a `PROCESS-NAME,<process_name>,<target>` rule in the
// generated config, so the same constraints the backend enforces on
// `process_rules` (src-tauri/src/settings.rs `normalize_process_rules`) are
// checked here to surface the error before persisting. Error strings are
// returned as i18n *source keys* so the UI can translate them; the backend
// keeps its own authoritative validation (e.g. whether `target` is a proxy
// that actually exists), which this file deliberately cannot do offline.

/** Length limits mirroring the backend: name <= 260, target <= 256, <= 256 rules. */
const MAX_RULES = 256;
const MAX_NAME = 260;
const MAX_TARGET = 256;

/**
 * One saved process-name rule. `enabled: false` keeps the entry in the list
 * without injecting it into the generated config.
 */
export interface ProcessRule {
  process_name: string;
  target: string;
  enabled: boolean;
}

/** Validation failure: `key` is an i18n source string, `params` its placeholders. */
export interface ProcessRuleError {
  key: string;
  params?: Record<string, string | number>;
}

export type ProcessRulesResult =
  | { ok: true; rules: ProcessRule[] }
  | { ok: false; error: ProcessRuleError };

/** Characters that would break `PROCESS-NAME,<name>,<target>` or the YAML. */
// eslint-disable-next-line no-control-regex
const BAD_CHAR = /[,\u0000-\u001f\u007f-\u009f]/;

/** Characters a process basename can never contain. */
const BAD_NAME_CHAR = /[/\\*?]/;

function invalid(key: string, params?: ProcessRuleError["params"]): ProcessRulesResult {
  return { ok: false, error: { key, params } };
}

/**
 * Trim + validate rules for persistence. Returns a **new** immutable array of
 * new objects (the input, e.g. `settings.process_rules`, is never mutated) or
 * the first error, reporting the 1-based position the user sees in the list.
 * Order, casing and `enabled` are preserved.
 */
export function normalizeProcessRules(rules: readonly ProcessRule[]): ProcessRulesResult {
  if (rules.length > MAX_RULES) return invalid("进程规则最多 {max} 条", { max: MAX_RULES });
  const out: ProcessRule[] = [];
  const seen: string[] = [];
  for (let i = 0; i < rules.length; i++) {
    const n = i + 1;
    const name = (rules[i].process_name ?? "").trim();
    const target = (rules[i].target ?? "").trim();
    if (!name) return invalid("第 {n} 条进程规则的进程名不能为空", { n });
    if (!target) return invalid("第 {n} 条进程规则的目标不能为空", { n });
    if ([...name].length > MAX_NAME)
      return invalid("第 {n} 条进程规则的进程名超过 {max} 个字符", { n, max: MAX_NAME });
    if ([...target].length > MAX_TARGET)
      return invalid("第 {n} 条进程规则的目标超过 {max} 个字符", { n, max: MAX_TARGET });
    if (BAD_NAME_CHAR.test(name))
      return invalid("第 {n} 条进程规则的进程名「{name}」不能包含路径分隔符或通配符（/ \\ * ?）", { n, name });
    if (BAD_CHAR.test(name))
      return invalid("第 {n} 条进程规则的进程名「{name}」不能包含逗号或控制字符", { n, name });
    if (BAD_CHAR.test(target))
      return invalid("第 {n} 条进程规则的目标「{target}」不能包含逗号或控制字符", { n, target });
    const key = name.toLowerCase();
    if (seen.includes(key))
      return invalid("进程名「{name}」重复（进程名不区分大小写）", { name });
    seen.push(key);
    out.push({ process_name: name, target, enabled: rules[i].enabled !== false });
  }
  return { ok: true, rules: out };
}
