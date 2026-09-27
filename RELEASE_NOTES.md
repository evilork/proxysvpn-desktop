# ProxysVPN Desktop v0.2.0-beta

🇷🇺 Быстрее подключается, работает на всех локациях, включая «Британия · XHTTP».

🇬🇧 Connects faster and works on every location, «Britain · XHTTP» included.

## 🇷🇺 Что нового

- **XHTTP.** Локации на новом транспорте XHTTP («Британия · XHTTP») подключаются. Прежняя версия собирала для них обычное TCP-соединение, и они не работали.
- **Протокол в списке стран.** Под каждой локацией видно, на чём она работает: «VLESS · Vision», «VLESS · XHTTP», «Hysteria2». Имя локации показывается целиком, как его пишет сервис.
- **Подключение за 2–4 секунды** вместо 7–30: сохранённая подписка, прогрев по адресу и гонка попыток.
- **Сайты открываются сразу после подключения.** Исправлено «подключено, но ничего не грузится» на Mac с DNS 8.8.8.8 / 1.1.1.1: на время сеанса приложение ведёт системный DNS через туннель и возвращает его при отключении, в том числе если приложение закроется аварийно.
- Журнал пишет, какая локация и протокол выбраны и сколько занял каждый шаг подключения (без адресов).

## 🇬🇧 What's new

- **XHTTP.** Locations on the XHTTP transport («Britain · XHTTP») now connect; the previous build set them up as plain TCP and they never worked.
- **Protocol in the list of countries**: «VLESS · Vision», «VLESS · XHTTP», «Hysteria2» under every location, with the full name the service gives it.
- **Connects in 2-4 seconds** instead of 7-30: a fresh stored subscription, an address-based warm-up and racing attempts.
- **Sites load right after connecting.** Fixed "connected but nothing loads" on Macs with DNS 8.8.8.8 / 1.1.1.1: for the session the system resolver goes through the tunnel, and is handed back on disconnect — or on a crash.
- The log names the chosen location and protocol and times every connect step (no addresses).

---

# ProxysVPN Desktop v0.1.0-beta

🇷🇺 Первый публичный релиз. Бета-версия для Apple Silicon Mac.

🇬🇧 First public release. Beta version for Apple Silicon Macs.

---

## 🇷🇺 Что нового

Первая публичная сборка десктоп-клиента для сервиса [ProxysVPN](https://proxysvpn.com).

**Возможности:**
- Системный VPN на всю Mac через TUN-устройство (работает Telegram, игры, App Store)
- VLESS Reality + ML-KEM (post-quantum шифрование)
- System tray с управлением подключением
- Светлая / тёмная тема
- Встроенный просмотр логов для диагностики
- Живой пинг через туннель

## Установка

1. Скачай **`ProxysVPN_0.1.0_aarch64.dmg`** из Assets ниже
2. Открой `.dmg` (двойной клик)
3. Двойной клик на **«ProxysVPN Installer»**
4. Нажми **«Установить»** → введи пароль администратора если попросит
5. Готово — приложение запустится автоматически. Введи свою ссылку подписки и нажми кнопку питания.

> При первом запуске Installer macOS может спросить «открыть приложение от неизвестного разработчика» — нажми **«Открыть»**. Это разовое действие.

Получить ссылку подписки: [proxysvpn.com](https://proxysvpn.com)

## Системные требования

- macOS 11 Big Sur или новее
- **Apple Silicon (M1/M2/M3/M4)** — Intel Mac пока не поддерживается
- Привилегии администратора (запрашивается один раз при запуске VPN)

## Известные ограничения

- 🟡 **Бета-сборка** — приложение подписано ad-hoc (без Apple Developer ID). Установщик берёт это на себя автоматически. В v1.0 будет полная нотаризация Apple
- 🟡 **Только Apple Silicon** — Intel x86_64 в планах на v0.2
- 🟡 **Нет авто-обновлений** — следующие версии надо скачивать вручную
- 🟡 **Только VLESS Reality** — другие протоколы (Hysteria2, WireGuard) в планах

## Сообщить о проблеме

1. Открой приложение
2. Нажми кнопку **(i)** в нижнем правом углу
3. Нажми **«Скачать .txt»**
4. Создай [issue](https://github.com/evilork/proxysvpn-desktop/issues/new) и приложи файл

---

## 🇬🇧 What's new

First public build of the desktop client for [ProxysVPN](https://proxysvpn.com).

**Features:**
- System-wide VPN on Mac via TUN device (Telegram, games, App Store all work)
- VLESS Reality + ML-KEM (post-quantum encryption)
- System tray controls
- Light / dark theme
- Built-in logs viewer for diagnostics
- Live tunnel ping

## Installation

1. Download **`ProxysVPN_0.1.0_aarch64.dmg`** from Assets below
2. Open the `.dmg` (double-click)
3. Double-click **"ProxysVPN Installer"**
4. Click **"Install"** → enter admin password if prompted
5. Done — the app launches automatically. Enter your subscription URL and hit the power button.

> On first launch the Installer, macOS may ask whether to "open an app from an unidentified developer" — click **"Open"**. One-time action.

Get a subscription URL: [proxysvpn.com](https://proxysvpn.com)

## System requirements

- macOS 11 Big Sur or newer
- **Apple Silicon (M1/M2/M3/M4)** — Intel Mac not yet supported
- Admin privileges (asked once at VPN start)

## Known limitations

- 🟡 **Beta build** — ad-hoc signed (no Apple Developer ID yet). The installer handles this transparently. Full Apple notarization coming in v1.0
- 🟡 **Apple Silicon only** — Intel x86_64 planned for v0.2
- 🟡 **No auto-updates** — future versions need manual download
- 🟡 **VLESS Reality only** — other protocols (Hysteria2, WireGuard) planned

## Reporting issues

1. Open the app
2. Click the **(i)** button in the bottom-right corner
3. Click **"Download .txt"**
4. Open an [issue](https://github.com/evilork/proxysvpn-desktop/issues/new) and attach the file

---

**SHA-256 of the .dmg** will be added by the release script.
