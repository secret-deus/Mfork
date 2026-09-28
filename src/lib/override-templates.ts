export const overrideTemplates = [
  { id: "domain", name: "精确域名", description: "仅匹配 example.com，不匹配其子域名。", rule: "DOMAIN,example.com,DIRECT" },
  { id: "suffix", name: "域名后缀", description: "匹配 example.com 及其所有子域名，如 www.example.com。", rule: "DOMAIN-SUFFIX,example.com,DIRECT" },
  { id: "prefix", name: "域名前缀", description: "用正则匹配以 api. 开头的域名；不匹配 myapi.example.com。没有 DOMAIN-PREFIX 规则。", rule: "DOMAIN-REGEX,^api[.],DIRECT" },
  { id: "keyword", name: "域名关键词", description: "匹配域名任意位置包含 example 的情况，不等同于前缀或后缀。", rule: "DOMAIN-KEYWORD,example,DIRECT" },
  { id: "cidr", name: "IP 网段", description: "匹配目标 IPv4 网段；no-resolve 表示不为此规则主动解析域名。", rule: "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve" },
  { id: "process", name: "进程名称", description: "按进程文件名匹配，如 Windows 的 chrome.exe；依赖平台与权限支持。", rule: "PROCESS-NAME,chrome.exe,DIRECT" },
] as const;

export function templateYaml(rule: string): string {
  // JSON double-quoted strings are valid YAML scalars and preserve regex escapes.
  return `prepend-rules:\n  - ${JSON.stringify(rule)}\n`;
}
