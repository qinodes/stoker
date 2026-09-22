import assert from "node:assert/strict";
import { test } from "node:test";
import { dictionaries, isLocale, resolveLocale, translate, translateMessage, translateState, uiMessage } from "../../src/i18n/messages.ts";
import { formatDate } from "../../src/formatters.ts";
import { formatPolicyValue } from "../../src/policy.ts";

test("saved language takes priority and unsupported settings fall back to browser preferences", () => {
  assert.equal(resolveLocale("zh-TW", ["ja-JP"]), "zh-TW");
  assert.equal(resolveLocale("ja", ["en-US"]), "ja");
  assert.equal(resolveLocale("en", ["zh-TW"]), "en");
  assert.equal(resolveLocale("invalid", ["ja-JP"]), "ja");
  assert.equal(resolveLocale(null, ["fr-FR", "zh-Hant-TW"]), "zh-TW");
  assert.equal(resolveLocale(null, ["en-US", "ja-JP"]), "en");
  assert.equal(resolveLocale(null, []), "en");
  assert.equal(resolveLocale(null, ["fr-FR"]), "en");
  assert.equal(isLocale("zh-CN"), false);
});

test("traditional Chinese detection respects script and region without matching Simplified Chinese", () => {
  for (const tag of ["zh-TW", "zh-Hant", "zh-Hant-HK", "zh-HK", "zh-MO", "ZH_tw"]) assert.equal(resolveLocale(null, [tag]), "zh-TW", tag);
  for (const tag of ["zh", "zh-CN", "zh-Hans", "zh-Hans-TW"]) assert.equal(resolveLocale(null, [tag]), "en", tag);
  assert.equal(resolveLocale(null, ["zh-CN", "ja"]), "ja");
});

test("all dictionaries have complete translations and matching interpolation parameters", () => {
  const placeholders = (value: string) => [...value.matchAll(/\{([^}]+)\}/g)].map(match => match[1]).sort();
  for (const [locale, dictionary] of Object.entries(dictionaries)) {
    assert.deepEqual(Object.keys(dictionary).sort(), Object.keys(dictionaries.en).sort(), locale);
    for (const key of Object.keys(dictionaries.en) as Array<keyof typeof dictionaries.en>) {
      assert.ok(dictionary[key].trim(), `${locale}: ${key}`);
      assert.deepEqual(placeholders(dictionary[key]), placeholders(dictionaries.en[key]), `${locale}: ${key}`);
    }
  }
});

test("localized sentences preserve literal data and English singular and plural wording", () => {
  assert.equal(translate("en", "jobs.visibleOne", { count: 1 }), "1 visible job");
  assert.equal(translate("en", "jobs.visibleMany", { count: 0 }), "0 visible jobs");
  assert.equal(translate("zh-TW", "jobs.visibleMany", { count: 2 }), "顯示 2 項工作");
  assert.equal(translate("ja", "pagination.page", { page: 2, total: 3 }), "3 ページ中 2 ページ");
  assert.equal(translate("zh-TW", "jobs.openDetails", { name: "$& {name} 日本語" }), "開啟「$& {name} 日本語」的詳細資訊");
});

test("known states translate while new states and backend messages retain original values", () => {
  assert.equal(translateState("ja", "DRAFT"), "下書き");
  assert.equal(translateState("zh-TW", "RUNNING"), "執行中");
  assert.equal(translateState("zh-TW", "RECOVERING"), "復原中");
  assert.equal(translateState("ja", null), "不明");
  for (const raw of ["NEW_STATE", "toString", "__proto__"]) assert.equal(translateState("ja", raw), raw);
  assert.equal(translateMessage("zh-TW", "job not found"), "job not found");
  assert.equal(translateMessage("ja", uiMessage("toast.created")), "下書きのジョブを作成しました。");
});

test("a missing translation falls back to English", () => {
  const original = dictionaries.ja["common.save"];
  try {
    dictionaries.ja["common.save"] = "";
    assert.equal(translate("ja", "common.save"), "Save");
  } finally {
    dictionaries.ja["common.save"] = original;
  }
});

test("locale changes formatting without changing the workspace timezone or policy values", () => {
  const value = "2026-01-01T00:00:00Z";
  const options: Intl.DateTimeFormatOptions = { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", timeZone: "Asia/Tokyo" };
  for (const locale of ["en", "zh-TW", "ja"] as const) {
    assert.equal(formatDate(value, "Asia/Tokyo", locale), new Intl.DateTimeFormat(locale, options).format(new Date(value)));
    assert.equal(formatPolicyValue(12345, "MB", locale), `${new Intl.NumberFormat(locale).format(12345)} MB`);
  }
  assert.equal(formatPolicyValue(null, "毫秒", "zh-TW", "停用"), "停用");
  assert.equal(formatPolicyValue(0, "件", "ja", "無効"), "0 件");
});
