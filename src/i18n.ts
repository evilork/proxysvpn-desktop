// src/i18n.ts
export type Lang = "ru" | "en";

type Dict = Record<string, string>;

const ru: Dict = {
  "status.disconnected": "Не подключено",
  "status.connecting": "Подключение...",
  "status.connected": "Подключено",
  "ping.measuring": "измерение...",
  "ping.label": "Пинг",
  "btn.paste": "Вставить из буфера",
  "btn.qrTitle": "Показать QR подписки",
  "settings.title": "Настройки",
  "settings.theme": "Тема",
  "settings.theme.light": "Светлая",
  "settings.theme.dark": "Тёмная",
  "settings.lang": "Язык",
  "settings.clearSub": "Удалить подписку",
  "settings.server": "Сервер",
  "settings.server.auto": "Авто (первый)",
  "settings.server.loading": "Загрузка серверов...",
  "settings.soon": "Скоро: Hysteria2, IPv4 / IPv6.",
  "qr.title": "QR подписки",
  "qr.soon": "Передача подписки по QR появится скоро. Пока используйте «Вставить из буфера».",
  "err.clipboardEmpty": "В буфере обмена нет ссылки",
  "err.clipboardRead": "Не удалось прочитать буфер обмена",
  "err.qr": "Не удалось сгенерировать QR",
  "err.noSub": "Сначала вставьте ссылку из буфера обмена",
};

const en: Dict = {
  "status.disconnected": "Disconnected",
  "status.connecting": "Connecting...",
  "status.connected": "Connected",
  "ping.measuring": "measuring...",
  "ping.label": "Ping",
  "btn.paste": "Paste from clipboard",
  "btn.qrTitle": "Show subscription QR",
  "settings.title": "Settings",
  "settings.theme": "Theme",
  "settings.theme.light": "Light",
  "settings.theme.dark": "Dark",
  "settings.lang": "Language",
  "settings.clearSub": "Remove subscription",
  "settings.server": "Server",
  "settings.server.auto": "Auto (first)",
  "settings.server.loading": "Loading servers...",
  "settings.soon": "Coming soon: Hysteria2, IPv4 / IPv6.",
  "qr.title": "Subscription QR",
  "qr.soon": "QR subscription transfer is coming soon. For now use \u00abPaste from clipboard\u00bb.",
  "err.clipboardEmpty": "No link in clipboard",
  "err.clipboardRead": "Could not read clipboard",
  "err.qr": "Could not generate QR",
  "err.noSub": "Paste a link from clipboard first",
};

const dicts: Record<Lang, Dict> = { ru, en };

export function makeT(lang: Lang) {
  return (key: string): string => dicts[lang][key] ?? key;
}
