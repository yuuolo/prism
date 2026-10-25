// Interface language. The Chinese text is the key; English comes from
// i18n-en.ts and falls back to the Chinese when a string is missing.
import { EN } from "./i18n-en";

export type Lang = "zh" | "en";

let current: Lang = "zh";

export function setLang(l: Lang) {
  current = l;
}

/** `t("{n} 个节点", { n: 3 })` */
export function t(zh: string, vars?: Record<string, string | number>): string {
  let s = current === "en" ? (EN[zh] ?? zh) : zh;
  if (vars) s = s.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m));
  return s;
}
