# ProxysVPN Desktop v0.3.7-beta

🇷🇺 Приложение принимает только ссылку подписки ProxysVPN. Страна, выбранная, пока интернета нет, включается, когда сеть возвращается. Образ для macOS подписан и заверен Apple: открывается двойным щелчком, без команды в Терминале. Знак приложения чёткий в любом размере.

🇬🇧 The app takes a ProxysVPN subscription link only. A country picked while the internet is away is the one protection comes back on when the network returns. The macOS image is signed and notarized by Apple: it opens with a double click, with no Terminal command. The app's mark is sharp at every size.

## 🇷🇺 Установка

Сверьте SHA-256 файла со строкой в `SHA256SUMS.txt`. Образ для macOS подписан сертификатом Apple Developer ID (Watafast LLC) и заверен Apple. У файлов для Windows и Linux подписи кода пока нет, поэтому эти системы один раз предупреждают.

- **macOS 12+ (Apple Silicon):** откройте `ProxysVPN_0.3.7_aarch64.dmg`, перетащите ProxysVPN на ярлык Applications, при первом запуске нажмите «Открыть». Команда в Терминале больше не нужна. При запуске приложение спрашивает пароль администратора.
- **Windows 10/11 (x64):** скачайте `ProxysVPN_0.3.7_x64-setup.exe`. Microsoft Edge может задержать загрузку: кнопка сохранения спрятана за стрелкой справа от «Удалить» → «Всё равно сохранить». Затем запустите файл; SmartScreen: «Подробнее» → «Выполнить в любом случае». При каждом запуске — запрос прав администратора (UAC).
- **Windows 10/11 на ARM (Snapdragon, Surface Pro X):** `ProxysVPN_0.3.7_arm64-setup.exe`, шаги те же. Файл x64 там тоже работает, через эмуляцию.
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.7_amd64.deb`. Нужен сеанс с агентом polkit. Удаление: `sudo apt remove proxys-vpn`.
- **Steam Deck (AppImage, Linux x86_64):** `ProxysVPN_0.3.7_amd64.AppImage`. Сделайте файл исполняемым (`chmod +x`), первый запуск и первое подключение — в режиме рабочего стола. Шаги, игровой режим и управление — в docs/STEAMDECK.md.

## 🇷🇺 Что нового

- **Только подписка ProxysVPN.** Приложение принимает ссылку только с наших адресов. На ссылку другого сервиса оно отвечает сразу: «Это не ссылка ProxysVPN. Возьмите её в боте: «📡 Мои устройства»» — и никуда по ней не обращается. Раньше такая ссылка принималась, а приложение потом писало, что серверы недоступны, и повторяло попытки без конца. Если чужая ссылка осталась от прежней версии, экран скажет «Эта ссылка не подходит», а «Что делать» → «Добавить ссылку» откроет экран, где её можно заменить.
- **Образ для macOS подписан и заверен Apple.** Образ открывается двойным щелчком, а при первом запуске macOS один раз спрашивает, открыть ли приложение, загруженное из интернета. Шаг с `xattr` в Терминале нужен только версиям 0.3.6 и старше.
- **Строка «Новости в Telegram».** В «Ещё» → «О приложении», сразу после «Поддержки»: открывает наш новостной канал в браузере.
- **Знак приложения чёткий в любом размере.** Щит на главном экране, значок окна, значок в трее и на панели задач нарисованы заново, без ступенек на краях.

## 🇷🇺 Что исправлено

- **Страна, выбранная без интернета.** Если выбрать страну, пока приложение пишет «Нет интернета», защита после возврата сети включается на выбранной стране. Раньше она возвращалась на прежний сервер, а в списке закреплённой значилась другая страна.
- **Страна, выбранная в первые секунды обрыва.** То же для выбора, сделанного, когда сеть уже пропала, а экран ещё пишет «Защищено»: приложение ждёт возврата сети и переводит защиту на выбранную страну. Раньше оно отвечало «Не удалось переключить страну» и возвращалось на прежний сервер.
- **«Германия · подключено» — только когда это так.** Строка о подключении к выбранной стране показывается, только пока защита действительно стоит на ней.

## 🇷🇺 Известные ограничения

- **Проверка этого выпуска:** код этого выпуска запускался вживую 7 октября 2026 года на четырёх системах, в сборках с прежним номером версии. Windows 11 на ARM в виртуальной машине (установщик arm64, и установщик x64 под эмуляцией — поверх выпуска 0.3.6): отказ чужой ссылке и имени-двойнику, замена чужой ссылки, оставшейся от 0.3.6, подключение, выбор страны при пропавшей сети, закрытые адреса сайта, знак приложения. Debian 13 в виртуальной машине (.deb поверх выпуска 0.3.6 и AppImage): то же, кроме закрытых адресов. macOS 27 на настоящем компьютере с Apple Silicon (подписанный и заверенный образ): установка перетаскиванием, первый запуск, подключение, два отключения Wi-Fi с выбором страны. Файлы выпуска собраны из того же кода с номером 0.3.7; перед публикацией каждый из пяти ставился поверх прежней сборки и подключался. Не проверялось: настоящий компьютер x64, настоящая Steam Deck, сеть с IPv6; на macOS — экраны отказа чужой ссылке и выбор страны под надписью «Нет интернета» (см. ниже).
- **Выбранная страна при выключенной защите:** строку «Страна: …» окно помнит само, пока приложение запущено. После перезапуска приложения выбор нужно сделать заново.
- **После возврата сети:** если сеть на компьютере пропала при включённой защите и потом вернулась, строка «Подтвердить не удалось» может держаться до 5 минут при открытом окне и до 15 при скрытом, хотя VPN уже работает. Трафик всё это время идёт через VPN. В 0.3.6 здесь было написано «до минуты» — проверка на настоящем Wi-Fi показала больше.
- **Закрытые адреса сайта:** при подготовке этого выпуска часть прогнали вживую в виртуальной машине Windows 11, где наши адреса закрывались искусственно (соединение висит или отклоняется): при закрытом основном адресе отвечает запасной, при всех закрытых приложение включается по сохранённому списку. В настоящей заблокированной сети и на других системах вживую не проверялась. Если приложение ещё ни разу не получало список серверов, а все наши адреса в вашей сети закрыты, включаться ему не по чему: войдите и один раз включите защиту в сети, где сайт открывается.
- **«Нет интернета» на Mac с виртуальными машинами:** пока на Mac запущена или стоит на паузе виртуальная машина (замечено с UTM), приложение принимает её внутреннюю сеть за подключение и при пропавшем интернете пишет «Подтвердить не удалось» вместо «Нет интернета». На защиту это не влияет.
- **IPv6:** у серверов нет выхода по IPv6, сайты открываются по IPv4. Более точный маршрут IPv6, который раздаёт сам роутер, сильнее нашего: строка «Утечек нет» такой случай показывает, но не исправляет. Если система не дала направить IPv6 в туннель (так бывает, когда IPv6 в ней выключен), приложение всё равно подключается; на Windows оно пробует ещё несколько раз примерно в первые 40 секунд после подключения. В настоящей сети с IPv6 не проверялось.
- **«Утечек нет»:** строка показывает состояние на момент проверки. То, что изменилось позже, она увидит при следующей проверке.
- **Windows:** мимо VPN имена всё ещё могут идти, если в настройках туннеля выбран системный резолвер или в самой Windows включён DNS поверх HTTPS. Имена устройств домашней сети (LLMNR, mDNS, NetBIOS) не закрываются.
- **macOS и Linux:** DNS в строке «Утечек нет» не проверяется, поэтому зелёной она там не бывает.
- **Пауза:** не проверена на настоящем сне и пробуждении компьютера и на настоящей смене Wi-Fi — только на их подобии в виртуальных машинах: смена сети — при подготовке 0.3.6 (Debian), сон — при подготовке 0.3.5. Если на миг не читается адрес роутера, приложение принимает это за смену сети: пауза кончится раньше, защита вернётся.
- **Починка соединения и закреплённая страна:** починка может перевести защиту на другую страну, даже если страна закреплена вручную; закрепление остаётся, и следующее включение вернётся на неё.
- **Щит и кнопки под ним:** первые 0,6 секунды после того, как кнопка сменилась или сдвинулась (например, «Включить» стала «Отменой», появился экран паузы, закрылся список стран или панель поверх экрана), нажатия, которые включают или выключают защиту, не действуют — так случайный второй щелчок и нажатие, которое метило в другое место, ничего не отменяют. Если нажатие не сработало, нажмите ещё раз.
- **Steam Deck:** сборка не запускалась на настоящей Steam Deck. В симуляции (окружение SteamOS 3.7, 3.8 и 3.9 под gamescope) проверен AppImage, собранный из кода этой версии (0.3.7). Не проверено то, что есть только на самой приставке: раскладки Steam Input, экранная клавиатура Steam, подключение без пароля в настоящем игровом режиме, касание и вид на её экране. Что именно проверено и что нет — в docs/STEAMDECK.md.
- **Windows, обновление с 0.3.2 и старше:** их деинсталлятор не умеет закрывать приложение. Если установщик скажет, что не может закрыть ProxysVPN, выйдите из приложения через трей и нажмите «Повторить».
- **Windows, меню в трее:** если меню открыто в тот момент, когда пауза заканчивается по времени, оно закрывается само, а на месте его угла до следующего щелчка остаётся белый квадратик. Ни на что не влияет.
- **Подпись кода и обновления:** образ для macOS подписан и заверен Apple; у файлов для Windows и Linux подписи кода нет. Автообновления нет.
- **macOS:** только Apple Silicon. Всё приложение работает от root: движки запускаются из копии в папке root, а сам исполняемый файл — из бандла в «Программах». После сбоя движки могут работать до следующего запуска ProxysVPN, и он их остановит. Щелчок по кнопке в окне приложения, когда активно другое приложение, только выводит окно вперёд: нажмите кнопку ещё раз.
- **Linux:** только x86_64. Окно пароля при первом подключении AppImage вживую не проверено: в Debian 13, где шла проверка, права администратора выдаются без запроса.
- **Linux без systemd-resolved:** если приложение и его служебный процесс убиты принудительно в один момент (например, `kill -9` или нехватка памяти), в `/etc/resolv.conf` остаётся резолвер туннеля, и сайты не открываются по имени, пока защита выключена. Включите защиту один раз: с ней имена работают, а при выключении файл возвращается к прежнему виду.

## 🇬🇧 Install

Compare each file's SHA-256 with `SHA256SUMS.txt`. The macOS image is signed with an Apple Developer ID certificate (Watafast LLC) and notarized by Apple. The Windows and Linux files are not code-signed yet, so those systems warn once.

- **macOS 12+ (Apple Silicon):** open `ProxysVPN_0.3.7_aarch64.dmg`, drag ProxysVPN onto the Applications shortcut, and press "Open" at the first launch. The Terminal command is no longer needed. The app asks for the administrator password at launch.
- **Windows 10/11 (x64):** download `ProxysVPN_0.3.7_x64-setup.exe`. Microsoft Edge may hold the download: the way to keep the file is hidden behind the arrow to the right of "Delete" → "Keep anyway". Then run the file; SmartScreen: "More info" → "Run anyway". Every launch asks for administrator rights (UAC).
- **Windows 10/11 on ARM (Snapdragon, Surface Pro X):** `ProxysVPN_0.3.7_arm64-setup.exe`, the same steps. The x64 file works there too, under emulation.
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.7_amd64.deb`. Needs a session with a polkit agent. Remove with `sudo apt remove proxys-vpn`.
- **Steam Deck (AppImage, x86_64 Linux):** `ProxysVPN_0.3.7_amd64.AppImage`. Make the file executable (`chmod +x`); the first start and the first connect happen in Desktop Mode. The steps, Gaming Mode and the controller are in docs/STEAMDECK.md.

## 🇬🇧 What's new

- **A ProxysVPN subscription only.** The app takes a link only on our own addresses. To another service's link it answers at once: "This is not a ProxysVPN link. Get it in the bot: «📡 My devices»", and it sends nothing to that address. Before, such a link was taken, and the app then said the servers could not be reached and went on retrying without end. If a foreign link is left over from an earlier version, the screen says "This link does not fit", and "What to do" → "Add link" opens the screen where it can be replaced.
- **The macOS image is signed and notarized by Apple.** The image opens with a double click, and at the first launch macOS asks once whether to open an app downloaded from the internet. The `xattr` step in Terminal is needed only by versions 0.3.6 and older.
- **A "News on Telegram" row.** In "More" → "About", right after "Support": it opens our news channel in the browser.
- **The app's mark is sharp at every size.** The shield on the main screen, the window icon, the tray icon and the taskbar icon are redrawn, with no steps along the edges.

## 🇬🇧 What's fixed

- **A country picked while offline.** If you pick a country while the app says "No internet", protection comes back on the picked country when the network returns. Before, it came back on the previous server while the list showed another country as pinned.
- **A country picked in the first seconds of an outage.** The same for a pick made when the network is already gone but the screen still says "Protected": the app waits for the network and moves protection to the picked country. Before, it answered "Could not switch the country" and came back on the previous server.
- **"Germany · connected" only when it is so.** The line about being connected to the picked country is shown only while protection really stands on it.

## 🇬🇧 Known limitations

- **How this release was checked:** the code of this release was run live on 7 October 2026 on four systems, in builds that still carried the previous version number. Windows 11 on ARM in a virtual machine (the arm64 installer, and the x64 installer under emulation over the released 0.3.6): the refusal of another service's link and of a look-alike name, replacing a foreign link left by 0.3.6, connecting, picking a country with the network gone, closed site addresses, the app's mark. Debian 13 in a virtual machine (the .deb over the released 0.3.6, and the AppImage): the same, except the closed addresses. macOS 27 on a real Apple Silicon computer (the signed and notarized image): install by drag, the first launch, connecting, two Wi-Fi outages with a country picked in each. The release files are built from the same code with the number 0.3.7; before publication each of the five was installed over the previous build and connected. Not checked: a real x64 computer, a real Steam Deck, a network with IPv6; on macOS, the screens that refuse a foreign link and a pick made under "No internet" (see below).
- **A picked country with protection off:** the window itself remembers the line "Country: …" for as long as the app runs. After the app is restarted the country has to be picked again.
- **After the network returns:** if the computer lost its network with protection on and then got it back, the line "Could not confirm" can stay for up to 5 minutes with the window open and up to 15 with it hidden, although the VPN already works. Traffic goes through the VPN all that time. The 0.3.6 notes said "up to a minute" here; a check on a real Wi-Fi showed more.
- **Closed site addresses:** while this release was prepared the part was run live in a Windows 11 virtual machine where our addresses were closed on purpose (a connection that hangs, or one that is refused): with the main address closed a reserve one answers, with all of them closed the app turns on from the saved list. It has not been tried on a real blocked network, or live on the other systems. If the app has never fetched the list of servers and all our addresses are closed on your network, it has nothing to turn on from: sign in and turn protection on once on a network where the site opens.
- **"No internet" on a Mac with virtual machines:** while a virtual machine is running or paused on the Mac (seen with UTM), the app takes its internal network for a connection and, with the internet gone, says "Could not confirm" instead of "No internet". Protection is not affected.
- **IPv6:** the servers have no IPv6 exit, sites open over IPv4. A more specific IPv6 route handed out by the router itself wins over ours: the "No leaks" row shows that case but does not repair it. If the system does not let IPv6 be sent into the tunnel (as when IPv6 is switched off in it), the app connects all the same; on Windows it tries a few more times in about the first 40 seconds after a connect. Not tried on a real network with IPv6.
- **"No leaks":** the row shows the state at the moment of the check. What changed later it sees at the next check.
- **Windows:** names can still go past the VPN if the system resolver is chosen in the tunnel settings or DNS over HTTPS is switched on in Windows itself. Names of devices on the home network (LLMNR, mDNS, NetBIOS) are not covered.
- **macOS and Linux:** the "No leaks" row does not check DNS, so it is never green there.
- **Pause:** not checked on a real sleep and wake of a computer or on a real change of Wi-Fi — only on their likeness in virtual machines: a change of network while 0.3.6 was prepared (Debian), sleep while 0.3.5 was prepared. If the router's address cannot be read for a moment, the app takes that for a change of network: the pause ends early and protection comes back.
- **Repair of the connection and a pinned country:** a repair can move protection to another country even when a country is pinned by hand; the pin stays, and the next connect returns to it.
- **The shield and the buttons under it:** for the first 0.6 seconds after a button has changed or moved (for example "Turn on" became "Cancel", the paused screen appeared, the country list or a panel over the screen closed), presses that turn protection on or off do nothing, so that a stray second click, or a press aimed at something else, cancels nothing. If a press did not work, press again.
- **Steam Deck:** the build has not been run on a real Steam Deck. The simulation (the userspace of SteamOS 3.7, 3.8 and 3.9 under gamescope) was run on an AppImage built from the code of this version (0.3.7). Not verified is what exists only on the device itself: Steam Input layouts, Steam's on-screen keyboard, connecting without a password in the real Gaming Mode, touch, and how it looks on its screen. What was checked and what was not is in docs/STEAMDECK.md.
- **Windows, upgrading from 0.3.2 or older:** their uninstaller cannot close the app. If the installer says it cannot close ProxysVPN, quit the app from the tray and press "Retry".
- **Windows, the tray menu:** if the menu is open at the moment a pause ends by its timer, it closes by itself, and a small white square stays where its corner was until the next click. It affects nothing.
- **Code signing and updates:** the macOS image is signed and notarized by Apple; the Windows and Linux files are not code-signed. There are no automatic updates.
- **macOS:** Apple Silicon only. The whole app runs as root: the engines run from a copy in a root-owned folder, the app's own executable from the bundle in Applications. After a crash the engines may keep running until ProxysVPN is opened again, which stops them. A click on a button in the app's window while another app is the active one only brings the window forward: press the button again.
- **Linux:** x86_64 only. The password window of the AppImage's first connect has not been checked live: on Debian 13, where the checks ran, administrator rights are granted without asking.
- **Linux without systemd-resolved:** if the app and its helper process are both killed by force at the same moment (for example `kill -9` or the memory killer), `/etc/resolv.conf` keeps the tunnel's resolver, and sites do not open by name while protection is off. Turn protection on once: names work with it, and turning it off puts the file back as it was.

---

# ProxysVPN Desktop v0.3.6-beta

🇷🇺 Окно сразу отвечает на нажатие: выбранная страна тут же появляется на главном экране, а «Включить» и «Выключить» сразу пишут «Включаем…» и «Выключаем…». Рядом с названием приложения стоит значок «бета». Три исправления после проверки 0.3.5 вживую.

🇬🇧 The window answers a press at once: a picked country appears on the main screen right away, and "Turn on" and "Turn off" say "Turning on…" and "Turning off…" at once. A "beta" mark stands beside the app's name. Three fixes after the live check of 0.3.5.

## 🇷🇺 Установка

Сверьте SHA-256 файла со строкой в `SHA256SUMS.txt`. Подписи кода пока нет, поэтому каждая система один раз предупреждает.

- **macOS 12+ (Apple Silicon):** откройте `ProxysVPN_0.3.6_aarch64.dmg`, перетащите ProxysVPN на ярлык Applications, затем один раз в Терминале `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app` (или «Всё равно открыть» в Системных настройках → Конфиденциальность и безопасность). При запуске приложение спрашивает пароль администратора.
- **Windows 10/11 (x64):** скачайте `ProxysVPN_0.3.6_x64-setup.exe`. Microsoft Edge может задержать загрузку: кнопка сохранения спрятана за стрелкой справа от «Удалить» → «Всё равно сохранить». Затем запустите файл; SmartScreen: «Подробнее» → «Выполнить в любом случае». При каждом запуске — запрос прав администратора (UAC).
- **Windows 10/11 на ARM (Snapdragon, Surface Pro X):** `ProxysVPN_0.3.6_arm64-setup.exe`, шаги те же. Файл x64 там тоже работает, через эмуляцию.
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.6_amd64.deb`. Нужен сеанс с агентом polkit. Удаление: `sudo apt remove proxys-vpn`.
- **Steam Deck (AppImage, Linux x86_64):** `ProxysVPN_0.3.6_amd64.AppImage`. Сделайте файл исполняемым (`chmod +x`), первый запуск и первое подключение — в режиме рабочего стола. Шаги, игровой режим и управление — в docs/STEAMDECK.md.

## 🇷🇺 Что нового

- **Выбранная страна видна сразу.** Нажали на страну в списке — приложение тут же возвращается на главный экран и пишет «Переключаем…» с названием страны, а когда соединение готово — «Германия · подключено» с галочкой. Раньше список несколько секунд стоял без ответа, а о смене страны говорила маленькая серая плашка. Если защита выключена, нажатие только выбирает страну: главный экран показывает «Страна: Германия» и кнопку «Включить». Не получилось переключить — приложение так и скажет: «Не удалось переключить страну».
- **«Включаем…» и «Выключаем…» появляются сразу.** Раньше после нажатия экран ещё секунду-другую показывал прежнее состояние, пока не ответит служебная часть приложения, и было непонятно, принято ли нажатие. Теперь окно пишет это само в момент нажатия, а кнопки на это время гаснут.
- **Значок «бета».** Рядом с названием на главном экране и на первых экранах после установки; в «Ещё» — рядом с номером версии, а в «О приложении» первая строка объясняет, что это значит: приложение ещё дорабатывается, и если что-то работает не так, напишите в поддержку.

## 🇷🇺 Что исправлено

- **«Подтвердить не удалось» больше не висит над работающим VPN.** После починки соединения, смены сети или смены страны эта строка могла оставаться до следующей проверки — до 5 минут при открытом окне и до 15 при скрытом. Теперь приложение переспрашивает через несколько секунд.
- **«Включить», нажатое из другого окна.** Если нажать кнопку, когда активным было другое окно, экран мог ещё 2–20 секунд показывать «Защита выключена», хотя защита уже включалась. Теперь экран показывает то, что происходит на самом деле.
- **Панель «Восстанавливаю соединение» не возвращается после своей кнопки.** После «Отменить и выключить защиту» панель на несколько секунд поднималась снова, пока защита выключалась. Теперь она убирается сразу.

## 🇷🇺 Известные ограничения

- **Проверка этого выпуска:** сборки 0.3.6 запускались вживую на трёх системах. Debian 13 в виртуальной машине (.deb поверх прежней версии и AppImage): всё новое в окне, три исправления, пауза, смена страны и протокола, «Проверка», выход. Windows 11 на ARM в виртуальной машине (установщик arm64 поверх работающей 0.3.5 и установщик x64 под эмуляцией): новое в окне, пауза, смена страны, фильтры DNS, выход; три исправления и первые экраны после установки на Windows не повторялись. macOS на настоящем компьютере с Apple Silicon (установка поверх 0.3.5): новое в окне, смена страны и протокола, пауза из окна и из строки меню, «Проверка», выход. Не проверялось: настоящий компьютер x64, Steam Deck, сеть с IPv6. Проверялись сборки того же кода, сделанные до правки этих заметок; файлы выпуска собраны заново из итогового коммита.
- **Выбранная страна при выключенной защите:** строку «Страна: …» окно помнит само, пока приложение запущено. После перезапуска приложения выбор нужно сделать заново.
- **Страна, выбранная без интернета:** если выбрать страну, пока приложение пишет «Нет интернета», выбор запоминается, но после возврата сети защита может вернуться на прежний сервер, и экран об этом не скажет. Посмотрите на строку под «Защищено»: если страна не та, выберите её в списке ещё раз.
- **Интернет пропал целиком:** если сеть на компьютере пропала при включённой защите и потом вернулась, строка «Подтвердить не удалось» может держаться ещё до минуты после возврата. Трафик всё это время идёт через VPN.
- **Закрытые адреса сайта:** эта часть проверена тестами; в виртуальной машине, где наши адреса закрывались искусственно, её пробовали при подготовке 0.3.5, для 0.3.6 не повторяли; в настоящей заблокированной сети не проверялась. Если приложение ещё ни разу не получало список серверов, а все наши адреса в вашей сети закрыты, включаться ему не по чему: войдите и один раз включите защиту в сети, где сайт открывается.
- **IPv6:** у серверов нет выхода по IPv6, сайты открываются по IPv4. Более точный маршрут IPv6, который раздаёт сам роутер, сильнее нашего: строка «Утечек нет» такой случай показывает, но не исправляет. Если система не дала направить IPv6 в туннель (так бывает, когда IPv6 в ней выключен), приложение всё равно подключается; на Windows оно пробует ещё несколько раз примерно в первые 40 секунд после подключения. В настоящей сети с IPv6 не проверялось.
- **«Утечек нет»:** строка показывает состояние на момент проверки. То, что изменилось позже, она увидит при следующей проверке.
- **Windows:** мимо VPN имена всё ещё могут идти, если в настройках туннеля выбран системный резолвер или в самой Windows включён DNS поверх HTTPS. Имена устройств домашней сети (LLMNR, mDNS, NetBIOS) не закрываются.
- **macOS и Linux:** DNS в строке «Утечек нет» не проверяется, поэтому зелёной она там не бывает.
- **Пауза:** не проверена на настоящем сне и пробуждении компьютера и на настоящей смене Wi-Fi — только на их подобии в виртуальных машинах: смена сети — и в этом выпуске (Debian), сон — при подготовке 0.3.5. Если на миг не читается адрес роутера, приложение принимает это за смену сети: пауза кончится раньше, защита вернётся.
- **Починка соединения и закреплённая страна:** починка может перевести защиту на другую страну, даже если страна закреплена вручную; закрепление остаётся, и следующее включение вернётся на неё.
- **Щит и кнопки под ним:** первые 0,6 секунды после того, как кнопка сменилась или сдвинулась (например, «Включить» стала «Отменой», появился экран паузы, закрылся список стран или панель поверх экрана), нажатия, которые включают или выключают защиту, не действуют — так случайный второй щелчок и нажатие, которое метило в другое место, ничего не отменяют. Если нажатие не сработало, нажмите ещё раз.
- **Steam Deck:** сборка не запускалась на настоящей Steam Deck. В симуляции (окружение SteamOS 3.7, 3.8 и 3.9 под gamescope) проверялся AppImage версии 0.3.4; для 0.3.5 и 0.3.6 симуляцию не повторяли. Не проверено то, что есть только на самой приставке: раскладки Steam Input, экранная клавиатура Steam, подключение без пароля в настоящем игровом режиме, касание и вид на её экране. Что именно проверено и что нет — в docs/STEAMDECK.md.
- **Windows, обновление с 0.3.2 и старше:** их деинсталлятор не умеет закрывать приложение. Если установщик скажет, что не может закрыть ProxysVPN, выйдите из приложения через трей и нажмите «Повторить».
- **Windows, меню в трее:** если меню открыто в тот момент, когда пауза заканчивается по времени, оно закрывается само, а на месте его угла до следующего щелчка остаётся белый квадратик. Ни на что не влияет.
- **Нет подписи кода и автообновления.**
- **macOS:** только Apple Silicon. Всё приложение работает от root: движки запускаются из копии в папке root, а сам исполняемый файл — из бандла в «Программах». После сбоя движки могут работать до следующего запуска ProxysVPN, и он их остановит. Щелчок по кнопке в окне приложения, когда активно другое приложение, только выводит окно вперёд: нажмите кнопку ещё раз.
- **Linux:** только x86_64. Окно пароля при первом подключении AppImage вживую не проверено: в Debian 13, где шла проверка, права администратора выдаются без запроса.
- **Linux без systemd-resolved:** если приложение и его служебный процесс убиты принудительно в один момент (например, `kill -9` или нехватка памяти), в `/etc/resolv.conf` остаётся резолвер туннеля, и сайты не открываются по имени, пока защита выключена. Включите защиту один раз: с ней имена работают, а при выключении файл возвращается к прежнему виду.

## 🇬🇧 Install

Compare each file's SHA-256 with `SHA256SUMS.txt`. Nothing is code-signed yet, so each system warns once.

- **macOS 12+ (Apple Silicon):** open `ProxysVPN_0.3.6_aarch64.dmg`, drag ProxysVPN onto the Applications shortcut, then once in Terminal `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app` (or "Open Anyway" in System Settings → Privacy & Security). The app asks for the administrator password at launch.
- **Windows 10/11 (x64):** download `ProxysVPN_0.3.6_x64-setup.exe`. Microsoft Edge may hold the download: the way to keep the file is hidden behind the arrow to the right of "Delete" → "Keep anyway". Then run the file; SmartScreen: "More info" → "Run anyway". Every launch asks for administrator rights (UAC).
- **Windows 10/11 on ARM (Snapdragon, Surface Pro X):** `ProxysVPN_0.3.6_arm64-setup.exe`, the same steps. The x64 file works there too, under emulation.
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.6_amd64.deb`. Needs a session with a polkit agent. Remove with `sudo apt remove proxys-vpn`.
- **Steam Deck (AppImage, x86_64 Linux):** `ProxysVPN_0.3.6_amd64.AppImage`. Make the file executable (`chmod +x`); the first start and the first connect happen in Desktop Mode. The steps, Gaming Mode and the controller are in docs/STEAMDECK.md.

## 🇬🇧 What's new

- **A picked country shows at once.** Press a country in the list and the app goes straight back to the main screen and says "Switching…" with the country's name, and when the connection is ready, "Germany · connected" with a tick. Before, the list stood for a few seconds with no answer, and a small grey note was all that told of the switch. With protection off a press only picks the country: the main screen shows "Country: Germany" and the "Turn on" button. If the switch did not work, the app says so: "Could not switch the country".
- **"Turning on…" and "Turning off…" appear at once.** After a press the screen used to show the old state for another second or two, until the app's service part answered, and it was not clear whether the press was taken. The window now says it by itself at the moment of the press, and the buttons dim for that time.
- **A "beta" mark.** Beside the name on the main screen and on the first screens after an install; in "More" next to the version number, and the first row of "About" says what it means: the app is still being worked on, and if something does not work as it should, write to support.

## 🇬🇧 What's fixed

- **"Could not confirm" no longer hangs over a working VPN.** After a repair of the connection, a change of network or a change of country that line could stay until the next check: up to 5 minutes with the window open and up to 15 with it hidden. The app now asks again within seconds.
- **"Turn on" pressed from another window.** If you pressed the button while another window was the active one, the screen could go on showing "Protection is off" for another 2–20 seconds although protection was already turning on. The screen now shows what is really happening.
- **The "Restoring the connection" panel does not come back after its own button.** After "Cancel and turn protection off" the panel rose again for a few seconds while protection was being turned off. It now goes away at once.

## 🇬🇧 Known limitations

- **How this release was checked:** the 0.3.6 builds were run live on three systems. Debian 13 in a virtual machine (the .deb over the previous version, and the AppImage): everything new in the window, the three fixes, pause, a change of country and of protocol, "Check", quitting. Windows 11 on ARM in a virtual machine (the arm64 installer over a running 0.3.5, and the x64 installer under emulation): what is new in the window, pause, a change of country, the DNS filters, quitting; the three fixes and the first screens after an install were not repeated on Windows. macOS on a real Apple Silicon computer (an install over 0.3.5): what is new in the window, a change of country and of protocol, pause from the window and from the menu bar, "Check", quitting. Not checked: a real x64 computer, a Steam Deck, a network with IPv6. The builds that were checked were made from the same code before these notes were updated; the release files were built again from the final commit.
- **A picked country with protection off:** the window itself remembers the line "Country: …" for as long as the app runs. After the app is restarted the country has to be picked again.
- **A country picked while offline:** if you pick a country while the app says "No internet", the pick is remembered, but when the network returns protection can come back on the previous server, and the screen does not say so. Look at the line under "Protected": if the country is not the one you picked, pick it in the list again.
- **The internet went away altogether:** if the computer lost its network with protection on and then got it back, the line "Could not confirm" can stay for up to a minute after the return. Traffic goes through the VPN all that time.
- **Closed site addresses:** this part is covered by tests; the trial in a virtual machine where our addresses were closed on purpose was made while 0.3.5 was prepared and was not repeated for 0.3.6; it has not been tried on a real blocked network. If the app has never fetched the list of servers and all our addresses are closed on your network, it has nothing to turn on from: sign in and turn protection on once on a network where the site opens.
- **IPv6:** the servers have no IPv6 exit, sites open over IPv4. A more specific IPv6 route handed out by the router itself wins over ours: the "No leaks" row shows that case but does not repair it. If the system does not let IPv6 be sent into the tunnel (as when IPv6 is switched off in it), the app connects all the same; on Windows it tries a few more times in about the first 40 seconds after a connect. Not tried on a real network with IPv6.
- **"No leaks":** the row shows the state at the moment of the check. What changed later it sees at the next check.
- **Windows:** names can still go past the VPN if the system resolver is chosen in the tunnel settings or DNS over HTTPS is switched on in Windows itself. Names of devices on the home network (LLMNR, mDNS, NetBIOS) are not covered.
- **macOS and Linux:** the "No leaks" row does not check DNS, so it is never green there.
- **Pause:** not checked on a real sleep and wake of a computer or on a real change of Wi-Fi — only on their likeness in virtual machines: a change of network in this release too (Debian), sleep while 0.3.5 was prepared. If the router's address cannot be read for a moment, the app takes that for a change of network: the pause ends early and protection comes back.
- **Repair of the connection and a pinned country:** a repair can move protection to another country even when a country is pinned by hand; the pin stays, and the next connect returns to it.
- **The shield and the buttons under it:** for the first 0.6 seconds after a button has changed or moved (for example "Turn on" became "Cancel", the paused screen appeared, the country list or a panel over the screen closed), presses that turn protection on or off do nothing, so that a stray second click, or a press aimed at something else, cancels nothing. If a press did not work, press again.
- **Steam Deck:** the build has not been run on a real Steam Deck. The simulation (the userspace of SteamOS 3.7, 3.8 and 3.9 under gamescope) was run on the AppImage of 0.3.4; it was not repeated for 0.3.5 or 0.3.6. Not verified is what exists only on the device itself: Steam Input layouts, Steam's on-screen keyboard, connecting without a password in the real Gaming Mode, touch, and how it looks on its screen. What was checked and what was not is in docs/STEAMDECK.md.
- **Windows, upgrading from 0.3.2 or older:** their uninstaller cannot close the app. If the installer says it cannot close ProxysVPN, quit the app from the tray and press "Retry".
- **Windows, the tray menu:** if the menu is open at the moment a pause ends by its timer, it closes by itself, and a small white square stays where its corner was until the next click. It affects nothing.
- **No code signing and no automatic updates.**
- **macOS:** Apple Silicon only. The whole app runs as root: the engines run from a copy in a root-owned folder, the app's own executable from the bundle in Applications. After a crash the engines may keep running until ProxysVPN is opened again, which stops them. A click on a button in the app's window while another app is the active one only brings the window forward: press the button again.
- **Linux:** x86_64 only. The password window of the AppImage's first connect has not been checked live: on Debian 13, where the checks ran, administrator rights are granted without asking.
- **Linux without systemd-resolved:** if the app and its helper process are both killed by force at the same moment (for example `kill -9` or the memory killer), `/etc/resolv.conf` keeps the tunnel's resolver, and sites do not open by name while protection is off. Turn protection on once: names work with it, and turning it off puts the file back as it was.

---

# ProxysVPN Desktop v0.3.5-beta

🇷🇺 Если наш сайт закрыт в вашей сети, приложение включается по сохранённому списку серверов, само его обновляет и само повторяет неудавшееся включение. Защита от утечек: IPv6 уходит в туннель, а на Windows запросы DNS не идут мимо VPN. Пауза на 5, 15 минут или час и быстрые действия в трее. Вход одной кнопкой «Открыть в ProxysVPN». Локальные порты движков закрыты.

🇬🇧 When our site is closed on your network, the app turns on from the list of servers it saved, renews it by itself and retries a failed connect by itself. Leak protection: IPv6 goes into the tunnel, and on Windows DNS queries no longer go past the VPN. A pause for 5 or 15 minutes or an hour, and quick actions in the tray. One-button sign-in with "Open in ProxysVPN". The engines' local ports are closed.

## 🇷🇺 Установка

Сверьте SHA-256 файла со строкой в `SHA256SUMS.txt`. Подписи кода пока нет, поэтому каждая система один раз предупреждает.

- **macOS 12+ (Apple Silicon):** откройте `ProxysVPN_0.3.5_aarch64.dmg`, перетащите ProxysVPN на ярлык Applications, затем один раз в Терминале `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app` (или «Всё равно открыть» в Системных настройках → Конфиденциальность и безопасность). При запуске приложение спрашивает пароль администратора.
- **Windows 10/11 (x64):** скачайте `ProxysVPN_0.3.5_x64-setup.exe`. Microsoft Edge может задержать загрузку: кнопка сохранения спрятана за стрелкой справа от «Удалить» → «Всё равно сохранить». Затем запустите файл; SmartScreen: «Подробнее» → «Выполнить в любом случае». При каждом запуске — запрос прав администратора (UAC).
- **Windows 10/11 на ARM (Snapdragon, Surface Pro X):** `ProxysVPN_0.3.5_arm64-setup.exe`, шаги те же. Файл x64 там тоже работает, через эмуляцию.
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.5_amd64.deb`. Нужен сеанс с агентом polkit. Удаление: `sudo apt remove proxys-vpn`.
- **Steam Deck (AppImage, Linux x86_64):** `ProxysVPN_0.3.5_amd64.AppImage`. Сделайте файл исполняемым (`chmod +x`), первый запуск и первое подключение — в режиме рабочего стола. Шаги, игровой режим и управление — в docs/STEAMDECK.md.

## 🇷🇺 Что нового

- **Наш сайт закрыт в вашей сети — приложение всё равно включается.** Если оно уже получало список серверов, то включается по сохранённому (он годен 30 суток) и говорит об этом одной строкой: «Список серверов сохранённый, от …. Обновим сами, как только получится.» Пока защита включена, приложение само обновляет список через туннель; срок доступа и объявления при этом не пропадают с экрана. Адреса сайта теперь приходят подписанным списком: новый адрес приложение узнаёт без нового выпуска, а отозванный перестаёт спрашивать.
- **Не получилось включиться — приложение пробует снова само:** примерно через 30 секунд, 2, 5 и 15 минут, дальше раз в 15 минут. На экране идёт отсчёт до следующей попытки и названа настоящая причина: нет интернета, наши адреса не отвечают из этой сети или сеть не выпускает наружу.
- **Чужая страница на нашем адресе не принимается за ответ сервиса.** Заглушка хостинга или чужой прокси раньше читались как «ссылка не подходит» и «нет устройств»; теперь приложение идёт к следующему адресу. Перенаправления принимаются только на наши адреса.
- **Вход, ссылки и «Проверка» не зависят от одного адреса.** Экран входа спрашивает наши адреса по очереди и запоминает ответивший. «Личный кабинет», «Политика конфиденциальности», «Условия использования» и «Удалить аккаунт» открываются на адресе, который отвечает именно вам; если защита выключена и не отвечает ни один, приложение так и скажет и предложит включить защиту. «Проверка» больше не винит DNS, когда дело в адресе, и различает три случая: ответил основной адрес, ответил запасной, не ответил ни один и приложение работает на сохранённом списке. Почта поддержки видна в самом приложении.
- **Дробление начала соединения: «Само», «Всегда», «Никогда».** В «Ещё → Туннель» вместо выключателя теперь выбор из трёх и сила «Мягко» / «Сильнее». «Само» (по умолчанию) включает дробление, только когда обычное подключение не проходит, и запоминает сеть, где это помогло. Кто включал старый выключатель, остаётся на «Всегда». На локациях Hysteria2 дробление не действует.
- **Срок доступа и объявления.** Приложение само говорит, когда кончается доступ: за три дня и за день, одной спокойной строкой и, если разрешены уведомления, одним уведомлением. Объявление об аварии показывается одной строкой; крестик его прячет.
- **IPv6 больше не идёт мимо VPN.** Пока защита включена, IPv6 компьютера уходит в туннель. У наших серверов выхода по IPv6 нет, поэтому сайты по HTTPS и HTTP приложение ведёт через сервер по их имени, а остальные соединения по IPv6 сразу отклоняет — программа обычно сама переходит на IPv4, который идёт через VPN. Раньше в сети с IPv6 такие соединения шли напрямую, с вашим настоящим адресом.
- **Windows: запросы DNS не идут мимо VPN.** Пока защита включена, имена сайтов спрашиваются через туннель, а обычные запросы DNS через другие сетевые адаптеры Windows не пропускает. Раньше имена разрешал DNS вашей сети, и она видела, какие сайты вы открываете. Если в настройках туннеля выбран системный резолвер, приложение Windows не трогает — это ваш выбор. Запрет действует, только пока работает приложение: если оно завершится аварийно, Windows снимает его сама, и компьютер не остаётся без DNS.
- **Строка «Утечек нет» на экране «Проверка».** Приложение спрашивает у самой системы, куда пойдёт IPv6 и — на Windows — проходит ли запрос DNS мимо туннеля, и показывает ответ. На macOS и Linux DNS пока не проверяется, и строка говорит об этом прямо.
- **Пауза.** Под щитом появилась «Пауза»: на 5 минут, 15 минут или 1 час. На это время защита выключается полностью — так же, как по кнопке «Выключить», — и приложение показывает, до какого времени. Пока пауза, трафик идёт без VPN. Потом защита включается сама, на ту же страну и тот же протокол. Вернуть её раньше можно кнопкой «Возобновить сейчас» или нажатием на щит. Пауза заканчивается раньше срока и сама, если компьютер проснулся после сна, сменилась сеть или перевели часы. Если после паузы защита не вернулась или VPN включился, но данные через него не идут, приложение говорит об этом системным уведомлением (его выключает настройка «Уведомления»).
- **Меню в трее.** Показывает состояние защиты и даёт быстрые действия: включить, выключить, пауза на 5 минут, 15 минут или 1 час, возобновить, открыть окно, выйти.
- **Двойной щелчок больше не отменяет включение.** Раньше второй щелчок попадал на «Отмену», которая появляется на месте «Включить», и защита оставалась выключенной. Теперь два быстрых нажатия в одно место — одно нажатие. А свёрнутое окно возвращается строкой «Открыть окно» в меню трея.
- **Нажатие мимо панели «Восстанавливаю соединение» больше не выключает защиту.** Эта панель сама поднимается поверх экрана, когда починка соединения затягивается. Раньше нажатие рядом с ней, клавиша Esc или смахивание вниз выключали защиту — так же, как её кнопка. Теперь они только убирают панель, а починка продолжается; защиту выключает одна кнопка — «Отменить и выключить защиту».
- **«Открыть в ProxysVPN».** В кабинете и в боте под кодом привязки появится кнопка, которая открывает приложение. Приложение спрашивает «Войти по коду?» и входит только после вашего нажатия: сама ссылка ничего сделать не может. Работает на macOS, на Windows и в сборке .deb для Linux.
- **Безопасность: локальные порты движков закрыты.** В 0.3.4 и раньше движки слушали на этом компьютере порты 10808 и 10809 без пароля: любая программа любого пользователя компьютера могла пустить свой трафик через ваш туннель. Теперь порты выбираются заново на каждый сеанс и открываются только по секрету этого сеанса.
- **Файл с паролем сервера не остаётся на диске.** На локациях Hysteria2 движок получает настройки файлом; теперь он удаляется, когда движок останавливается, и при следующем запуске после сбоя.
- **Windows: запущенное приложение больше не мешает выключить компьютер.** Раньше при завершении работы Windows показывала окно с ошибкой netsh.exe, потом экран о том, что приложение не даёт завершить работу, и примерно через минуту отменяла выключение. Теперь при завершении работы, перезагрузке и выходе из системы приложение ничего не запускает и закрывается сразу. Судя по коду, так же вели себя версии 0.3.2–0.3.4.

## 🇷🇺 Известные ограничения

- **Закрытые адреса сайта:** эта часть проверена тестами и в виртуальной машине, где наши адреса закрывались искусственно; в настоящей заблокированной сети этот выпуск не проверялся. Если приложение ещё ни разу не получало список серверов, а все наши адреса в вашей сети закрыты, включаться ему не по чему: войдите и один раз включите защиту в сети, где сайт открывается.
- **IPv6:** у серверов нет выхода по IPv6, сайты открываются по IPv4. Более точный маршрут IPv6, который раздаёт сам роутер, сильнее нашего: строка «Утечек нет» такой случай показывает, но не исправляет. Если система не дала направить IPv6 в туннель (так бывает, когда IPv6 в ней выключен), приложение всё равно подключается; на Windows оно пробует ещё несколько раз примерно в первые 40 секунд после подключения. Проверено в сетях без IPv6 и на его подобии в виртуальной машине, где читались только маршруты; в настоящей сети с IPv6 этот выпуск не проверялся.
- **«Утечек нет»:** строка показывает состояние на момент проверки. То, что изменилось позже, она увидит при следующей проверке.
- **Windows:** мимо VPN имена всё ещё могут идти, если в настройках туннеля выбран системный резолвер или в самой Windows включён DNS поверх HTTPS. Имена устройств домашней сети (LLMNR, mDNS, NetBIOS) не закрываются.
- **macOS и Linux:** DNS в строке «Утечек нет» не проверяется, поэтому зелёной она там не бывает.
- **Пауза:** не проверена на настоящем сне и пробуждении компьютера и на настоящей смене Wi-Fi — только на их подобии в виртуальных машинах. Если на миг не читается адрес роутера, приложение принимает это за смену сети: пауза кончится раньше, защита вернётся.
- **Починка соединения:** после неё строка «Подтвердить не удалось» может оставаться на экране, хотя VPN уже работает, — до следующей проверки: до 5 минут, когда окно открыто, и до 15, когда оно скрыто. Починка может перевести защиту на другую страну, даже если страна закреплена вручную; закрепление остаётся, и следующее включение вернётся на неё.
- **Кнопка «Включить»:** если нажать её, когда активным было другое окно, экран может ещё 2–20 секунд показывать «Защита выключена», хотя защита уже включается. Второй раз нажимать не нужно. Так было и в прежних версиях.
- **Щит и кнопки под ним:** первые 0,6 секунды после того, как кнопка сменилась или сдвинулась (например, «Включить» стала «Отменой», появился экран паузы, закрылся список стран или панель поверх экрана), нажатия, которые включают или выключают защиту, не действуют — так случайный второй щелчок и нажатие, которое метило в другое место, ничего не отменяют. Если нажатие не сработало, нажмите ещё раз.
- **Steam Deck:** сборка не запускалась на настоящей Steam Deck. В симуляции (окружение SteamOS 3.7, 3.8 и 3.9 под gamescope) проверялся AppImage версии 0.3.4; для 0.3.5 симуляцию не повторяли, её AppImage запускали на Debian 13. Не проверено то, что есть только на самой приставке: раскладки Steam Input, экранная клавиатура Steam, подключение без пароля в настоящем игровом режиме, касание и вид на её экране. Что именно проверено и что нет — в docs/STEAMDECK.md.
- **Windows:** выпуск проверен в виртуальной машине Windows 11 на ARM — и нативная сборка, и сборка x64 через эмуляцию (обе подключались примерно за 10 секунд). На настоящем ARM-компьютере (Snapdragon, Surface Pro X) и на настоящем компьютере x64 этот выпуск ещё не запускался.
- **Windows, обновление с 0.3.2 и старше:** их деинсталлятор не умеет закрывать приложение. Если установщик скажет, что не может закрыть ProxysVPN, выйдите из приложения через трей и нажмите «Повторить».
- **Windows, меню в трее:** если меню открыто в тот момент, когда пауза заканчивается по времени, оно закрывается само, а на месте его угла до следующего щелчка остаётся белый квадратик. Ни на что не влияет.
- **Нет подписи кода и автообновления.**
- **macOS:** только Apple Silicon. Всё приложение работает от root: движки запускаются из копии в папке root, а сам исполняемый файл — из бандла в «Программах». После сбоя движки могут работать до следующего запуска ProxysVPN, и он их остановит.
- **Linux:** только x86_64. Сборки .deb и AppImage проверены в Debian 13, где права администратора выдаются без запроса: окно пароля при первом подключении AppImage в этом выпуске вживую не проверено.
- **Linux без systemd-resolved:** если приложение и его служебный процесс убиты принудительно в один момент (например, `kill -9` или нехватка памяти), в `/etc/resolv.conf` остаётся резолвер туннеля, и сайты не открываются по имени, пока защита выключена. Включите защиту один раз: с ней имена работают, а при выключении файл возвращается к прежнему виду.

## 🇬🇧 Install

Compare each file's SHA-256 with `SHA256SUMS.txt`. Nothing is code-signed yet, so each system warns once.

- **macOS 12+ (Apple Silicon):** open `ProxysVPN_0.3.5_aarch64.dmg`, drag ProxysVPN onto the Applications shortcut, then once in Terminal `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app` (or "Open Anyway" in System Settings → Privacy & Security). The app asks for the administrator password at launch.
- **Windows 10/11 (x64):** download `ProxysVPN_0.3.5_x64-setup.exe`. Microsoft Edge may hold the download: the way to keep the file is hidden behind the arrow to the right of "Delete" → "Keep anyway". Then run the file; SmartScreen: "More info" → "Run anyway". Every launch asks for administrator rights (UAC).
- **Windows 10/11 on ARM (Snapdragon, Surface Pro X):** `ProxysVPN_0.3.5_arm64-setup.exe`, the same steps. The x64 file works there too, under emulation.
- **Linux (Debian 12+, Ubuntu 22.04+, x86_64):** `sudo apt install ./ProxysVPN_0.3.5_amd64.deb`. Needs a session with a polkit agent. Remove with `sudo apt remove proxys-vpn`.
- **Steam Deck (AppImage, x86_64 Linux):** `ProxysVPN_0.3.5_amd64.AppImage`. Make the file executable (`chmod +x`); the first start and the first connect happen in Desktop Mode. The steps, Gaming Mode and the controller are in docs/STEAMDECK.md.

## 🇬🇧 What's new

- **Our site is closed on your network — the app still turns on.** If it has fetched the list of servers before, it turns on from the saved one (good for 30 days) and says so in one line: "The server list is a saved one, from …. We'll update it ourselves as soon as we can." While protection is on, the app renews the list by itself through the tunnel; your access term and the notices stay on the screen meanwhile. The site's addresses now arrive in a signed list: the app learns a new address without a new release and stops asking one that was withdrawn.
- **A connect that failed is tried again by the app itself:** after about 30 seconds, 2, 5 and 15 minutes, then every 15 minutes. The screen counts down to the next try and names the real cause: no internet, our addresses do not answer from this network, or the network lets nothing out.
- **Someone else's page on our address is not taken for the service's answer.** A hosting placeholder or a foreign proxy used to read as "this link does not fit" and "no devices"; the app now goes on to the next address. Redirects are followed only to our own addresses.
- **Signing in, the links and "Check" no longer depend on one address.** The sign-in screen asks our addresses in turn and remembers the one that answered. "Your account", "Privacy Policy", "Terms of Use" and "Delete account" open at the address that answers for you; if protection is off and none answers, the app says so and offers to turn protection on. "Check" no longer blames the DNS when the address is the matter, and tells three cases apart: the main address answered, a backup answered, none answered and the app works on the saved list. The support email is shown in the app itself.
- **Splitting the start of a connection: "Auto", "Always", "Never".** "More → Tunnel" now has a choice of three in place of the switch, and a strength, "Gentle" / "Stronger". "Auto" (the default) turns splitting on only when an ordinary connect does not get through, and remembers the network where it helped. Whoever had the old switch on stays on "Always". Splitting does not apply to Hysteria2 locations.
- **Access term and notices.** The app tells you when your access is ending: three days and one day before, as one calm line and, if notifications are allowed, one notification. An announcement of an outage shows as one line; the cross hides it.
- **IPv6 no longer goes past the VPN.** While protection is on, the computer's IPv6 goes into the tunnel. Our servers have no IPv6 exit, so the app carries HTTPS and HTTP sites through the server by their name and refuses other IPv6 connections at once — a program then usually moves to IPv4 by itself, and that goes through the VPN. Before, on a network with IPv6 such connections left directly, with your real address.
- **Windows: DNS queries no longer go past the VPN.** While protection is on, site names are asked through the tunnel, and Windows lets no ordinary DNS query out through the other network adapters. Before, your network's DNS resolved the names and saw which sites you opened. If the system resolver is chosen in the tunnel settings, the app leaves Windows alone — that is your choice. The block lasts only while the app runs: if the app dies, Windows lifts it by itself, and the computer is not left without DNS.
- **A "No leaks" row on the Check screen.** The app asks the system itself where IPv6 would go and — on Windows — whether a DNS query gets past the tunnel, and shows the answer. On macOS and Linux DNS is not checked yet, and the row says so.
- **Pause.** There is a "Pause" under the shield: for 5 minutes, 15 minutes or 1 hour. For that time protection is fully off — the same as with "Turn off" — and the app shows until when. During a pause traffic goes without the VPN. Then protection comes back by itself, on the same country and protocol. "Resume now" or a press on the shield brings it back earlier. A pause also ends early by itself when the computer wakes from sleep, the network changes or the clock is set. If protection does not come back after a pause, or the VPN is on but no data moves through it, the app says so with a system notification (the "Notifications" setting turns it off).
- **A tray menu.** It shows the state of protection and gives quick actions: turn on, turn off, pause for 5 minutes, 15 minutes or 1 hour, resume, open the window, quit.
- **A double click no longer cancels the connect.** The second click used to land on "Cancel", which appears where "Turn on" was, and protection stayed off. Two fast presses on one spot are now one press. And a minimized window comes back with "Open window" in the tray menu.
- **A press beside the "Restoring the connection" panel no longer turns protection off.** The panel rises by itself over the screen when a repair of the connection drags on. A press beside it, Esc or a swipe down used to turn protection off, the same as its button. They now only put the panel away, and the repair goes on; protection is turned off by its one button, "Cancel and turn protection off".
- **"Open in ProxysVPN".** The cabinet and the bot get a button under the pair code that opens the app. The app asks "Sign in with this code?" and signs in only after you press the button: the link can do nothing by itself. Works on macOS, on Windows and in the .deb build for Linux.
- **Security: the engines' local ports are closed.** In 0.3.4 and earlier the engines listened on ports 10808 and 10809 of this computer with no password: any program of any user of the computer could send its traffic through your tunnel. The ports are now picked anew for every session and open only with that session's secret.
- **The file with the server's password does not stay on disk.** On Hysteria2 locations the engine gets its settings as a file; it is now removed when the engine stops, and at the next launch after a crash.
- **Windows: the running app no longer keeps the computer from shutting down.** A shutdown used to bring up an error box for netsh.exe, then the screen saying that an app prevents shutting down, and about a minute later Windows called the shutdown off. Now, when Windows shuts down, restarts or signs you out, the app starts nothing and closes at once. By the code, 0.3.2–0.3.4 behaved the same way.

## 🇬🇧 Known limitations

- **Closed site addresses:** this part is covered by tests and was tried in a virtual machine where our addresses were closed on purpose; this release has not been tried on a real blocked network. If the app has never fetched the list of servers and all our addresses are closed on your network, it has nothing to turn on from: sign in and turn protection on once on a network where the site opens.
- **IPv6:** the servers have no IPv6 exit, sites open over IPv4. A more specific IPv6 route handed out by the router itself wins over ours: the "No leaks" row shows that case but does not repair it. If the system does not let IPv6 be sent into the tunnel (as when IPv6 is switched off in it), the app connects all the same; on Windows it tries a few more times in about the first 40 seconds after a connect. Checked on networks without IPv6 and on a likeness of one in a virtual machine, where only the routes were read; this release has not been tried on a real network with IPv6.
- **"No leaks":** the row shows the state at the moment of the check. What changed later it sees at the next check.
- **Windows:** names can still go past the VPN if the system resolver is chosen in the tunnel settings or DNS over HTTPS is switched on in Windows itself. Names of devices on the home network (LLMNR, mDNS, NetBIOS) are not covered.
- **macOS and Linux:** the "No leaks" row does not check DNS, so it is never green there.
- **Pause:** not checked on a real sleep and wake of a computer or on a real change of Wi-Fi — only on their likeness in virtual machines. If the router's address cannot be read for a moment, the app takes that for a change of network: the pause ends early and protection comes back.
- **Repair of the connection:** after it, the line "Could not confirm" can stay on the screen although the VPN already works — until the next check: up to 5 minutes while the window is open and up to 15 while it is hidden. A repair can move protection to another country even when a country is pinned by hand; the pin stays, and the next connect returns to it.
- **The "Turn on" button:** if you press it while another window was the active one, the screen can go on showing "Protection is off" for another 2–20 seconds although protection is already turning on. No second press is needed. Earlier versions did the same.
- **The shield and the buttons under it:** for the first 0.6 seconds after a button has changed or moved (for example "Turn on" became "Cancel", the paused screen appeared, the country list or a panel over the screen closed), presses that turn protection on or off do nothing, so that a stray second click, or a press aimed at something else, cancels nothing. If a press did not work, press again.
- **Steam Deck:** the build has not been run on a real Steam Deck. The simulation (the userspace of SteamOS 3.7, 3.8 and 3.9 under gamescope) was run on the AppImage of 0.3.4; it was not repeated for 0.3.5, whose AppImage was run on Debian 13. Not verified is what exists only on the device itself: Steam Input layouts, Steam's on-screen keyboard, connecting without a password in the real Gaming Mode, touch, and how it looks on its screen. What was checked and what was not is in docs/STEAMDECK.md.
- **Windows:** the release was checked in a Windows 11 on ARM virtual machine — both the native build and the x64 build under emulation (each connected in about 10 seconds). It has not been run on a real ARM PC (Snapdragon, Surface Pro X) or on a real x64 PC yet.
- **Windows, upgrading from 0.3.2 or older:** their uninstaller cannot close the app. If the installer says it cannot close ProxysVPN, quit the app from the tray and press "Retry".
- **Windows, the tray menu:** if the menu is open at the moment a pause ends by its timer, it closes by itself, and a small white square stays where its corner was until the next click. It affects nothing.
- **No code signing and no automatic updates.**
- **macOS:** Apple Silicon only. The whole app runs as root: the engines run from a copy in a root-owned folder, the app's own executable from the bundle in Applications. After a crash the engines may keep running until ProxysVPN is opened again, which stops them.
- **Linux:** x86_64 only. The .deb and the AppImage were checked on Debian 13 where administrator rights are granted without asking: the password window of the AppImage's first connect was not checked live in this release.
- **Linux without systemd-resolved:** if the app and its helper process are both killed by force at the same moment (for example `kill -9` or the memory killer), `/etc/resolv.conf` keeps the tunnel's resolver, and sites do not open by name while protection is off. Turn protection on once: names work with it, and turning it off puts the file back as it was.

---

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
