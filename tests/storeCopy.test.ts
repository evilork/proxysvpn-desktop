// tests/storeCopy.test.ts
//
// node --test tests/storeCopy.test.ts (Node >= 23.6 strips the types).

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  errorBodyKey,
  errorTitleKey,
  hasStoreCopy,
  mentionsPayment,
  shownServerText,
  storeSafeAction,
  storeSafeActions,
} from "../src/storeCopy.ts";

test("payment words are caught in both languages", () => {
  for (const text of [
    "Пополните баланс - proxysvpn.com",
    "Закончились средства. Пополнить можно в боте",
    "Оплатите доступ",
    "Доступ оплачен до 1 октября",
    "Продлите подписку",
    "Цена снижена до 99 ₽",
    "Скидка 20% по промокоду",
    "Купите PRO",
    "Напишите в бот",
    "Откройте личный кабинет",
    "Top up your balance",
    "top-up now",
    "Renew your plan",
    "Pay with a card",
    "New prices from Monday",
    "Buy PRO",
    "Only $3",
    "Write to @proxysvpn_bot",
    "https://t.me/proxysvpn_bot",
    "Open the dashboard",
    // Verbatim from the service (frontend route.ts, lib/sub-reserve-notice.ts).
    "⚠️ Нет средств",
    "⚠️ No funds",
    "Ваш баланс на нуле. Пополните на proxysvpn.com или в боте, чтобы продолжить пользоваться сервисом.",
    "Баланс на нуле. Если сайт proxysvpn.com не открывается, это фильтр у провайдера. Помощь: @proxysvpn_bot.",
    "Your balance is zero. If proxysvpn.com does not open, your provider is filtering it. Help: @proxysvpn_bot.",
    "На балансе 120 ₽, этого хватит до 3 октября, 12:00 МСК.",
    "3 октября продление PRO за 169 ₽, а на балансе к этому дню будет 20 ₽.",
    "PRO renews on 3 October for 169 ₽, and your balance by then will be 20 ₽.",
    "Пополнить: proxysvpn.com/login",
    "Ссылка уже привязана к другому устройству. Откройте бот → «📡 Мои устройства» → это устройство → «🔄 Сбросить привязку», затем обновите подписку в приложении.",
    "Сбросьте привязку в боте",
    "Эта ссылка больше не работает: устройство удалено или ссылка устарела. Возьмите свежую ссылку в боте («📡 Мои устройства») или в кабинете и добавьте её в приложение заново.",
    "Возьмите новую ссылку в боте",
  ]) {
    assert.equal(mentionsPayment(text), true, text);
  }
});

test("ordinary notices are not mistaken for sales lines", () => {
  for (const text of [
    "Идут работы на серверах в Германии — переключили всех на Нидерланды.",
    "Оценка времени восстановления — 20 минут",
    "Обновили лицензии и процент потерь снизился",
    "Работаем над ускорением Британии",
    "Maintenance in Germany, everyone moved to the Netherlands.",
    "The display of locations was fixed",
    "Both nodes are back",
    "Профиль временно приостановлен. Напишите в поддержку.",
    "proxysvpn.com",
    "",
  ]) {
    assert.equal(mentionsPayment(text), false, text);
  }
});

test("server text is shown unchanged in direct builds", () => {
  assert.equal(shownServerText("Пополните баланс", false), "Пополните баланс");
  assert.equal(shownServerText("Идут работы", false), "Идут работы");
  assert.equal(shownServerText(undefined, false), undefined);
  assert.equal(shownServerText("", false), undefined);
  assert.equal(shownServerText(null, false), undefined);
});

test("App Store builds drop server text that mentions money", () => {
  assert.equal(shownServerText("Пополните баланс - proxysvpn.com", true), undefined);
  assert.equal(shownServerText("Renew now", true), undefined);
  assert.equal(shownServerText("Идут работы", true), "Идут работы");
  assert.equal(shownServerText(undefined, true), undefined);
});

test("actions that leave for the cabinet become retry", () => {
  assert.equal(storeSafeAction("topUp"), "retry");
  assert.equal(storeSafeAction("openCabinet"), "retry");
  for (const action of ["addLink", "retry", "diagnose", "waitAndSee", "contactSupport"] as const) {
    assert.equal(storeSafeAction(action), action);
  }
});

test("the mapped action table keeps every code and no cabinet action", () => {
  const direct = {
    BALANCE_EMPTY: "topUp",
    EXPIRED: "topUp",
    DEVICE_TAKEN: "openCabinet",
    NO_SUBSCRIPTION: "addLink",
    UNKNOWN: "contactSupport",
  } as const;
  // A subset is enough: the function is a pure value map.
  const mapped = storeSafeActions(direct as unknown as Parameters<typeof storeSafeActions>[0]);
  assert.deepEqual(Object.keys(mapped).sort(), Object.keys(direct).sort());
  assert.deepEqual(mapped, {
    BALANCE_EMPTY: "retry",
    EXPIRED: "retry",
    DEVICE_TAKEN: "retry",
    NO_SUBSCRIPTION: "addLink",
    UNKNOWN: "contactSupport",
  });
  // The input is not modified.
  assert.equal(direct.BALANCE_EMPTY, "topUp");
});

test("App Store builds use their own wording for the money codes only", () => {
  assert.equal(errorTitleKey("EXPIRED", true), "err.EXPIRED.appstore.title");
  assert.equal(errorBodyKey("BALANCE_EMPTY", true), "err.BALANCE_EMPTY.appstore.body");
  assert.equal(errorTitleKey("EXPIRED", false), "err.EXPIRED.title");
  assert.equal(errorBodyKey("BALANCE_EMPTY", false), "err.BALANCE_EMPTY.body");
  assert.equal(errorTitleKey("NO_ROUTE", true), "err.NO_ROUTE.title");
  assert.equal(hasStoreCopy("DEVICE_TAKEN"), true);
  assert.equal(hasStoreCopy("TUN_FAILED"), false);
});
