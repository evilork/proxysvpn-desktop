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

1. Скачай **`ProxysVPN_0.1.0_aarch64.dmg`** из этого релиза (внизу страницы, в Assets)
2. Открой `.dmg`, перетащи `ProxysVPN.app` в **Программы**
3. **Один раз** запусти в Терминале:
   ```bash
   xattr -cr /Applications/ProxysVPN.app
   ```
4. Запусти из Программ
5. Введи свою ссылку подписки (получить на [proxysvpn.com](https://proxysvpn.com))
6. Нажми кнопку питания

> Команда `xattr` нужна один раз — снимает карантинный флаг macOS. Без Apple Developer ID ($99/год) приложение не может пройти полную нотаризацию, поэтому это пока такой workaround.

## Системные требования

- macOS 11 Big Sur или новее
- **Apple Silicon (M1/M2/M3/M4)** — Intel Mac пока не поддерживается
- Привилегии администратора (запрашивается один раз при запуске VPN)

## Известные ограничения

- 🟡 **Нет Apple Developer подписи** — нужна команда `xattr` (см. выше). Будет в v1.0
- 🟡 **Только Apple Silicon** — Intel x86_64 в планах на v0.2
- 🟡 **Нет авто-обновлений** — следующие версии надо скачивать вручную
- 🟡 **Только VLESS Reality** — другие протоколы (Hysteria2, WireGuard) в планах

## Сообщить о проблеме

1. Открой приложение
2. Нажми кнопку (i) в нижнем правом углу
3. Нажми **Скачать .txt**
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
2. Open `.dmg`, drag `ProxysVPN.app` to **Applications**
3. Run **once** in Terminal:
   ```bash
   xattr -cr /Applications/ProxysVPN.app
   ```
4. Launch from Applications
5. Enter your subscription URL (get one at [proxysvpn.com](https://proxysvpn.com))
6. Hit the power button

> The `xattr` command is needed once — it removes the macOS quarantine flag. Without an Apple Developer ID ($99/year) we can't do full notarization yet, so this is a temporary workaround.

## System requirements

- macOS 11 Big Sur or newer
- **Apple Silicon (M1/M2/M3/M4)** — Intel Mac not yet supported
- Admin privileges (asked once at VPN start)

## Known limitations

- 🟡 **No Apple Developer signature** — `xattr` workaround required (see above). Coming in v1.0
- 🟡 **Apple Silicon only** — Intel x86_64 planned for v0.2
- 🟡 **No auto-updates** — future versions need manual download
- 🟡 **VLESS Reality only** — other protocols (Hysteria2, WireGuard) planned

## Reporting issues

1. Open the app
2. Click the (i) button in the bottom-right corner
3. Click **Download .txt**
4. Open an [issue](https://github.com/evilork/proxysvpn-desktop/issues/new) and attach the file

---

**SHA-256 of the .dmg** will be added by the release script.
