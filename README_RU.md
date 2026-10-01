# ProxysVPN Desktop

> 🇬🇧 [English version](README.md)
>
> 📱 **iPhone / iPad:** из этой же кодовой базы собирается iOS-приложение с
> туннелем в Network Extension (движок Xray-core через libXray) — см. [docs/IOS.md](docs/IOS.md).

VPN-клиент для компьютера под сервис [ProxysVPN](https://proxysvpn.com) —
**macOS, Windows и Linux**. Пропускает IPv4-трафик системы через узел VLESS
(Reality / XHTTP) или Hysteria2 с раздельной маршрутизацией сервиса:
российские сайты и всё из вашего списка «всегда напрямую» идут напрямую.

## Возможности

- **Туннель на всю систему** через TUN-устройство и `tun2socks` (`utun225` на
  macOS, адаптер Wintun `ProxysVPN` на Windows, `proxysvpn0` на Linux). Пока
  только IPv4 — см. [Известные ограничения](#известные-ограничения).
- **VLESS Reality / XHTTP и Hysteria2.** xray всегда стоит первым в цепочке,
  поэтому правила раздельной маршрутизации действуют на любом протоколе.
- **Привязка без ввода ссылки:** код из 8 символов из кабинета или бота либо
  QR-код, который сканируется телефоном.
- **Чинится сам:** упавший движок перезапускается, узел, который перестал
  пропускать, заменяется другим, и окно говорит, что произошло.
- **Значок в трее** с пунктами «Открыть окно», «Выключить защиту», «Выйти»;
  закрытие окна не разрывает VPN.
- **Русский и английский** интерфейс.

## Скачать и установить

Установщики лежат на [странице релизов](https://github.com/evilork/proxysvpn-desktop/releases).
Подписи кода пока нет (ни Apple Developer ID, ни сертификата Windows), поэтому
каждая система один раз предупреждает. Перед установкой сверьте SHA-256
установщика с файлом `SHA256SUMS.txt` на странице релиза:

```bash
shasum -a 256 ProxysVPN_*            # macOS
sha256sum ProxysVPN_*                # Linux
certutil -hashfile ProxysVPN_<версия>_x64-setup.exe SHA256   # Windows
```

### macOS (Apple Silicon)

1. Откройте `ProxysVPN_<версия>_aarch64.dmg` и перетащите **ProxysVPN** на
   ярлык **Applications**.
2. Один раз снимите карантин загрузки, в Терминале:
   `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app`
   (или запустите приложение, нажмите **«Готово»**, затем Системные настройки →
   Конфиденциальность и безопасность → **«Всё равно открыть»**; с macOS 15
   «правый клик → Открыть» больше не работает).
3. Запустите ProxysVPN и введите пароль администратора — он нужен туннелю.
4. Привяжите приложение кодом или QR из кабинета / бота либо вставьте ссылку.

Подробности и решение проблем: [INSTALL.md](INSTALL.md).

### Windows 10 / 11 (x64)

1. Запустите `ProxysVPN_<версия>_x64-setup.exe`. SmartScreen скажет, что
   издатель неизвестен: **«Подробнее» → «Выполнить в любом случае»**.
2. Разрешите запрос прав администратора (UAC). Приложение просит их при
   каждом запуске: оно создаёт адаптер туннеля и маршруты.
3. Привяжите приложение кодом или QR либо вставьте ссылку.

### Linux (x86_64, .deb)

```bash
sudo apt install ./ProxysVPN_<версия>_amd64.deb
```

Запустите **ProxysVPN** из меню приложений. Первое подключение покажет окно
пароля polkit (в сеансе нужен агент polkit: он есть в GNOME, KDE, Xfce,
Cinnamon и MATE). Удаление: `sudo apt remove proxys-vpn`.

## Системные требования

- **macOS** 12 Monterey или новее, Apple Silicon (Mac на Intel не поддерживаются).
- **Windows** 10 или 11, x64 (проверено на Windows 11). WebView2 установщик
  поставит сам, если его нет.
- **Linux** x86_64 с glibc 2.35+ (Debian 12+, Ubuntu 22.04+; проверено на
  Debian 13), сеанс рабочего стола с агентом polkit, `iproute2`.

## Известные ограничения

- **IPv6 пока не идёт через туннель** ни на одной системе. Если ваша сеть
  даёт IPv6, программы, которые сами соединяются по IPv6-адресу (звонки
  WebRTC в браузере, приложения со своим DNS), могут ходить мимо VPN. Для имён,
  которые разрешает само приложение, IPv6-адреса не выдаются.
- **Windows: DNS пока не направляется в туннель.** Имена сайтов разрешает DNS
  вашей сети (роутер или провайдер), и эти запросы могут идти мимо VPN. Об этом
  сказано в уведомлении о данных в приложении.
- **Нет подписи кода и автообновления.** Новые версии ставятся вручную со
  страницы релизов.
- **macOS:** после сбоя или принудительного завершения движки могут работать,
  пока ProxysVPN не открыт снова; при открытии приложение их останавливает.
- **macOS:** всё приложение работает от root (пока нет подписанного
  привилегированного помощника); движки при запуске копируются в папку,
  принадлежащую root, так что после ввода пароля от root не запускается ничего
  из бандла, который может изменить пользователь.

## Архитектура

```
tun2socks (TUN) ──socks5 127.0.0.1:10808──> xray ──vless──> узел
                                             └──socks5 127.0.0.1:10809──> hysteria ──> узел (Hysteria2)
```

- **Фронтенд:** React 19 + TypeScript (webview Tauri 2).
- **Ядро:** Rust (Tauri 2 + tokio): подписка и привязка, лестница починки,
  супервизор (`src-tauri/src/lib.rs`).
- **Платформенный слой:** `src-tauri/crates/pvpn-platform` — маршруты,
  TUN-устройство, DNS и права на каждой ОС. На Linux от root работает только
  маленький помощник через `pkexec`, окно остаётся без прав. См.
  [docs/WINDOWS.md](docs/WINDOWS.md), [docs/LINUX.md](docs/LINUX.md),
  [docs/DESIGN.md](docs/DESIGN.md).
- **Движки:** Xray-core 26.3.27, Hysteria 2.9.3, tun2socks 2.6.0, закреплены
  с SHA-256 в `scripts/sidecars.lock`.

## Сборка из исходников

```bash
# Нужны: Rust (stable), Node.js 22+, npm
git clone https://github.com/evilork/proxysvpn-desktop.git
cd proxysvpn-desktop

bash scripts/fetch-binaries.sh      # закреплённые движки и geo-данные, с проверкой сумм
npm ci

# macOS: .dmg с root-лаунчером
bash src-tauri/scripts/build-release.sh
# Windows / Linux
npx tauri build --bundles nsis      # или: --bundles deb
```

Подробнее — [docs/BUILDING.md](docs/BUILDING.md).

## Приватность

- **Никакой телеметрии и аналитики.** Приложение обращается к:
  - сайту вашей ссылки подписки и, если это наш сайт и он не отвечает, к
    резервным `proxysvpn.com`, `proxysvnovich.vercel.app`, `proksya.com`,
    `proksya.xyz` (подписка, подписанный список серверов, привязка). В этих
    запросах есть токен ссылки и случайный идентификатор устройства
    (`x-hwid`): так сервис соблюдает правило «одна ссылка — одно устройство»;
  - VPN-узлам из подписки;
  - пока вы подключены — к проверке, что туннель пропускает трафик:
    `proxysvpn.store/api/exit-ip` и страница `/api/tools/whoami` резервных
    сайтов, через туннель;
  - DNS: Cloudflare (DNS-over-HTTPS) и Google, через VPN-узел (или резолвер,
    выбранный в настройках туннеля).
- **Журнал остаётся на устройстве:** `~/Library/Logs/ProxysVPN/app.log`
  (macOS), `%LOCALAPPDATA%\ProxysVPN\logs\app.log` (Windows),
  `~/.local/state/ProxysVPN/app.log` (Linux). Адреса узлов, токены и имена
  открытых вами сайтов маскируются до записи строки. «Отчёт для поддержки»
  покидает устройство, только если вы отправите его сами.
- **Никаких аккаунтов в приложении.** Оно привязано к вашей ссылке подписки.

## Удаление

- **macOS:** перенесите ProxysVPN в Корзину, затем
  `sudo rm -rf "/Library/Application Support/ProxysVPN"` и удалите
  `~/Library/Application Support/com.proxysvpn.desktop` и
  `~/Library/Logs/ProxysVPN`.
- **Windows:** Параметры → Приложения → ProxysVPN → Удалить, затем удалите
  папку `%LOCALAPPDATA%\ProxysVPN` (деинсталлятор её пока не трогает, а в ней
  ссылка, идентификатор устройства и журнал).
- **Linux:** `sudo apt purge proxys-vpn`, затем удалите
  `~/.local/share/ProxysVPN` и `~/.local/state/ProxysVPN`.

## Лицензия

MIT — см. [LICENSE](LICENSE).

В установщиках есть сторонние компоненты под своими лицензиями: Xray-core
(MPL-2.0), Hysteria (MIT), tun2socks (MIT), Wintun на Windows (лицензия
WireGuard LLC на готовые бинари), Tauri (MIT/Apache-2.0) и другие. Стоковые
бинари Xray-core и Hysteria содержат и Go-модули под GPL-3.0-or-later. Полный
список с версиями и исходниками — в [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)
и на экране «Лицензии открытого ПО» в приложении.

## Ссылки

- [proxysvpn.com](https://proxysvpn.com) — сервис
- [@proxysvpn_bot](https://t.me/proxysvpn_bot) — Telegram-бот для аккаунта и оплаты
- [Issues](https://github.com/evilork/proxysvpn-desktop/issues) — сообщения об ошибках и предложения
