# ProxysVPN Desktop v0.3.4-beta

🇷🇺 Steam Deck: сборка AppImage и управление геймпадом. Нативная сборка для Windows на ARM. Ссылки открываются только на наши сайты. Плюс «Вставить другую ссылку» и название WataFast.

🇬🇧 Steam Deck: an AppImage build and controller support. A native Windows on ARM build. Links open only our own sites. Plus "Paste another link" and the name WataFast.

## 🇷🇺 Установка

Сверьте SHA-256 файла со строкой в `SHA256SUMS.txt`. Подписи кода пока нет, поэтому каждая система один раз предупреждает.

- **macOS 12+ (Apple Silicon):** откройте `ProxysVPN_0.3.4_aarch64.dmg`, перетащите ProxysVPN на ярлык Applications, затем один раз в Терминале `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app` (или «Всё равно открыть» в Системных настройках → Конфиденциальность и безопасность). При запуске приложение спрашивает пароль администратора.
- **Windows 10/11 (x64):** скачайте `ProxysVPN_0.3.4_x64-setup.exe`. Microsoft Edge может задержать загрузку: кнопка сохранения спрятана за стрелкой справа от «Удалить» → «Всё равно сохранить». Затем запустите файл; SmartScreen: «Подробнее» → «Выполнить в любом случае». При каждом запуске — запрос прав администратора (UAC).
- **Windows 10/11 на ARM (Snapdragon, Surface Pro X):** `ProxysVPN_0.3.4_arm64-setup.exe`, шаги те же. Файл x64 там тоже работает, через эмуляцию.
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.4_amd64.deb`. Нужен сеанс с агентом polkit. Удаление: `sudo apt remove proxys-vpn`.
- **Steam Deck (AppImage, Linux x86_64):** `ProxysVPN_0.3.4_amd64.AppImage`. Сделайте файл исполняемым (`chmod +x`), первый запуск и первое подключение — в режиме рабочего стола. Шаги, игровой режим и управление — в docs/STEAMDECK.md.

## 🇷🇺 Что нового

- **Steam Deck.** Сборка в виде AppImage для Linux x86_64: тот же туннель, что в .deb. Первое подключение — в режиме рабочего стола: приложение копирует своего помощника в папку root (`/home/.proxysvpn`) с одним запросом пароля пользователя deck и спрашивает, разрешить ли игровому режиму подключаться без пароля. Передумать можно в «Ещё» → «Удалить системные файлы». После обновления приложения первое подключение снова нужно сделать в режиме рабочего стола. В образ вложен список библиотек LGPL с адресами их исходного кода.
- **Геймпад и полный экран на Steam Deck.** В игровом режиме окно занимает весь экран, а текст и элементы крупнее. По окну можно ходить без мыши и касания: крестовина переносит кольцо фокуса, A нажимает, B закрывает верхний экран. Код привязки вводится через экранную клавиатуру (Steam + X). Раскладку Steam Input лучше один раз выбрать, как описано в docs/STEAMDECK.md. Работает только на SteamOS.
- **Windows на ARM.** Нативный установщик `ProxysVPN_0.3.4_arm64-setup.exe`: движки и драйвер Wintun для arm64, ничего из туннеля не работает в эмуляции.
- **«Вставить другую ссылку».** Если ссылка занята другим устройством, под главной кнопкой появилась тихая вторая строка: она открывает экран добавления ссылки. Раньше единственная кнопка вела в кабинет, а чтобы вставить другую ссылку, приходилось искать «Отвязать это устройство» внизу настроек. Текст называет оба выхода: добавить в кабинете или в боте ещё одно устройство и вставить его ссылку либо сбросить привязку там.
- **«Страны» говорят про обновление.** Строка под списком: список и пинг обновляются каждый раз, когда открывается этот экран. Отдельных кнопок для этого нет.
- **Безопасность: ссылки открываются только на наши сайты.** Приложение принимает любую ссылку подписки, в том числе чужой панели, а адреса кнопки «Поддержка» и сообщения сервиса (`support-url`, `announce-url`) приходят из подписки: чужая подписка могла поставить под нашу кнопку любую страницу. Теперь приложение открывает только страницы по https на наших сайтах и страницу нашего бота в Telegram; чужие адреса из подписки не показываются, а кабинет строится только на наших сайтах.
- **Название — WataFast.** Автоматический выбор и протокол в списке стран пишутся «WataFast», с заглавной F, как на сайтах.
- **Инструкция для Windows** (README) описывает остановку загрузки в Edge: «Перед открытием файла убедитесь, что вы ему доверяете» и где спрятана кнопка «Всё равно сохранить». Там же сказано, почему Windows предупреждает: установщик не подписан сертификатом издателя.

## 🇷🇺 Известные ограничения

- **IPv6 не идёт через туннель** ни на одной системе. Программы, которые сами соединяются по IPv6 (звонки WebRTC, приложения со своим DNS), в сети с IPv6 могут ходить мимо VPN.
- **Windows:** системный DNS пока не направляется в туннель: имена разрешает DNS вашей сети.
- **Steam Deck:** сборка не запускалась на настоящей Steam Deck. Она проверена в симуляции: тот же AppImage в окружении SteamOS 3.7, 3.8 и 3.9 под gamescope — окно занимает весь экран 1280×800, стрелки, Enter и Escape и виртуальный геймпад двигают фокус и нажимают. Не проверено то, что есть только на самой приставке: раскладки Steam Input, экранная клавиатура Steam, подключение без пароля в настоящем игровом режиме, касание и вид на её экране. Что именно проверено и что нет — в docs/STEAMDECK.md.
- **Windows на ARM** (Snapdragon, Surface Pro X): нативная сборка собрана и проверена в CI на ARM-машине, но на настоящем ARM-компьютере ещё не запускалась, поэтому не подтверждено, что она убирает задержку подключения. Не проверялось и то, что на компьютере не ARM64 её установщик останавливается до первой страницы и называет файл x64. Сборка x64 работает через эмуляцию, и подключение может занимать от 30 секунд до 2 минут, потому что в эмуляции драйвер Wintun не может удалить адаптер от прошлого запуска.
- **Windows, обновление с 0.3.2 и старше:** их деинсталлятор не умеет закрывать приложение. Если установщик скажет, что не может закрыть ProxysVPN, выйдите из приложения через трей и нажмите «Повторить».
- **Нет подписи кода и автообновления.**
- **macOS:** только Apple Silicon. Всё приложение работает от root: движки запускаются из копии в папке root, а сам исполняемый файл — из бандла в «Программах». После сбоя движки могут работать до следующего запуска ProxysVPN, и он их остановит.
- **Linux:** только x86_64.

## 🇬🇧 Install

Compare each file's SHA-256 with `SHA256SUMS.txt`. Nothing is code-signed yet, so each system warns once.

- **macOS 12+ (Apple Silicon):** open `ProxysVPN_0.3.4_aarch64.dmg`, drag ProxysVPN onto the Applications shortcut, then once in Terminal `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app` (or "Open Anyway" in System Settings → Privacy & Security). The app asks for the administrator password at launch.
- **Windows 10/11 (x64):** download `ProxysVPN_0.3.4_x64-setup.exe`. Microsoft Edge may hold the download: the way to keep the file is hidden behind the arrow to the right of "Delete" → "Keep anyway". Then run the file; SmartScreen: "More info" → "Run anyway". Every launch asks for administrator rights (UAC).
- **Windows 10/11 on ARM (Snapdragon, Surface Pro X):** `ProxysVPN_0.3.4_arm64-setup.exe`, the same steps. The x64 file works there too, under emulation.
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.4_amd64.deb`. Needs a session with a polkit agent. Remove with `sudo apt remove proxys-vpn`.
- **Steam Deck (AppImage, x86_64 Linux):** `ProxysVPN_0.3.4_amd64.AppImage`. Make the file executable (`chmod +x`); the first start and the first connect happen in Desktop Mode. The steps, Gaming Mode and the controller are in docs/STEAMDECK.md.

## 🇬🇧 What's new

- **Steam Deck.** An AppImage build for x86_64 Linux: the same tunnel as the .deb. The first connect happens in Desktop Mode: the app copies its root helper into a root-owned folder (`/home/.proxysvpn`) with one password prompt for the deck user, and asks whether Gaming Mode may connect without a password. You can change your mind in "More" → "Remove system files". After the app is updated, the first connect has to be made in Desktop Mode again. The image carries a list of the LGPL libraries it bundles, with the addresses of their source.
- **Controller and a full screen on the Steam Deck.** In Gaming Mode the window fills the screen, and the text and controls are larger. The window can be used without a mouse or a touch: the D-pad moves a focus ring, A presses, B closes the top screen. The pair code is typed with the on-screen keyboard (Steam + X). It is best to pick the Steam Input layout once, as docs/STEAMDECK.md describes. SteamOS only.
- **Windows on ARM.** A native installer, `ProxysVPN_0.3.4_arm64-setup.exe`: arm64 engines and Wintun driver, nothing of the tunnel runs under emulation.
- **"Paste another link".** When a link is taken by another device, a quiet second line under the main button opens the add-link screen. Before, the only button led to the cabinet, and to try another link you had to find "Unlink this device" at the bottom of the settings. The text names both ways out: add one more device in the cabinet or the bot and paste its link, or reset the binding there.
- **Countries says it refreshes.** One line under the list: the list and the ping are refreshed every time this screen is opened. There is no button for it.
- **Security: links open only our own sites.** The app takes any subscription link, a third-party panel's included, and the addresses behind the "Support" button and the service's notice (`support-url`, `announce-url`) come from the subscription: a foreign subscription could put any page under our button. The app now opens only https pages of our own sites and our bot's page in Telegram; a foreign address from a subscription is not offered, and the cabinet is built only on our sites.
- **The name is WataFast.** The automatic choice and the protocol in the list of countries are written "WataFast", with a capital F, as on the sites.
- **The Windows install guide** (README) covers Edge holding the download ("Make sure you trust the file before you open it") and where the "Keep anyway" button is hidden. It also says why Windows warns: the installer is not signed with a publisher certificate.

## 🇬🇧 Known limitations

- **IPv6 does not go through the tunnel** on any system. Programs that connect over IPv6 on their own (WebRTC calls, apps with their own DNS) can go outside the VPN on an IPv6 network.
- **Windows:** the system DNS is not sent into the tunnel yet; your network's DNS resolves names.
- **Steam Deck:** the build has not been run on a real Steam Deck. It was checked in a simulation: the same AppImage in the userspace of SteamOS 3.7, 3.8 and 3.9 under gamescope — the window fills the 1280×800 screen, and the arrow keys, Enter, Escape and a virtual controller move the focus and press. Not verified is what exists only on the device itself: Steam Input layouts, Steam's on-screen keyboard, connecting without a password in the real Gaming Mode, touch, and how it looks on its screen. What was checked and what was not is in docs/STEAMDECK.md.
- **Windows on ARM** (Snapdragon, Surface Pro X): the native build is built and checked in CI on an ARM machine, but it has not been run on a real ARM PC, so it is not confirmed that it removes the connect delay. Nor has it been checked that on a PC that is not ARM64 its installer stops before the first page and names the x64 file. The x64 build runs under emulation, and connecting can take 30 seconds to 2 minutes because the Wintun driver cannot remove the previous run's adapter there.
- **Windows, upgrading from 0.3.2 or older:** their uninstaller cannot close the app. If the installer says it cannot close ProxysVPN, quit the app from the tray and press "Retry".
- **No code signing and no automatic updates.**
- **macOS:** Apple Silicon only. The whole app runs as root: the engines run from a copy in a root-owned folder, the app's own executable from the bundle in Applications. After a crash the engines may keep running until ProxysVPN is opened again, which stops them.
- **Linux:** x86_64 only.

---

# ProxysVPN Desktop v0.3.3-beta

🇷🇺 Windows 10/11 (x64) после живой проверки: подключение на Windows исправлено. Плюс исправления, найденные при проверке в виртуальных машинах.

🇬🇧 Windows 10/11 (x64) after a live check: connecting on Windows is fixed. Plus the fixes found while testing in virtual machines.

## 🇷🇺 Установка

Сверьте SHA-256 файла со строкой в `SHA256SUMS.txt`. Подписи кода пока нет, поэтому каждая система один раз предупреждает.

- **macOS 12+ (Apple Silicon):** откройте `ProxysVPN_0.3.3_aarch64.dmg`, перетащите ProxysVPN на ярлык Applications, затем один раз в Терминале `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app` (или «Всё равно открыть» в Системных настройках → Конфиденциальность и безопасность). При запуске приложение спрашивает пароль администратора.
- **Windows 10/11 (x64):** запустите `ProxysVPN_0.3.3_x64-setup.exe`; SmartScreen: «Подробнее» → «Выполнить в любом случае». При каждом запуске — запрос прав администратора (UAC).
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.3_amd64.deb`. Нужен сеанс с агентом polkit. Удаление: `sudo apt remove proxys-vpn`.

## 🇷🇺 Что исправлено

- **Windows: подключение.** Приложение настраивало адаптер туннеля раньше, чем Windows поднимала его с адресом IPv4, и подключение падало с «Элемент не найден». Теперь оно ждёт, пока адаптер поднимется, находит его по номеру, а не по имени (имя мог держать адаптер, оставшийся от прошлого запуска), и при отказе повторяет настройку.
- **Переход на Hysteria2 и починка на медленном компьютере.** Движок считался запущенным раньше, чем начинал принимать соединения, и проверка сразу после перезапуска могла ошибиться. Теперь приложение ждёт xray до 8 секунд, hysteria до 10.
- **Задержки в «Странах» при включённом VPN.** Раньше у части локаций было 1–5 мс: замер уходил в сам туннель. Теперь каждая строка меряется мимо туннеля, через физическую сеть. Это же чинит выбор самой быстрой локации при включённом VPN. Если замерить мимо туннеля нельзя, показывается последнее настоящее число с отметкой, когда его сняли.
- **Главный экран после смены локации** сразу показывает новую локацию.
- **Английский интерфейс:** слова рядом с названиями локаций («резерв», «квота», «ГБ/мес» и другие) теперь переводятся.
- **Windows: установщик сам закрывает запущенное приложение** перед установкой и удалением.
- **Журнал:** при сбое системной команды в журнал попадает её ответ, а не только код выхода; на Windows в правильной кодировке.

## 🇷🇺 Известные ограничения

- **IPv6 не идёт через туннель** ни на одной системе. Программы, которые сами соединяются по IPv6 (звонки WebRTC, приложения со своим DNS), в сети с IPv6 могут ходить мимо VPN.
- **Windows:** системный DNS пока не направляется в туннель: имена разрешает DNS вашей сети.
- **Windows на ARM** (Snapdragon, Surface Pro X): сборка x64 работает через эмуляцию, и подключение может занимать от 30 секунд до 2 минут, потому что в эмуляции драйвер Wintun не может удалить адаптер от прошлого запуска. Нативная сборка для ARM запланирована.
- **Windows, обновление с более старой версии:** её деинсталлятор не умеет закрывать приложение. Если установщик скажет, что не может закрыть ProxysVPN, выйдите из приложения через трей и нажмите «Повторить».
- **Нет подписи кода и автообновления.**
- **macOS:** только Apple Silicon. Всё приложение работает от root: движки запускаются из копии в папке root, а сам исполняемый файл — из бандла в «Программах». После сбоя движки могут работать до следующего запуска ProxysVPN, и он их остановит.
- **Linux:** только x86_64.

## 🇬🇧 Install

Compare each file's SHA-256 with `SHA256SUMS.txt`. Nothing is code-signed yet, so each system warns once.

- **macOS 12+ (Apple Silicon):** open `ProxysVPN_0.3.3_aarch64.dmg`, drag ProxysVPN onto the Applications shortcut, then once in Terminal `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app` (or "Open Anyway" in System Settings → Privacy & Security). The app asks for the administrator password at launch.
- **Windows 10/11 (x64):** run `ProxysVPN_0.3.3_x64-setup.exe`; SmartScreen: "More info" → "Run anyway". Every launch asks for administrator rights (UAC).
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.3_amd64.deb`. Needs a session with a polkit agent. Remove with `sudo apt remove proxys-vpn`.

## 🇬🇧 What's fixed

- **Windows: connecting.** The app configured the tunnel adapter before Windows had brought it up with an IPv4 address, and connecting failed with "Element not found". It now waits for the adapter to come up, finds it by its index rather than by name (a leftover adapter from an earlier run could hold the name) and retries the setup when Windows refuses it.
- **Switching to Hysteria2 and repair on a slow computer.** An engine counted as started before it accepted connections, so the check right after a restart could fail. The app now waits up to 8 seconds for xray and up to 10 for hysteria.
- **Latency in Countries while the VPN is on.** Some locations used to show 1-5 ms because the tunnel itself was measured. Every row is now measured around the tunnel, over the physical network, which also fixes picking the fastest location while the VPN is on. When a row cannot be measured around the tunnel, it shows the last real number with when it was taken.
- **The main screen after a location change** shows the new location right away.
- **English interface:** the words around location names ("reserve", "quota", "GB/mo" and others) are translated now.
- **Windows: the installer closes the running app itself** before installing and uninstalling.
- **Log:** when a system command fails, the log has what it said, not only its exit code; on Windows in the right encoding.

## 🇬🇧 Known limitations

- **IPv6 does not go through the tunnel** on any system. Programs that connect over IPv6 on their own (WebRTC calls, apps with their own DNS) can go outside the VPN on an IPv6 network.
- **Windows:** the system DNS is not sent into the tunnel yet; your network's DNS resolves names.
- **Windows on ARM** (Snapdragon, Surface Pro X): the x64 build runs under emulation, and connecting can take 30 seconds to 2 minutes because the Wintun driver cannot remove the previous run's adapter there. A native ARM build is planned.
- **Windows, upgrading from an older version:** its uninstaller cannot close the app. If the installer says it cannot close ProxysVPN, quit the app from the tray and press "Retry".
- **No code signing and no automatic updates.**
- **macOS:** Apple Silicon only. The whole app runs as root: the engines run from a copy in a root-owned folder, the app's own executable from the bundle in Applications. After a crash the engines may keep running until ProxysVPN is opened again, which stops them.
- **Linux:** x86_64 only.

---

# ProxysVPN Desktop v0.3.2-beta

🇷🇺 Первый выпуск для трёх систем: macOS (Apple Silicon), Windows 10/11 (x64) и Linux (.deb, x86_64). Плюс исправления по итогам аудита перед публикацией.

🇬🇧 The first release for three systems: macOS (Apple Silicon), Windows 10/11 (x64) and Linux (.deb, x86_64). Plus the fixes from the pre-release audit.

## 🇷🇺 Установка

Сверьте SHA-256 файла со строкой в `SHA256SUMS.txt` ниже. Подписи кода пока нет, поэтому каждая система один раз предупреждает.

- **macOS 12+ (Apple Silicon):** откройте `ProxysVPN_0.3.2_aarch64.dmg`, перетащите ProxysVPN на ярлык Applications, затем один раз в Терминале `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app` (или «Всё равно открыть» в Системных настройках → Конфиденциальность и безопасность). При запуске приложение спрашивает пароль администратора. Подробно — INSTALL.md.
- **Windows 10/11 (x64):** запустите `ProxysVPN_0.3.2_x64-setup.exe`; SmartScreen: «Подробнее» → «Выполнить в любом случае». При каждом запуске — запрос прав администратора (UAC).
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.2_amd64.deb`. Нужен сеанс с агентом polkit. Удаление: `sudo apt remove proxys-vpn`.

## 🇷🇺 Что исправлено

- **Локации Hysteria2 снова работают.** Переход xray → hysteria привязывался к физическому интерфейсу, и соединение с 127.0.0.1 не устанавливалось — ни трафика, ни DNS.
- **Windows: повторное подключение к тому же узлу.** Маршрут до узла не удалялся (netsh требует имя интерфейса), и следующее подключение, смена локации туда и обратно и перезапуск движка падали до перезагрузки.
- **Окно на маленьких экранах.** На 1366×768 и ноутбуках с масштабом 150 % нижняя кнопка уходила под панель задач. Окно теперь подстраивается под экран и меняет размер.
- **Безопасность:** помощник Linux больше не запускает файл по пути от непривилегированного процесса; на macOS движки запускаются из копии в папке root, а не из бандла пользователя (само приложение — пока из бандла, см. ограничения); журнал от root не идёт по ссылкам ни в одной папке на пути к нему; ссылка подписки только по https и не уходит чужим сайтам; QR-привязка принимает ссылки только наших сайтов; конфигурация hysteria защищена от подстановки ключей.
- **Приватность:** имена открытых сайтов больше не попадают в журнал и «Отчёт для поддержки», а журналы, оставшиеся от 0.3.1, маскируются один раз при первом запуске; на компьютере больше нельзя выбрать «IPv6»/«Оба» (туннель IPv6 не несёт), уведомление о данных говорит об этом; hysteria больше не проверяет обновления у api.hy2.io; настройки на macOS не читаются другими учётными записями.
- **Починка:** проверка во время починки не засчитывается, если туннель лежит (раньше щит мог стать зелёным при трафике мимо VPN); выбор локации во время починки больше не гоняется с ней; после неудачи «Повторить» берёт свежий список серверов, а если ни один сайт не отвечает — подключается по уже полученному; выбор локации во время отключения больше не включает VPN обратно; на Windows и Linux починка снова поднимает туннель после падения tun2socks; сохранённый список Watafast больше не перекрывает отказ подписки («ссылка занята», «нет средств») и не подменяет свежий список, а ответ защиты хостинга (403) больше не стирает его.
- **macOS:** запуск и выход больше не удаляют маршруты другого VPN, а подключение больше не пишет в журнал несуществующий «другой VPN»; если движки не удалось скопировать при запуске, окно говорит об этом прямо; минимальная версия — macOS 12 (движки на ней не работают на 11).
- **Linux:** при смене сети, а также после сбоя или отключения питания отключение возвращает DNS текущей сети, а не старый /etc/resolv.conf; отключение больше не убивает tun2socks других VPN-клиентов; удаление пакета восстанавливает /etc/resolv.conf; без агента polkit приложение говорит об этом прямо.
- **Интерфейс:** названия стран (в окне, трее, всплывающих сообщениях и истории) и трей по-английски в английском интерфейсе; Esc закрывает только верхнее окно; на Windows нет квадратиков вместо флагов; экран лицензий теперь есть и для Windows и Linux: движки, компоненты Rust и JavaScript и GPL-модули Go внутри движков (остальные Go-модули движков под MIT, BSD, Apache-2.0 пока не перечислены поштучно).

## 🇷🇺 Известные ограничения

- **IPv6 не идёт через туннель** ни на одной системе. Программы, которые сами соединяются по IPv6 (звонки WebRTC, приложения со своим DNS), в сети с IPv6 могут ходить мимо VPN.
- **Windows:** системный DNS пока не направляется в туннель — имена разрешает DNS вашей сети.
- **Нет подписи кода и автообновления.**
- **macOS:** после сбоя или принудительного завершения движки могут работать до следующего запуска ProxysVPN (он их и остановит). Всё приложение работает от root: движки запускаются из копии в папке root, но сам исполняемый файл приложения и его лаунчер — из бандла в «Программах», который принадлежит вам, поэтому программа под вашей учётной записью может подменить их до следующего запуска. Установщик .pkg с привилегированным помощником запланирован.
- Шаг починки «перечитать подписку» идёт через туннель, который чинится; если все адреса узлов сменились, нажмите «Повторить» — теперь это загрузит свежий список.
- Windows и Linux проверены вживую на версии 0.3.1 (VLESS); 0.3.2 и локации Hysteria2 на них ещё не проверялись.

## 🇬🇧 Install

Compare each file's SHA-256 with `SHA256SUMS.txt` below. Nothing is code-signed yet, so each system warns once.

- **macOS 12+ (Apple Silicon):** open `ProxysVPN_0.3.2_aarch64.dmg`, drag ProxysVPN onto the Applications shortcut, then once in Terminal `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app` (or "Open Anyway" in System Settings → Privacy & Security). The app asks for the administrator password at launch.
- **Windows 10/11 (x64):** run `ProxysVPN_0.3.2_x64-setup.exe`; SmartScreen: "More info" → "Run anyway". Every launch asks for administrator rights (UAC).
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.2_amd64.deb`. Needs a session with a polkit agent. Remove with `sudo apt remove proxys-vpn`.

## 🇬🇧 What's fixed

- **Hysteria2 locations work again.** The xray → hysteria hop was pinned to the physical interface, so the 127.0.0.1 connection never opened: no traffic, no DNS.
- **Windows: reconnecting to the same node.** The node's host route was never deleted (netsh needs the interface), so the next connect, a there-and-back location change and an engine revive failed until a reboot.
- **The window on small screens.** On 1366×768 and laptops at 150 % the bottom button sat under the taskbar. The window now fits the screen and can be resized.
- **Security:** the Linux helper no longer runs a file path given by an unprivileged process; macOS runs the engines from a copy in a root-owned folder, not from the user's bundle (the app itself still runs from the bundle, see the limitations); root's log writes follow no link in any folder on the way; subscription links are https-only and never sent to other services' sites; QR pairing accepts only links on our own sites; the hysteria config cannot be fed extra keys.
- **Privacy:** the names of sites you open no longer reach the log or the support report, and logs left by 0.3.1 are masked once on the first start; desktops no longer offer "IPv6"/"Both" (the tunnel does not carry IPv6) and the data notice says so; hysteria no longer checks api.hy2.io for updates; macOS settings are no longer readable by other accounts.
- **Repair:** a probe during repair no longer counts when the tunnel is down (the shield could turn green while traffic went outside the VPN); picking a location during repair no longer races the repair; "Retry" after a failure fetches a fresh server list, and connects with the list it already has when no site answers; picking a location during Disconnect no longer turns the VPN back on; on Windows and Linux repair raises the tunnel again after tun2socks died; a cached Watafast list no longer overrides the subscription's refusals ("link in use", "no funds") or a fresh list, and a hosting firewall's 403 no longer deletes it.
- **macOS:** launching and quitting no longer delete another VPN's routes, and connecting no longer logs a phantom "other VPN"; when the engines could not be copied at launch the window says so; the minimum is now macOS 12 (the engines do not run on 11).
- **Linux:** after a network change, a crash or a power cut, Disconnect restores the current network's DNS, not an old /etc/resolv.conf; Disconnect no longer kills other VPN clients' tun2socks; removing the package restores /etc/resolv.conf; a session without a polkit agent is named as such.
- **Interface:** country names (in the window, the tray, the toasts and the history) and the tray in English in the English interface; Esc closes only the top layer; no boxed letters instead of flags on Windows; the licences screen now covers Windows and Linux too: the engines, the Rust and JavaScript components and the GPL Go modules inside the engines (the engines' other Go modules, under MIT, BSD and Apache-2.0, are not listed one by one yet).

## 🇬🇧 Known limitations

- **IPv6 does not go through the tunnel** on any system. Programs that connect over IPv6 on their own (WebRTC calls, apps with their own DNS) can go outside the VPN on an IPv6 network.
- **Windows:** the system DNS is not sent into the tunnel yet — your network's DNS resolves names.
- **No code signing and no automatic updates.**
- **macOS:** after a crash or Force Quit the engines may keep running until ProxysVPN is opened again (which stops them). The whole app runs as root: the engines run from a copy in a root-owned folder, but the app's own executable and its launcher run from the bundle in Applications, which you own, so a program running as your user can replace them before the next launch. A .pkg installer with a privileged helper is planned.
- The repair step that re-reads the subscription goes through the tunnel being repaired; if every node address changed, press "Retry", which now fetches a fresh list.
- Windows and Linux were tried live with 0.3.1 (VLESS); 0.3.2 and Hysteria2 locations have not been run on them yet.

---

# ProxysVPN Desktop v0.3.1-beta

🇷🇺 Новые настройки: способ подключения, подключение при запуске, свои правила для сайтов и уведомления. Плюс исправления по итогам проверки 0.3.0.

🇬🇧 New settings: how to connect, connect on launch, your own site rules and notifications. Plus fixes from testing 0.3.0.

## 🇷🇺 Что нового

- **Способ подключения.** Авто, только XHTTP или только Vision. Если у локации нет выбранного способа, она подключится тем, что есть, и напишет об этом в списке.
- **Подключаться при запуске.** Выключено по умолчанию.
- **Свои правила.** Два списка сайтов: «всегда напрямую» и «всегда через VPN». Они сильнее правил из подписки.
- **Уведомления.** macOS сообщит, если защита пропала, и когда она вернулась.
- Исправлено: двойное «Watafast» в названии локации; сообщение об ошибке ifconfig при каждом подключении; мелькание «не удалось» сразу после подключения; огромные числа в журнале после перезапуска туннеля; долгие ответы DNS в начале сеанса.

## 🇬🇧 What's new

- **How to connect.** Auto, XHTTP only or Vision only. A location without the chosen way still connects with what it has and says so in the list.
- **Connect on launch.** Off by default.
- **Your own rules.** Two site lists, «always direct» and «always through the VPN». They take priority over the subscription's rules.
- **Notifications.** macOS tells you when protection drops and when it is back.
- Fixed: «Watafast» shown twice in a location name; an ifconfig error logged on every connect; a brief «failed» flash right after connecting; huge byte counts in the log after a tunnel restart; slow DNS answers at the start of a session.

---

# ProxysVPN Desktop v0.3.0-beta

🇷🇺 Watafast: приложение само выбирает способ подключения, помнит, что работает в вашей сети, и подключается, даже когда сайты сервиса не открываются.

🇬🇧 Watafast: the app picks the way to connect by itself, remembers what works on your network, and connects even when the service's sites do not open.

## 🇷🇺 Что нового

- **Watafast.** Локации на XHTTP (Британия, США, Франция) в списке называются «Watafast», автоматический выбор — тоже «Watafast».
- **Память по сети.** Приложение запоминает, какая локация сработала дома, на работе, в мобильной сети, и в следующий раз начинает с неё.
- **Гонка в новой сети.** В незнакомой сети два варианта подключения стартуют одновременно, побеждает первый ответивший.
- **Подписанный список серверов.** Приложение получает от сервиса список, подписанный ключом сервиса, и хранит последний проверенный. Если ни один сайт сервиса не открывается, оно подключается по сохранённому списку. Поддельный список отвергается.
- Исправлено: смена сети без отключения больше не путает память по сетям; служебный локальный порт гонки закрывается сразу после выбора и, пока открыт, требует пароль.

## 🇬🇧 What's new

- **Watafast.** XHTTP locations (Britain, USA, France) are shown as «Watafast», and so is the automatic choice.
- **Per-network memory.** The app remembers which location worked at home, at work and on mobile data, and starts there next time.
- **A race on a new network.** On a network it does not know yet, two ways to connect start at once and the first to answer wins.
- **Signed server list.** The app gets a list signed with the service's key and keeps the last verified one. When none of the service's sites opens, it connects from the stored list; a forged list is rejected.
- Fixed: changing networks without disconnecting no longer mixes up the per-network memory; the race's local helper port closes right after the choice and requires a password while it exists.

---

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
