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
# 1. Движок расширения (sing-box → Libbox.xcframework), один раз
bash scripts/build-libbox.sh

# 2. Окружение и Xcode-проект. PVPN_TEAM_ID — Team ID платного аккаунта
#    (developer.apple.com → Membership details); xcodegen пишет его в ОБА
#    таргета, подпись автоматическая. Без него собирается только симулятор.
PVPN_TEAM_ID=XXXXXXXXXX npm run ios:setup

# 3. Запуск: устройство или симулятор выбирается из списка
npm run ios:dev
```

В git из `src-tauri/gen/apple` лежит только написанное руками: `project.yml`,
`ExportOptions.plist`, `VpnBridge.swift`, `PacketTunnelProvider.swift`, оба
`PrivacyInfo.xcprivacy`, `InfoPlist.strings` (en, ru) и набор иконок. На
свежем клоне `setup-ios.sh` сам запускает `tauri ios init`: тот дописывает
только недостающие файлы (main.mm, bindings, LaunchScreen) и ничего
существующего не перезаписывает. Дальше скрипт перегенерирует проект
xcodegen'ом и падает с понятным сообщением, если нет `VpnBridge.swift` или
`Libbox.xcframework`.

После правок `project.yml` — снова `npm run ios:setup` (или
`cd src-tauri/gen/apple && PVPN_TEAM_ID=… xcodegen generate`: без переменной
команда в проект не попадёт).

### Bundle ID и версия

- Bundle ID в App Store — `com.proxysvpn.app`, у расширения —
  `com.proxysvpn.app.PacketTunnel`. Он записан дважды: `PVPN_BUNDLE_ID` в
  `project.yml` (приложение `$(PVPN_BUNDLE_ID)`, расширение
  `$(PVPN_BUNDLE_ID).PacketTunnel`) и `"identifier"` в
  `src-tauri/tauri.ios.conf.json`, потому что `tauri ios build/dev` при каждой
  сборке переписывает bundle ID приложения из конфига Tauri. При расхождении
  `setup-ios.sh` откажется работать. В Swift id не записан: id провайдера и
  subsystem логов берутся из id запущенного бандла. DMG для macOS остаётся
  `com.proxysvpn.desktop` (`src-tauri/tauri.conf.json`).
- `MARKETING_VERSION` в `project.yml` = `version` из `tauri.conf.json`
  (проверяет `setup-ios.sh`); при релизе поднимать вместе.
  `CURRENT_PROJECT_VERSION` 1 = `bundle.iOS.bundleVersion` в
  `tauri.ios.conf.json` — основа номера сборки (ниже).

## Выпуск в App Store Connect

```bash
PVPN_TEAM_ID=XXXXXXXXXX npm run ios:setup    # если менялся project.yml
npm run ios:build -- --build-number N        # N больше, чем у прошлой загрузки
# IPA: src-tauri/gen/apple/build/arm64/ProxysVPN.ipa
```

**Номер сборки.** Каждая загрузка требует нового `CFBundleVersion`.
`tauri ios build --build-number N` пишет его через `agvtool` в оба таргета:
`bundleVersion` из `src-tauri/tauri.ios.conf.json` (`1`) плюс `.N`, то есть
`1.N`. N берите больше, чем у прошлой загрузки (TestFlight показывает
последнюю). `bundleVersion` там стоит не зря: без него основой был бы
`version`, и вышло бы `0.3.1.N` — четыре числа, а App Store Connect принимает в
`CFBundleVersion` не больше трёх.

**Entitlements расширения (Tauri issue 15663).** Если сборка подписывается
ключом App Store Connect API (`APPLE_API_KEY`, `APPLE_API_ISSUER`,
`APPLE_API_KEY_PATH`), Tauri архивирует без подписи и переподписывает только
само приложение — `PacketTunnel.appex` может уйти без entitlements, и туннель
не запустится. Проверять после каждого экспорта:

```bash
cd "$(mktemp -d)" && unzip -q /path/to/ProxysVPN.ipa
codesign -d --entitlements - Payload/ProxysVPN.app/PlugIns/PacketTunnel.appex
# должно быть: com.apple.developer.networking.networkextension = [packet-tunnel-provider]
```

Если entitlements нет — архивировать и экспортировать через Xcode, а не
через `tauri ios build`. Фаза «Build Rust Code» спрашивает опции у запущенного
Tauri CLI, поэтому сначала `npm run tauri -- ios build --open` (CLI держит их,
пока открыт Xcode), затем в Xcode Product → Archive → Distribute App → App
Store Connect, либо во втором терминале. Номер сборки в этом пути ставится
руками: `CURRENT_PROJECT_VERSION=1.N`.

```bash
cd src-tauri/gen/apple
xcodebuild -project proxysvpn-desktop.xcodeproj -scheme proxysvpn-desktop_iOS \
  -configuration release -sdk iphoneos -archivePath build/ProxysVPN.xcarchive \
  -allowProvisioningUpdates CURRENT_PROJECT_VERSION=1.N archive
xcodebuild -exportArchive -archivePath build/ProxysVPN.xcarchive \
  -exportOptionsPlist ExportOptions.plist -exportPath build/export \
  -allowProvisioningUpdates
```

**Проверка IPv6-only (NAT64).** App Review тестирует в IPv6-only сети
(правило 2.5.5). Mac с проводным интернетом → Системные настройки → Основные →
Общий доступ → с зажатым Option нажать (i) у «Общий интернет» → включить
«Create NAT64 Network», раздавать с Ethernet по Wi-Fi. iPhone подключить к этой
сети (в настройках Wi-Fi у него будет только IPv6-адрес), поставить сборку из
TestFlight, пройти привязку, подключиться ко всем локациям и открыть несколько
сайтов. Адреса серверов в подписке должны быть именами, а не IPv4-литералами:
Go такие адреса в NAT64 не переводит.

**Что ещё приходит на ревью.**
- `PrivacyInfo.xcprivacy` приложения и расширения: причины для «required
  reason» API и собираемые данные. Собираемые данные должны совпадать с App
  Privacy в App Store Connect и политикой конфиденциальности. После смены
  движка или зависимостей сверить `nm -u` бинарников приложения и расширения со
  списком Apple (stat/fstat/…, mach_absolute_time/systemUptime, statfs/…,
  NSUserDefaults) и дописать недостающее.
- `ITSAppUsesNonExemptEncryption` добавляется в Info.plist (через
  `project.yml`) только после ответа на вопросы экспортного контроля в App
  Store Connect и со значением, которое они дают.
- Иконка: `AppIcon-512@2x.png` и все размеры — без альфа-канала
  (`sips -g hasAlpha` → `no`). `tauri icon` всегда пишет RGBA, после него
  альфу надо снять, например ImageMagick'ом (`brew install imagemagick`):
  `magick in.png -background white -alpha remove -alpha off out.png`.

## Ключевые файлы

| Файл | Роль |
|---|---|
| `src-tauri/src/singbox.rs` | генерация конфига sing-box из подписки |
| `src-tauri/src/ios_vpn.rs` | FFI-мост Rust → Swift, ожидание статуса NE |
| `src-tauri/tauri.ios.conf.json` | iOS-оверрайд: bundle ID App Store, основа номера сборки, без externalBin-бинарников |
| `src-tauri/capabilities/desktop.json` | разрешения плагинов, которые линкуются только на desktop |
| `src-tauri/gen/apple/project.yml` | спека xcodegen: app + PacketTunnel таргеты, bundle ID, версии, команда |
| `src-tauri/gen/apple/ExportOptions.plist` | экспорт для App Store Connect |
| `.../Sources/proxysvpn-desktop/VpnBridge.swift` | управление NETunnelProviderManager |
| `.../PacketTunnel/PacketTunnelProvider.swift` | расширение: sing-box внутри NE |
| `.../proxysvpn-desktop_iOS/PrivacyInfo.xcprivacy`, `.../PacketTunnel/PrivacyInfo.xcprivacy` | манифесты приватности |
| `.../proxysvpn-desktop_iOS/{en,ru}.lproj/InfoPlist.strings` | имя под иконкой |
| `.../Assets.xcassets/AppIcon.appiconset` | иконки iOS (копия `src-tauri/icons/ios`) |
| `scripts/build-libbox.sh` | сборка Libbox.xcframework (пиновая версия sing-box) |
| `scripts/setup-ios.sh` | проверка окружения, `tauri ios init` на свежем клоне, xcodegen |

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
- Логи: `Documents/app.log` — через Finder (iPhone по кабелю → «Файлы» →
  ProxysVPN); в приложении «Файлы» на телефоне папки нет, потому что
  `LSSupportsOpeningDocumentsInPlace` убран. Логи в Console.app: subsystem
  `com.proxysvpn.app.PacketTunnel` (расширение) и `com.proxysvpn.app`
  (`VpnBridge.swift`).

## Известные ограничения и грабли

- **Лимит памяти расширения ~50 МБ** — перед стартом вызывается
  `LibboxSetMemoryLimit(true)`; не добавляйте в конфиг тяжёлые rule-set'ы без
  необходимости.
- `PacketTunnelProvider.swift` собирается с **sing-box v1.12.4**
  (версия запинована в `scripts/build-libbox.sh`). При обновлении sing-box
  сверяйте сигнатуры протокола с `ExtensionPlatformInterface.swift` из
  [sing-box-for-apple](https://github.com/SagerNet/sing-box-for-apple) того же
  тега — gomobile-API между минорными версиями меняется, а Swift ещё и
  переименовывает методы (`autoDetectInterfaceControl` → `autoDetectControl`,
  `sendNotification` → `send`). Расширению нужен `libresolv.tbd`.
- **Xcode 27 (Swift 6.4):** swift-rs 1.0.7 передаёт SDK и target iOS в
  `swift build` так, что новая система сборки их игнорирует и собирает
  Swift-пакет Tauri под macOS — фаза «Build Rust Code» падает с `unable to
  resolve module dependency: 'UIKit'`. Исправлено в swift-rs 1.0.8, она
  записана в `src-tauri/Cargo.lock`; если ошибка вернулась, проверить, что lock
  не откатился на 1.0.7 (`cd src-tauri && cargo update -p swift-rs`).
- **Release-сборка на Xcode 27:** фаза «Build Rust Code» в `project.yml`
  выставляет `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`. Без этого
  swift-rs собирает Swift-часть Tauri оптимизированной, её хелперы
  `retain_object`, `release_object`, `string_from_bytes` становятся локальными,
  и архив падает на `Undefined symbols`. Rust остаётся оптимизированным,
  исполняемый файл в архиве всё равно очищается от символов, строки кода уходят
  в dSYM. Убрать, когда swift-rs начнёт экспортировать эти хелперы.
- **iOS 27 требует UIScene.** Приложение, собранное Xcode 27 с Tauri 2.10,
  в симуляторе iOS 27 не запускается: `UIScene life cycle is required for apps
  built with this SDK`. Нужен Tauri, который поднимает окно через UIScene;
  обновление Tauri — отдельная задача.
- Разрешение плагина, который линкуется только на desktop (как
  `notification:default`), кладётся в `src-tauri/capabilities/desktop.json`
  (`platforms`: macOS, windows, linux): иначе iOS-сборка падает на
  `Permission … not found`.
- `tauri ios xcode-script` собирает крейт обычным `cargo build`, то есть вместе
  с бинарником `src-tauri/src/main.rs`. На iOS его `main` пустой: вызов `run()`
  потянул бы функции `pvpn_*`, которые есть только в `VpnBridge.swift`, и
  линковка падала бы на `Undefined symbols`.
- Для App Store: приложениям с NEVPN нужен ответ на вопросы ревью о VPN
  (форма в App Store Connect), политика конфиденциальности обязательна.

## Лицензии

iOS-сборка линкует [sing-box](https://github.com/SagerNet/sing-box) (GPL-3.0
с дополнительными условиями) вместо xray/tun2socks. Проверьте совместимость с
вашей моделью распространения до публикации в App Store.
