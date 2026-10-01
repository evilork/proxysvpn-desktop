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

Поэтому на iOS все протоколы (VLESS REALITY с Vision или XHTTP и Hysteria2)
обслуживает один движок — **Xray-core** из `LibXray.xcframework`
([libXray](https://github.com/XTLS/libXray), MIT, поверх Xray-core, MPL-2.0),
работающий внутри расширения. Тот же Xray, что на macOS, и тот же генератор
конфига: у iPhone те же локации и правила, что у Mac.

До 28.09.2026 здесь был sing-box (Libbox). Его сменили по трём причинам: в нём
нет XHTTP (Британия, США и Франция с 27.09 только XHTTP), он не проходит
REALITY на Xray ≥ 26.9.8 (issue SagerNet/sing-box#4520), и он GPL-3.0, что
несовместимо с App Store.

## Архитектура

```
┌────────────────────────────────────────────────────────────┐
│ ProxysVPN.app (iOS)                                        │
│                                                            │
│  React UI (WKWebView) ──invoke──> Rust core                │
│    │  адаптив: safe areas,          │ subscription.rs      │
│    │  44pt-таргеты, iPad-центровка  │ xray_apple.rs (конфиг)│
│    │                                │ ios_vpn.rs (FFI)     │
│    │                                ▼                      │
│    │                     VpnBridge.swift (@_cdecl)         │
│    │                           │ NETunnelProviderManager   │
└────┼───────────────────────────┼───────────────────────────┘
     │                           ▼  (запускает система)
     │            ┌──────────────────────────────────┐
     │            │ PacketTunnel.appex               │
     │            │  PacketTunnelProvider.swift      │
     │            │  └─ Xray-core (LibXray.xcframework)
     │            │     REALITY Vision/XHTTP, Hy2    │
     │            └──────────────────────────────────┘
     ▼
  весь трафик устройства
```

Поток подключения:

1. UI вызывает `vpn_connect` (тот же интерфейс команд, что на macOS).
2. Rust скачивает подписку, выбирает сервер и строит **JSON-конфиг Xray**
   (`src-tauri/src/xray_apple.rs`) из десктопного генератора
   (`subscription.rs`): те же исходящие, тот же роутинг из профиля подписки и
   «Своих правил» — приватные сети и российские домены напрямую, QUIC
   блокируется для VLESS. Вместо SOCKS-порта — вход `tun`, правила без
   geo-файлов (сети прописаны литералами). Рядом — адреса туннеля для
   `NEPacketTunnelNetworkSettings`: всё уходит одной строкой
   `{"version":1,"xray":…,"tunnel":…}`.
3. Через C FFI конфиг уходит в `VpnBridge.swift`, который сохраняет
   VPN-профиль (`NETunnelProviderManager`) и стартует туннель. При первом
   подключении iOS покажет системный диалог «Разрешить конфигурацию VPN».
4. Система поднимает расширение `PacketTunnel`. Оно применяет адреса и
   маршруты (IPv4 и IPv6 по умолчанию в туннель, локальные сети мимо, DNS —
   адрес внутри туннеля), находит файловый дескриптор utun, кладёт его в
   `env["xray.tun.fd"]` конфига и запускает Xray (`CGoInvoke` → `runXray`;
   остановка — `stopXray`).
5. Rust опрашивает статус NE и возвращает результат в UI.

## Требования

- Mac с **полным Xcode** (не только Command Line Tools):
  `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`
- **Платный Apple Developer аккаунт** — entitlement Network Extension не
  работает с бесплатным.
- Go (`brew install go`) и python3 — для сборки `LibXray.xcframework`. Нужную
  версию Go (сейчас go1.27.1) скрипт скачивает сам.
- Rust + iOS-таргеты (`rustup target add aarch64-apple-ios aarch64-apple-ios-sim`).
- Node.js 18+, CocoaPods, xcodegen (поставит setup-скрипт).
- **Реальный iPhone/iPad** — симулятор не поддерживает Network Extension:
  UI запустится, но подключение VPN невозможно.

## Сборка

```bash
# 1. Движок расширения (Xray-core → LibXray.xcframework), один раз и после
#    смены версий в скрипте. Срезы: iOS, iOS Simulator, macOS, tvOS, tvOS Sim.
bash scripts/build-libxray.sh

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
`LibXray.xcframework`. Старый `Frameworks/Libbox.xcframework` можно удалить:
его больше ничто не линкует.

### Движок: что и из чего собирается

`scripts/build-libxray.sh` закрепляет всё, от чего зависит двоичный код
расширения: тег **и коммит** libXray (v26.9.9), коммит Xray-core (v26.9.9),
который libXray пинует в go.mod, и версию Go (go1.27.1). Сдвинутый тег или
разошедшийся пин ломают сборку, а не подменяют движок. Сборка — штатная
`python3 build/main.py apple go local` из libXray (cgo, один Go-рантайм, есть
срез tvOS), с двумя изменениями ради лицензии (проверено 28.09.2026):

- Xray-core тянет sagernet/sing (GPL-3.0) ради Shadowsocks 2022 — через
  загрузчик конфига и CLI, даже если конфиг его не использует.
  `scripts/libxray/xray-core-no-gpl.patch` убирает Shadowsocks 2022 и
  CLI-команды (мы не используем ни то, ни другое). Если патч не ложится на
  новую версию Xray-core, сборка падает — патч нужно переделать.
- XTLS/REALITY тянет juju/ratelimit (LGPL-3.0) ради серверной функции.
  Вместо него подставлен `scripts/libxray/ratelimit` — своя реализация тех же
  трёх имён под MIT (`go test` в этой папке).

В конце скрипт перечисляет Go-модули, реально слинкованные в iOS-срез, с их
лицензиями и текстами в `scripts/libxray-notices.json` (коммитится; вход для
экрана лицензий) и падает на любой GPL-семейной или нераспознанной лицензии.
Рядом с фреймворком пишется `Frameworks/LibXray.version`.

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

### Сборка без покупок (App Store)

Каждая iOS-сборка — сборка для App Store / TestFlight, а приложение идёт без
покупок внутри (правило 3.1.3(f)), поэтому в нём нет ни одной кнопки, ссылки
или фразы, ведущей к оплате вне Apple. `tauri.ios.conf.json` собирает окно с
`VITE_DIST=appstore` (`beforeDevCommand` и `beforeBuildCommand`), а ядро — с
фичей `appstore` (`build.features`). Что это меняет:

- окно (`src/dist.ts`): нет «Пополнить», кабинета и бота; ошибки про баланс и
  срок предлагают «Проверить снова»; текст от сервиса (объявление, info-текст,
  пояснение к ошибке) показывается, только если в нём нет слов про деньги
  (`src/storeCopy.ts`); наружу открываются только страницы из
  `src/externalUrl.ts` (политика, условия, поддержка, удаление аккаунта,
  wata.fast); привязка звучит как вход в аккаунт; мок-ядро и демо-полоса
  вырезаются из бандла;
- ядро: User-Agent `ProxysVPN/<версия> (iOS; appstore)` и заголовок
  `x-client-dist: appstore` при загрузке подписки — по ним сервис убирает
  текст и ссылки про оплату из ответа.

DMG собирается без того и другого и ведёт себя как раньше. Ядро встраивает
то, что лежит в `dist/`: если окно собирается не через `tauri ios build/dev`
(например, `dist/` готовится заранее для ручного `xcodebuild`), собирать его
надо командой `VITE_DIST=appstore npm run build`, а ручной `cargo build` для
iOS — с `--features appstore`.

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
пока открыт Xcode; адрес и токен CLI 2.12 пишет в
`src-tauri/gen/apple/.tauri/cli-options-server.json`, вне git), затем в
Xcode Product → Archive → Distribute App → App Store Connect, либо во втором
терминале. Номер сборки в этом пути ставится
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
сайтов, в том числе из «прямого» списка (ya.ru) и политику конфиденциальности
на proxysvpn.com. В Console.app (subsystem расширения, категория `Tunnel`,
Include Info Messages) при подключении должна появиться строка
`IPv6-only network: N addresses mapped through NAT64, M direct routes through
the node`. Сам Go IPv4-литералы в NAT64 не переводит: это делает расширение
(см. «Адрес узла» ниже).

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
- Вход по коду («Войти по коду из аккаунта», pair-code v1) приложение
  показывает всем: своих пользователей у него пока нет, отдельного шлюза в
  нём нет. Поэтому сборку на ревью не отправлять, пока на боевом сайте нет
  `POST /api/pair/code` и пока выдача кодов не открыта всем
  (`pair:code:users` содержит `"*"`). До выката маршрута сайт отвечает на
  этот запрос из `/api/pair/[token]` — 400 `bad token`, ядро идёт по всем
  именам и пишет «Не удалось связаться с сервисом»; до открытия шлюза у
  демо-аккаунта ревьюера нет «Кода для приложения», и вход на экране
  привязки не работает (правило 2.1).

## Ключевые файлы

| Файл | Роль |
|---|---|
| `src-tauri/src/xray_apple.rs` | конфиг Xray для расширения (из десктопного генератора) и адреса туннеля |
| `src-tauri/src/ios_vpn.rs` | FFI-мост Rust → Swift, ожидание статуса NE |
| `src-tauri/tauri.ios.conf.json` | iOS-оверрайд: bundle ID App Store, основа номера сборки, без externalBin-бинарников |
| `src-tauri/capabilities/{desktop,mobile}.json` | разрешения окна: desktop (с уведомлениями) и iOS (без них) |
| `src-tauri/gen/apple/project.yml` | спека xcodegen: app + PacketTunnel таргеты, bundle ID, версии, команда |
| `src-tauri/gen/apple/ExportOptions.plist` | экспорт для App Store Connect |
| `.../Sources/proxysvpn-desktop/VpnBridge.swift` | управление NETunnelProviderManager |
| `.../PacketTunnel/PacketTunnelProvider.swift` | расширение: Xray внутри NE, дескриптор utun, журнал памяти |
| `.../PacketTunnel/Ipv6OnlyNetwork.swift` | сеть без IPv4 (NAT64): синтез адресов для IPv4-литералов, «прямые» маршруты через узел |
| `tests/packet-tunnel/`, `scripts/test-packet-tunnel.sh` | проверка чистой Swift-логики расширения на Mac (`bash scripts/test-packet-tunnel.sh`) |
| `.../proxysvpn-desktop_iOS/PrivacyInfo.xcprivacy`, `.../PacketTunnel/PrivacyInfo.xcprivacy` | манифесты приватности |
| `.../proxysvpn-desktop_iOS/{en,ru}.lproj/InfoPlist.strings` | имя под иконкой |
| `.../Assets.xcassets/AppIcon.appiconset` | иконки iOS (копия `src-tauri/icons/ios`) |
| `scripts/build-libxray.sh` | сборка LibXray.xcframework (пины, патч лицензии, список лицензий) |
| `scripts/libxray/` | патч Xray-core без GPL и замена juju/ratelimit |
| `scripts/libxray-notices.json` | лицензии Go-модулей движка (генерируется скриптом) |
| `scripts/setup-ios.sh` | проверка окружения, `tauri ios init` на свежем клоне, xcodegen |

## Отличия поведения от macOS

- **Нет прав root и диалога администратора** — вместо него системный запрос
  «Разрешить конфигурацию VPN» (один раз).
- **Нет трея** — статус VPN виден в статус-баре iOS и в Настройках → VPN.
- **DNS идёт через туннель, и это политика, на которую ссылается экран
  данных в приложении** — менять её только вместе с тем текстом:
  - все имена — Cloudflare DoH (`https://1.1.1.1/dns-query`) через узел;
  - российские и прочие «прямые» имена (профиль подписки, запасной список
    `RU_DIRECT_DOMAINS`, «Всегда напрямую») — Yandex `77.88.8.8` напрямую, мимо
    туннеля; «Всегда через VPN» выигрывает у прямого списка;
  - имя самого узла — системным резолвером расширения (он знает NAT64/DNS64);
  - приложениям отдаются только A-записи: IPv6-выхода у узлов нет. IPv6,
    который всё же дошёл до туннеля и не ушёл «напрямую», отбивается сразу;
  - запросы не-адресных типов (HTTPS/SVCB, MX, TXT, SRV) получают пустой
    ответ NOERROR: десктоп отправляет их через узел `proxySettings`, которого
    в Xray 26.9 больше нет.

  Поэтому экран туннеля на iOS не показывает выбор DNS и IPv4/IPv6
  (`TunnelScreen.tsx`): движок заменил бы их этой политикой. На desktop DNS
  устроен иначе (см. `build_xray_config`).
- **Адрес узла** резолвится системным резолвером: IPv4, если сеть его умеет,
  иначе IPv6 (на сети IPv6-only с NAT64 это синтезированный адрес).
- **Сеть без IPv4 (IPv6-only с NAT64, как у App Review).** Go открывает
  IPv4-литерал как AF_INET и в NAT64 его не переводит, поэтому расширение
  до подъёма туннеля спрашивает `getaddrinfo` (AI_DEFAULT) о каждом
  IPv4-литерале, который набирает сам Xray: узел, записанный адресом, и
  `77.88.8.8`. На такой сети ответ — синтезированный IPv6-адрес, и расширение
  подставляет его по плану из конфига (`Ipv6OnlyPlan` в `xray_apple.rs`,
  `Ipv6OnlyNetwork.swift`); на любой сети с IPv4 конфиг не меняется. Там же
  «прямые» маршруты (российские имена и сети, «Всегда напрямую») уходят через
  узел: приложения получают IPv4-адреса, а напрямую IPv4 с такой сети
  недостижим. Запросы имён к Яндексу по-прежнему идут напрямую (на
  синтезированный адрес), локальная сеть — тоже.
- **Смена сети / сон** обрабатывает система + `sleep()/wake()` в расширении —
  route-watchdog не нужен.
- **Пинг** меряется через туннель (на desktop — напрямую до edge-сервера),
  значения могут быть чуть выше.
- **Hysteria2** — встроенный клиент Xray (фаза 1). Пин сертификата
  (`pinSHA256`) проверяется: он превращается в `pinnedPeerCertSha256`, режима
  `insecure` нет. Ссылка с `insecure=1` без пина не подключается (движок
  проверяет каждый сертификат). Если клиент Xray окажется нерабочим из
  российских сетей (Xray-core#6717), фаза 2 — собрать клиент apernet в ту же
  Go-сборку (не отдельным фреймворком).
- Логи: `Documents/app.log` — через Finder (iPhone по кабелю → «Файлы» →
  ProxysVPN); в приложении «Файлы» на телефоне папки нет, потому что
  `LSSupportsOpeningDocumentsInPlace` убран. Логи в Console.app: subsystem
  `com.proxysvpn.app.PacketTunnel` (расширение) и `com.proxysvpn.app`
  (`VpnBridge.swift`).

## Известные ограничения и грабли

- **Лимит памяти расширения ~50 МБ** (цель — не больше 45 МиБ). libXray сам
  ограничивает кучу Go 30 МиБ и раз в секунду возвращает память системе.
  Расширение каждые 30 с пишет в журнал `footprint … MiB, available … MiB`
  (Console.app, subsystem `com.proxysvpn.app.PacketTunnel`, категория
  `Memory`, уровень info — включить «Include Info Messages»). Geo-файлы в
  расширение не кладутся: правила только литералами.
- **Один Go-рантайм на процесс.** В расширение линкуется только
  `LibXray.xcframework`; второй Go-фреймворк (Libbox, отдельный Hysteria)
  роняет процесс при загрузке. Расширению нужны `libresolv.tbd`,
  `Security.framework` и `CoreFoundation.framework` (их зовёт рантайм Go;
  статическая библиотека флаги линковки не несёт).
- **Сеть без IPv4 распознаётся один раз, при подключении.** После подъёма
  туннеля у устройства есть IPv4 (адрес utun), и `getaddrinfo` уже не
  отличает IPv6-only сеть от обычной. Если посреди сессии сменить IPv6-only
  Wi-Fi на сеть с IPv4 или наоборот, а узел записан IPv4-адресом, туннель
  останется со старым адресом узла — нужно переподключиться. Узлы с именами
  и сети с 464XLAT (сотовые IPv6-only) этого не касается.
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
- **iOS 27 требует UIScene (Apple TN3187).** Приложение, собранное SDK
  iOS 27 без сценового жизненного цикла, не запускается: `Application failed
  to launch: UIScene life cycle is required for apps built with this SDK`.
  Так было с Tauri 2.10. С Tauri 2.12 (tao 0.37) делегат приложения всегда
  отвечает на `application:configurationForConnectingSceneSession:options:` и
  ставит сценовый делегат tao, поэтому `UIApplicationSceneManifest` в
  `Info.plist` не нужен и шаблон проекта перегенерировать не надо. Проверено
  28.09.2026 в симуляторах iOS 27.0 и 26.5, iPhone и iPad: 2.10 падает с
  этой ошибкой, 2.12 запускается, строк про UIScene в журнале нет. Ниже
  `tauri = "2.12"` в `Cargo.toml` не опускать.
- Разрешения окна у каждой платформы свои: `src-tauri/capabilities/desktop.json`
  (macOS, windows, linux) и `mobile.json` (iOS). Разрешение плагина, который
  линкуется только на desktop (как `notification:default`), пишется только в
  `desktop.json`: иначе iOS-сборка падает на `Permission … not found`. Общие
  разрешения (opener, буфер обмена) записаны в оба файла.
- `tauri ios xcode-script` собирает крейт обычным `cargo build`, то есть вместе
  с бинарником `src-tauri/src/main.rs`. На iOS его `main` пустой: вызов `run()`
  потянул бы функции `pvpn_*`, которые есть только в `VpnBridge.swift`, и
  линковка падала бы на `Undefined symbols`.
- Для App Store: приложениям с NEVPN нужен ответ на вопросы ревью о VPN
  (форма в App Store Connect), политика конфиденциальности обязательна.

## Лицензии

Расширение линкует только разрешительный код и MPL-2.0 (файловый копилефт):
Xray-core и XTLS/REALITY (MPL-2.0), libXray (MIT), utls, quic-go, gVisor и
остальные — полный список с текстами в `scripts/libxray-notices.json`. MPL
требует сказать получателю, где взять исходники: Xray-core — коммит
`52a412d9e2f5` плюс наш патч `scripts/libxray/xray-core-no-gpl.patch` (этот
репозиторий публичный). Apache-2.0 требует показать файлы NOTICE — они в том
же JSON (`noticeText`). GPL-кода в сборке нет: это проверяет
`scripts/build-libxray.sh`.

В приложении это экран «Лицензии открытого ПО» (`LicensesScreen.tsx`). Его
список `src/assets/third-party-notices.json` собирает
`node scripts/gen-notices.mjs` из Cargo, npm и `scripts/libxray-notices.json`:
модули `xtls/xray-core` и `xtls/libxray` становятся iOS-строками «Xray-core» и
«libXray» (закреплённый коммит; у Xray-core ещё и ссылка на патч), наш
`scripts/libxray/ratelimit` в список не попадает, у остальных Go-модулей —
строки Copyright из их LICENSE и NOTICE. После каждого запуска
`build-libxray.sh`, изменившего `libxray-notices.json`, — перезапустить
генератор и закоммитить оба файла; `node scripts/gen-notices.mjs --check`
скажет, устарел ли список.

## Проверка на устройствах (владелец)

Симулятор Network Extension не запускает, поэтому всё ниже — на железе, с
командой разработчика организации:

1. Самый старый поддерживаемый iPhone (iOS 15+) и iPad: привязка, подключение
   к каждой локации — Vision, XHTTP (Британия, США, Франция), Hysteria2
   (Нидерланды); несколько сайтов, видео.
2. Из российских сетей (домашний провайдер и мобильный): Hysteria2 через
   встроенный клиент Xray — если не работает, это сигнал к фазе 2.
3. REALITY против узла на Xray ≥ 26.9.8.
4. NAT64: Mac → Общий доступ → с Option (i) у «Общий интернет» → «Create NAT64
   Network»; iPhone на этой сети: подписка скачивается, подключение
   поднимается ко всем локациям (и к узлу, записанному IPv4-адресом), сайты
   через узел открываются, ya.ru и proxysvpn.com/privacy тоже; в Console
   строка `IPv6-only network: …`. На сети с IPv4 этой строки быть не должно.
5. Двухчасовой прогон на максимальной скорости со сменой сетей (Wi-Fi ↔
   сотовая, самолётный режим): в Console `footprint` расширения не выше
   45 МиБ, расширение не перезапускается.
6. Страница проверки утечек: нет IPv6 и нет DNS мимо туннеля (кроме
   российских имён через 77.88.8.8).
7. Apple TV — когда появится tvOS-таргет (срез во фреймворке уже есть).
