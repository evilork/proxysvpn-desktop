# ProxysVPN для iPhone / iPad

Сборка и архитектура iOS-версии. UI (React + Tauri) общий с macOS; VPN-слой на
iOS полностью другой — этого требует платформа.

## Почему iOS-версия устроена иначе

На macOS приложение спавнит процессы `xray` / `hysteria` / `tun2socks` и правит
таблицу маршрутов. На iOS всё это запрещено песочницей:

- нельзя запускать дочерние процессы — движок должен быть **библиотекой**;
- VPN живёт только внутри **Network Extension** (`NEPacketTunnelProvider`) —
  отдельный процесс-расширение, которым управляет система;
- маршруты задаются декларативно через `NEPacketTunnelNetworkSettings`.

Поэтому на iOS оба протокола (VLESS Reality и Hysteria2) обслуживает один
движок — **sing-box** (`Libbox.xcframework`), работающий внутри расширения.

## Архитектура

```
┌────────────────────────────────────────────────────────────┐
│ ProxysVPN.app (iOS)                                        │
│                                                            │
│  React UI (WKWebView) ──invoke──> Rust core                │
│    │  адаптив: safe areas,          │ subscription.rs      │
│    │  44pt-таргеты, iPad-центровка  │ singbox.rs (конфиг)  │
│    │                                │ ios_vpn.rs (FFI)     │
│    │                                ▼                      │
│    │                     VpnBridge.swift (@_cdecl)         │
│    │                           │ NETunnelProviderManager   │
└────┼───────────────────────────┼───────────────────────────┘
     │                           ▼  (запускает система)
     │            ┌──────────────────────────────────┐
     │            │ PacketTunnel.appex               │
     │            │  PacketTunnelProvider.swift      │
     │            │  └─ sing-box (Libbox.xcframework)│
     │            │     VLESS Reality / Hysteria2    │
     │            └──────────────────────────────────┘
     ▼
  весь трафик устройства
```

Поток подключения:

1. UI вызывает `vpn_connect` (тот же интерфейс команд, что на macOS).
2. Rust скачивает подписку, выбирает сервер, строит **JSON-конфиг sing-box**
   (`src-tauri/src/singbox.rs`) — умный роутинг сохранён: приватные сети и
   российские домены идут напрямую, QUIC блокируется для VLESS.
3. Через C FFI конфиг уходит в `VpnBridge.swift`, который сохраняет
   VPN-профиль (`NETunnelProviderManager`) и стартует туннель. При первом
   подключении iOS покажет системный диалог «Разрешить конфигурацию VPN».
4. Система поднимает расширение `PacketTunnel`, оно запускает sing-box и
   отдаёт ему tun-интерфейс.
5. Rust опрашивает статус NE и возвращает результат в UI.

## Требования

- Mac с **полным Xcode** (не только Command Line Tools):
  `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`
- **Платный Apple Developer аккаунт** — entitlement Network Extension не
  работает с бесплатным.
- Go 1.23+ (`brew install go`) — для сборки Libbox.
- Rust + iOS-таргеты (`rustup target add aarch64-apple-ios aarch64-apple-ios-sim`).
- Node.js 18+, CocoaPods, xcodegen (поставит setup-скрипт).
- **Реальный iPhone/iPad** — симулятор не поддерживает Network Extension:
  UI запустится, но подключение VPN невозможно.

## Сборка

```bash
# 1. Всё окружение одной командой (проверит Xcode, соберёт Libbox и т.д.)
npm run ios:setup

# 2. Один раз в Xcode: открыть src-tauri/gen/apple/proxysvpn-desktop.xcodeproj
#    и для ОБОИХ таргетов (proxysvpn-desktop_iOS, PacketTunnel) указать
#    Signing & Capabilities → Team.

# 3. Запуск на подключённом устройстве
npm run ios:dev

# Release IPA
npm run ios:build
```

## Ключевые файлы

| Файл | Роль |
|---|---|
| `src-tauri/src/singbox.rs` | генерация конфига sing-box из подписки |
| `src-tauri/src/ios_vpn.rs` | FFI-мост Rust → Swift, ожидание статуса NE |
| `src-tauri/tauri.ios.conf.json` | iOS-оверрайд (без externalBin-бинарников) |
| `src-tauri/gen/apple/project.yml` | спека xcodegen: app + PacketTunnel таргеты |
| `.../Sources/proxysvpn-desktop/VpnBridge.swift` | управление NETunnelProviderManager |
| `.../PacketTunnel/PacketTunnelProvider.swift` | расширение: sing-box внутри NE |
| `scripts/build-libbox.sh` | сборка Libbox.xcframework (пиновая версия sing-box) |
| `scripts/setup-ios.sh` | проверка/установка всего окружения |

После правок `project.yml` перегенерируйте проект: `cd src-tauri/gen/apple && xcodegen generate`.
Повторно `tauri ios init` запускать не нужно (перезапишет project.yml).

## Отличия поведения от macOS

- **Нет прав root и диалога администратора** — вместо него системный запрос
  «Разрешить конфигурацию VPN» (один раз).
- **Нет трея** — статус VPN виден в статус-баре iOS и в Настройках → VPN.
- **DNS идёт через туннель** (DoH 1.1.1.1, для RU-доменов — 77.88.8.8
  напрямую). На desktop DNS сознательно оставлен системным ради скорости.
- **Смена сети / сон** обрабатывает система + `sleep()/wake()` в расширении —
  route-watchdog не нужен.
- **Пинг** меряется через туннель (на desktop — напрямую до edge-сервера),
  значения могут быть чуть выше.
- Hysteria2 c `pinSHA256`: sing-box не поддерживает пин, используется
  `insecure` (сервер аутентифицируется паролем hy2 до передачи данных —
  тот же компромисс, что и на desktop, где пин включает insecure).
- Логи: `Файлы (Files) → ProxysVPN → app.log`; логи расширения — Console.app
  по subsystem `com.proxysvpn.desktop.PacketTunnel`.

## Известные ограничения и грабли

- **Лимит памяти расширения ~50 МБ** — sing-box укладывается; не добавляйте в
  конфиг тяжёлые rule-set'ы без необходимости.
- `PacketTunnelProvider.swift` написан под API **sing-box v1.12.x**
  (версия запинована в `scripts/build-libbox.sh`). При обновлении sing-box
  сверяйте сигнатуры протокола с `ExtensionPlatformInterface.swift` из
  [sing-box-for-apple](https://github.com/SagerNet/sing-box-for-apple) того же
  тега — gomobile-API между минорными версиями меняется.
- Bundle ID расширения (`com.proxysvpn.desktop.PacketTunnel`) обязан быть
  префиксован bundle ID приложения и продублирован в
  `VpnBridge.swift → providerBundleId`.
- Для App Store: приложениям с NEVPN нужен ответ на вопросы ревью о VPN
  (форма в App Store Connect), политика конфиденциальности обязательна.

## Лицензии

iOS-сборка линкует [sing-box](https://github.com/SagerNet/sing-box) (GPL-3.0
с дополнительными условиями) вместо xray/tun2socks. Проверьте совместимость с
вашей моделью распространения до публикации в App Store.
