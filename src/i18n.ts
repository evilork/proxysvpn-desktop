// src/i18n.ts
//
// EVERY sentence a human reads lives here. Nothing else in the app — and
// nothing at all in Rust — is allowed to hold a Russian string.
//
// Why so strict. The core answers with CODES (src-tauri/src/errors.rs); the
// window owns the wording. That is what makes ten languages, a wording fix
// without rebuilding the core, and a test that asserts on a code rather than
// on a phrase all possible at the same time.
//
// Shape of the file:
//   `ru` is the source of truth and is ALWAYS complete. `MsgKey` is derived
//   from it, so a key that does not exist fails `tsc` instead of rendering as
//   a raw key on screen.
//   Other languages are `Partial` — a missing line falls back to Russian, and
//   never to the key itself (an untranslated sentence is bad, a visible
//   "more.unlink" is a broken app).
//
// Adding a language: write a `Partial<Record<MsgKey, string>>`, add it to
// `DICTS`, `LANGS`, `LANG_NAME` and `INTL_LOCALE`. Nothing else changes.
//
// Plurals: a value may carry forms separated by "|" — one / few / many for
// Russian, one / other for English. The form is chosen by `params.n`.
// Substitution: "{name}" is replaced by `params.name`.

export type Lang = "ru" | "en";

/** Order matters: this is the order of the language list in [12]. */
export const LANGS: readonly Lang[] = ["ru", "en"] as const;

/** BCP-47 tags for Intl. Kept next to `LANGS` so a new language adds one line. */
const INTL_LOCALE: Record<Lang, string> = {
  ru: "ru-RU",
  en: "en-GB",
};

/** Name of the language in that language — never translated. */
export const LANG_NAME: Record<Lang, string> = {
  ru: "Русский",
  en: "English",
};

const ru = {
  // ── Shared ────────────────────────────────────────────────────────────────
  "app.name": "ProxysVPN",
  "common.close": "Закрыть",
  "common.back": "Назад",
  "common.retry": "Повторить",
  "common.cancel": "Отмена",
  "common.ok": "Понятно",
  "common.copy": "Скопировать",
  "common.copied": "Скопировано",
  "common.loading": "Загружаем…",

  // ── Time, said out loud ───────────────────────────────────────────────────
  "time.justNow": "только что",
  "time.secondsAgo": "{n} секунду назад|{n} секунды назад|{n} секунд назад",
  "time.minutesAgo": "{n} минуту назад|{n} минуты назад|{n} минут назад",
  "time.hoursAgo": "{n} час назад|{n} часа назад|{n} часов назад",
  "time.daysAgo": "{n} день назад|{n} дня назад|{n} дней назад",
  "time.days": "{n} день|{n} дня|{n} дней",
  "unit.kb": "{n} КБ",
  "unit.mb": "{n} МБ",

  // ── [2] Main screen — the shield ──────────────────────────────────────────
  "main.off.title": "Защита выключена",
  "main.off.hint": "Нажмите, чтобы включить",
  "main.off.action": "Включить",
  "main.starting.title": "Включаем…",
  "main.starting.action": "Отмена",
  // The steps are emitted by the core as they actually happen, so the wait is
  // honest: the old screen showed a spinner and invented reassurance.
  "main.starting.fetchingSub": "Проверяю подписку",
  "main.starting.pickingServer": "Подбираю сервер",
  "main.starting.startingEngine": "Готовлю соединение",
  "main.starting.raisingTun": "Настраиваю сеть",
  "main.starting.probing": "Проверяю, что данные идут",
  "main.on.title": "Защищено",
  "main.on.action": "Выключить",
  "main.unconfirmed.title": "Подтвердить не удалось",
  "main.unconfirmed.hint": "Соединение поднято, проверочная страница не ответила",
  "main.healing.quiet": "Обновляю соединение…",
  "main.healing.title": "Восстанавливаю соединение",
  "main.healing.hint": "Интернет вернётся сам",
  "main.failed.action": "Что делать",
  "main.nolink.title": "Не настроено",
  "main.nolink.hint": "Добавьте ссылку — это одно действие",
  "main.nolink.action": "Добавить ссылку",
  "main.shield.off": "Защита выключена. Нажмите, чтобы включить",
  "main.shield.starting": "Включаем защиту",
  "main.shield.on": "Защищено. Нажмите, чтобы выключить",
  "main.shield.unconfirmed": "Защита включена, подтвердить не удалось",
  "main.shield.healing": "Восстанавливаем соединение",
  "main.shield.failed": "Защита не работает",
  "main.truth.details": "Подробности о соединении",
  "main.truth.rtt": "{n} мс",
  "main.truth.checked": "проверено {age}",
  "main.truth.checking": "проверяем",
  "main.nav.country": "Страна",
  "main.nav.check": "Проверка",
  "main.nav.more": "Ещё",

  // ── Service bar: outage, expiry, money. Silent when there is nothing ──────
  "bar.expiry": "Осталось {n} день|Осталось {n} дня|Осталось {n} дней",
  "bar.expirySoon":
    "Осталось {n} день. Пополните, чтобы не прерывалось|Осталось {n} дня. Пополните, чтобы не прерывалось|Осталось {n} дней. Пополните, чтобы не прерывалось",
  "bar.expired": "Срок доступа закончился",

  // ── [0] First run ─────────────────────────────────────────────────────────
  "ob.step": "Шаг {n} из {total}",
  "ob.move.title": "Перенесите ProxysVPN в «Программы»",
  "ob.move.body":
    "Из папки «Загрузки» система не разрешит создать защищённое подключение. Мы перенесём приложение сами — оно закроется и сразу откроется заново.",
  "ob.move.action": "Перенести и перезапустить",
  "ob.move.skip": "Перенесу сам",
  "ob.move.failed":
    "Не удалось перенести. Откройте «Загрузки» и перетащите ProxysVPN в «Программы»",
  "ob.pass.title": "Сейчас система спросит пароль",
  "ob.pass.body":
    "Он нужен один раз за запуск — чтобы направить интернет через защищённое соединение. Мы не видим и не сохраняем пароль: его спрашивает сама macOS.",
  "ob.pass.action": "Понятно, продолжить",
  "ob.ios.title": "Разрешите настройку VPN",
  "ob.ios.body":
    "iPhone спросит один раз и попросит Face ID или код. Больше этот вопрос не повторится.",
  "ob.ios.action": "Разрешить",
  "ob.waiting": "Ждём ответа системы…",
  "ob.denied": "Разрешение не получено. Без него включить не получится",
  "ob.showWhere": "Показать, где нажать",
  "ob.help": "Нужна помощь",

  // ── [1] Adding the subscription ───────────────────────────────────────────
  "pair.title": "Осталось добавить вашу ссылку",
  "pair.body":
    "Откройте личный кабинет на телефоне, нажмите значок кода рядом со ссылкой и наведите камеру сюда.",
  "pair.preparing": "Готовим код…",
  "pair.countdown": "Код действует {time}",
  "pair.waiting": "Ждём телефон…",
  "pair.paste": "Вставить ссылку из буфера",
  "pair.codeFailed": "Не удалось получить код. Проверьте интернет",
  "pair.expired": "Код устарел",
  "pair.refresh": "Обновить код",
  "pair.notOurLink": "Это не ссылка ProxysVPN. Возьмите её в боте: «📡 Мои устройства»",
  "pair.clipboardEmpty": "В буфере обмена нет ссылки",
  "pair.clipboardFailed": "Не удалось прочитать буфер обмена",
  "pair.done": "Готово. Ссылка добавлена",
  "pair.keychain": "Ссылка хранится в защищённом хранилище системы",
  "pair.scan": "Навести камеру",

  // ── [3] Details sheet ─────────────────────────────────────────────────────
  "details.title": "Подробности",
  "details.change": "Сменить",
  "details.edit": "Изменить",
  "details.locationAuto": "Выберем сами при включении",
  "details.whereRu": "Вы в России — российские сайты идут напрямую",
  "details.whereAbroad": "Вы за границей — весь интернет идёт через нас",
  "details.paidUntil": "Оплачено до {date} · осталось {days}",
  "details.expiryUnknown": "Срок неизвестен — не удалось получить данные",
  "details.split": "Куда идёт трафик: через VPN {via} · напрямую {direct}",
  "details.splitNote":
    "Мы не записываем, какие сайты вы открываете. Только сколько соединений куда ушло.",
  "details.checkNow": "Проверить сейчас",
  "details.checkOk": "Всё работает. Данные идут. Текущий сервер — {location}",
  "details.sub": "Подписка и это устройство",

  // ── [4] Recovery sheet ────────────────────────────────────────────────────
  "heal.title": "Восстанавливаю соединение",
  "heal.body": "Интернет вернётся сам. Обычно это занимает несколько секунд.",
  "heal.step.checkNode": "Проверил сервер",
  "heal.step.restartEngine": "Перезапустил соединение",
  "heal.step.otherNode": "Попробовал другой сервер",
  "heal.step.otherProto": "Пробую другой способ связи",
  "heal.step.refreshSub": "Обновляю настройки подписки",
  "heal.step.askService": "Спрашиваю сервис",
  "heal.cancel": "Отменить и выключить защиту",
  "heal.ok": "Соединение восстановлено",
  "heal.fail": "Пока не получается. Покажу, в чём дело",

  // ── [5] Countries ─────────────────────────────────────────────────────────
  "loc.title": "Страны",
  "loc.auto": "Автоматически",
  "loc.autoHint": "Выбираем самый быстрый из доступных",
  "loc.current": "Сейчас: {location}",
  "loc.recent": "Недавние",
  "loc.all": "Все страны",
  "loc.unnamed": "Сервер №{n}",
  "loc.q.good": "быстро",
  "loc.q.ok": "средне",
  "loc.q.poor": "медленно",
  "loc.q.blocked": "сейчас не проходит в вашей сети",
  "loc.q.unknown": "проверим при подключении",
  "loc.pinNote":
    "Выбранная страна закрепится на сутки. Вернуть автоматический выбор можно здесь же.",
  "loc.refreshing": "Обновляем список…",
  "loc.stale": "Список от {time}, обновить не удалось",
  "loc.refresh": "Обновить",
  "loc.switching": "Переключаем…",
  "loc.switched": "Готово. Новый сервер — {location}",
  "loc.pinned": "Закреплено на сутки",

  // ── [6] Where are you ─────────────────────────────────────────────────────
  "where.title": "Где вы сейчас?",
  "where.ru": "Я в России",
  "where.ruBody":
    "Российские сайты, банки и Госуслуги открываются напрямую — так быстрее и ничего не ломается.",
  "where.abroad": "Я за границей",
  "where.abroadBody":
    "Весь интернет идёт через нас, включая российские сайты. Так работают RuTube, банки и телефония.",
  "where.guessRu": "Определили сами: похоже, вы в России.",
  "where.guessAbroad": "Определили сами: похоже, вы за границей.",
  "where.failed": "Не удалось применить, оставили как было",
  "where.short.ru": "Я в России",
  "where.short.abroad": "Я за границей",

  // ── [7] What happened — one reason, one action ────────────────────────────
  "problem.title": "Что случилось",
  "problem.check": "Проверка",
  "problem.timeline": "Что происходило",
  "problem.working": "Работаем…",

  "action.addLink": "Добавить ссылку",
  "action.retry": "Повторить",
  "action.openCabinet": "Открыть кабинет",
  "action.topUp": "Пополнить",
  "action.diagnose": "Проверить",
  "action.waitAndSee": "Понятно",
  "action.contactSupport": "Написать в поддержку",

  // One title and one paragraph for every code in ERROR_ACTION.
  // The server's own text, when it sent one, is printed under the paragraph —
  // it is already proof-read by support and already translated by `?lang=`.
  "err.NO_SUBSCRIPTION.title": "Ссылка не добавлена",
  "err.NO_SUBSCRIPTION.body": "Без неё приложению нечего включать. Это одно действие.",
  "err.SUB_MALFORMED.title": "Эта ссылка не подходит",
  "err.SUB_MALFORMED.body":
    "Похоже, скопировалась не та строка. Возьмите ссылку в боте: «📡 Мои устройства».",
  "err.SUB_UNREACHABLE.title": "Не удалось получить настройки",
  "err.SUB_UNREACHABLE.body":
    "Мы пробовали все запасные адреса. Обычно дело в сети, из которой вы выходите: попробуйте ещё раз или переключитесь с Wi-Fi на мобильный интернет.",
  "err.SUB_INVALID.title": "Настройки пришли непонятными",
  "err.SUB_INVALID.body":
    "Сервер ответил, но прочитать ответ не получилось. Такое бывает, когда в сети стоит промежуточный сервер провайдера.",
  "err.SUB_EMPTY.title": "В подписке нет ни одной локации",
  "err.SUB_EMPTY.body":
    "Сервис ответил пустым списком. Посмотрите состояние подписки в кабинете — обычно ответ там.",
  "err.BALANCE_EMPTY.title": "Закончились средства",
  "err.BALANCE_EMPTY.body": "Доступ включится сразу после пополнения.",
  "err.EXPIRED.title": "Срок доступа закончился",
  "err.EXPIRED.body": "Продлите доступ, и защита включится тем же нажатием.",
  "err.DEVICE_TAKEN.title": "Эта ссылка занята другим устройством",
  "err.DEVICE_TAKEN.body":
    "Одна ссылка работает на одном устройстве. Чтобы перенести подписку сюда, сбросьте привязку в кабинете или в боте.",
  "err.NO_DEVICES.title": "К аккаунту не привязано ни одного устройства",
  "err.NO_DEVICES.body": "Добавьте устройство в кабинете — ссылка заработает сразу.",
  "err.SUB_NOTICE.title": "Сообщение от сервиса",
  "err.SUB_NOTICE.body": "Сервис ответил вместо списка локаций. Вот что он пишет:",
  "err.PERMISSION_DENIED.title": "Нужно разрешение системы",
  "err.PERMISSION_DENIED.body":
    "Без него приложение не может направить интернет через защищённое соединение. Нажмите «Повторить» и разрешите в системном окне.",
  "err.ENGINE_START_FAILED.title": "Соединение не запустилось",
  "err.ENGINE_START_FAILED.body":
    "Внутренняя часть приложения не поднялась. Обычно помогает повторное нажатие.",
  "err.ENGINE_DIED.title": "Соединение оборвалось",
  "err.ENGINE_DIED.body": "Мы уже поднимаем его заново — делать ничего не нужно.",
  "err.PORT_BUSY.title": "Предыдущий запуск ещё не закрылся",
  "err.PORT_BUSY.body":
    "Старое соединение держит приложение занятым. Нажмите «Повторить» — мы закроем его сами.",
  "err.TUN_FAILED.title": "Не удалось настроить сеть",
  "err.TUN_FAILED.body":
    "Системное сетевое подключение не поднялось. Чаще всего мешает другой VPN, работающий одновременно с нами.",
  "err.NETWORK_OFFLINE.title": "Нет интернета",
  "err.NETWORK_OFFLINE.body":
    "Устройство сейчас не в сети. Включим защиту сами, как только интернет вернётся.",
  "err.NO_ROUTE.title": "Сервер недоступен из вашей сети",
  "err.NO_ROUTE.body":
    "До выбранного сервера не доходит соединение — так выглядит блокировка адреса у провайдера. Мы уже пробуем другие.",
  "err.BLOCKED.title": "Подключение есть, но сайты не открываются",
  "err.BLOCKED.body":
    "Соединение установлено, а данные через него не идут. Обычно это фильтрация у провайдера: поможет другая страна или другой способ связи.",
  "err.PROBE_UNCONFIRMED.title": "Подтвердить не удалось",
  "err.PROBE_UNCONFIRMED.body":
    "Соединение поднято и данные идут, но наша проверочная страница не ответила. Возможно, всё работает — мы просто не можем это доказать.",
  // Not a failure screen: Hysteria2 gives no latency figure and never will.
  // The action for this code is "offer nothing", so the wording must not
  // sound like something the person has to fix.
  "err.PING_NOT_APPLICABLE.title": "Отклик не измеряется",
  "err.PING_NOT_APPLICABLE.body":
    "На этом способе связи время отклика измерить нельзя. Это не поломка: защита работает, просто числа для неё не будет.",
  "err.UNKNOWN.title": "Что-то пошло не так",
  "err.UNKNOWN.body":
    "Мы не смогли назвать причину. Пришлите отчёт — в нём есть всё, что нужно, чтобы разобраться.",

  // ── [8] Check ─────────────────────────────────────────────────────────────
  "check.title": "Проверка",
  "check.device": "На этом устройстве",
  "check.service": "На стороне сервиса",
  "check.run": "Проверить",
  "check.running": "Проверяем…",
  "check.again": "Проверить ещё раз",
  "check.notChecked": "не проверяли",
  "check.row.clock": "Часы на устройстве",
  "check.row.internet": "Интернет есть",
  "check.row.dns": "Имена сайтов разрешаются",
  "check.row.sub": "Наш адрес отвечает",
  "check.row.handshake": "Соединение поднимается",
  "check.row.traffic": "Данные идут",
  "check.row.network": "Сеть пропускает VPN",
  "check.row.balance": "Баланс и срок",
  "check.row.profile": "Профиль на серверах",
  "check.row.freshness": "Свежесть подписки",
  "check.row.nodes": "Состояние локаций",
  "check.note.fallback": "запасной",
  "check.serviceInCabinet": "Проверить с нашей стороны можно в кабинете",
  "check.serviceFailed": "Проверить с нашей стороны не удалось",
  "check.openCabinet": "Открыть кабинет",
  "check.verdict": "Главное: {text}",
  "check.allGood": "Всё, что можем проверить, в порядке",
  "check.allGoodNote":
    "Если сайты всё равно не открываются — пришлите отчёт, посмотрим вместе.",
  "check.fix": "Починить",
  "check.fixing": "Чиним…",
  "check.report": "Отправить отчёт",
  "check.verdict.clock":
    "часы на устройстве идут неверно, из-за этого не проходит проверка сервера.",
  "check.verdict.internet": "устройство сейчас не в сети.",
  "check.verdict.dns": "имена сайтов не разрешаются — обычно это DNS провайдера.",
  "check.verdict.sub": "наш адрес не отвечает из вашей сети.",
  "check.verdict.handshake": "соединение до сервера не поднимается.",
  "check.verdict.traffic": "соединение есть, а данные через него не идут.",
  "check.verdict.network": "эта сеть пропускает только отдельные сайты.",
  "check.verdict.profile": "ваш профиль не заведён на одном из серверов.",
  "check.verdict.balance": "на подписке закончились средства или срок.",
  "check.verdict.freshness": "настройки подписки устарели.",
  "check.verdict.nodes": "выбранная локация сейчас недоступна.",

  // ── [9] Support report ────────────────────────────────────────────────────
  "report.title": "Отчёт для поддержки",
  "report.lead": "Вот что уйдёт — прочитайте, прежде чем отправлять",
  "report.privacy":
    "В отчёте нет ссылки подписки, адресов серверов и сайтов, которые вы открывали.",
  "report.size": "Размер: {size}",
  "report.openBot": "Открыть бот",
  "report.failed": "Не удалось собрать отчёт",

  // ── [10] Subscription and this device ─────────────────────────────────────
  "sub.title": "Подписка и это устройство",
  "sub.paidUntil": "Доступ оплачен до {date} · осталось {days}",
  "sub.asOf": "по состоянию на {time}",
  "sub.balanceFailed": "Не удалось получить баланс. Срок показан по данным подписки",
  "sub.device": "Это устройство",
  "sub.deviceLine": "{name} · подключено {date}",
  "sub.oneLink":
    "Одна ссылка работает на одном устройстве. Если переносите подписку на другое — сбросьте привязку в кабинете или в боте.",
  "sub.openCabinet": "Открыть кабинет",
  "sub.orBot": "или в боте",
  "sub.topUp": "Пополнить на {host}",

  // ── [11] What happened ────────────────────────────────────────────────────
  "tl.title": "Что происходило",
  "tl.today": "Сегодня",
  "tl.yesterday": "Вчера",
  "tl.earlier": "Ранее",
  "tl.empty": "Пока ничего не происходило — это хороший знак",
  "tl.report": "Отправить отчёт в поддержку",
  "tl.techLog": "Технический журнал",
  "tl.techLog.more": "Показать ещё 200 строк",
  "tl.techLog.empty": "Журнал пуст",
  "tl.techLog.failed": "Не удалось прочитать журнал",
  "tl.ev.connected": "Защита включена, {location}",
  "tl.ev.disconnected": "Защита выключена",
  "tl.ev.healed": "Соединение восстановлено. Новый сервер — {location}",
  "tl.ev.trafficStopped": "Данные перестали идти",
  // Two phrasings on purpose. The core reports THAT the machine changed
  // how it reaches the internet; it does not report Wi-Fi vs mobile, and
  // on macOS it has no cheap way to. So the plain line is the one that
  // actually renders today, and the "на {network}" line waits for the day
  // `network` arrives — rather than leaving a literal "{network}" on screen.
  "tl.ev.networkChanged": "Сеть сменилась",
  "tl.ev.networkChangedTo": "Сеть сменилась на {network}",
  "tl.ev.subRefreshed": "Настройки подписки обновлены",
  "tl.ev.subRefreshedFallback": "Настройки подписки обновлены (запасной адрес)",
  "tl.ev.locationSwitched": "Новый сервер — {location}",
  "tl.ev.locationUnreadable": "Одну локацию не удалось прочитать",
  "tl.ev.failed": "Не удалось подключиться",
  "tl.net.wifi": "Wi-Fi",
  "tl.net.mobile": "мобильную",

  // ── [12] More ─────────────────────────────────────────────────────────────
  "more.title": "Ещё",
  "more.where": "Где вы сейчас: {value}",
  "more.refresh": "Обновить настройки подписки",
  "more.refreshing": "Обновляем…",
  "more.updatedAt": "Обновлено {time}",
  "more.updatedNever": "ещё не обновляли",
  "more.refreshFailed": "Не удалось обновить",
  "more.lang": "Язык",
  "more.theme": "Оформление",
  "more.theme.system": "Как в системе",
  "more.theme.light": "Светлое",
  "more.theme.dark": "Тёмное",
  "more.unlink": "Отвязать ссылку от этого устройства",
  "more.unlinkConfirm":
    "Ссылка будет удалена с этого устройства. Подписка и оплата не тронуты — ссылку можно добавить снова.",
  "more.unlinkYes": "Отвязать",
  "more.cabinet": "Личный кабинет",
  "more.geoNote": "Списки маршрутизации уже внутри приложения — ничего не скачивается",

  // ── Demo strip. Development only, never reachable inside the app ──────────
  "demo.title": "Демо-режим",
  "demo.hint": "Ядро не подключено: экраны играют сценарий из адресной строки.",
} as const;

/** Every key that exists. A typo in `t("…")` is a compile error, not a bug. */
export type MsgKey = keyof typeof ru;

const en: Partial<Record<MsgKey, string>> = {
  "common.close": "Close",
  "common.back": "Back",
  "common.retry": "Retry",
  "common.cancel": "Cancel",
  "common.ok": "Got it",
  "common.copy": "Copy",
  "common.copied": "Copied",
  "common.loading": "Loading…",

  "time.justNow": "just now",
  "time.secondsAgo": "{n} second ago|{n} seconds ago",
  "time.minutesAgo": "{n} minute ago|{n} minutes ago",
  "time.hoursAgo": "{n} hour ago|{n} hours ago",
  "time.daysAgo": "{n} day ago|{n} days ago",
  "time.days": "{n} day|{n} days",
  "unit.kb": "{n} KB",
  "unit.mb": "{n} MB",

  "main.off.title": "Protection is off",
  "main.off.hint": "Tap to turn it on",
  "main.off.action": "Turn on",
  "main.starting.title": "Turning on…",
  "main.starting.action": "Cancel",
  "main.starting.fetchingSub": "Checking the subscription",
  "main.starting.pickingServer": "Picking a server",
  "main.starting.startingEngine": "Preparing the connection",
  "main.starting.raisingTun": "Setting up the network",
  "main.starting.probing": "Checking that data flows",
  "main.on.title": "Protected",
  "main.on.action": "Turn off",
  "main.unconfirmed.title": "Could not confirm",
  "main.unconfirmed.hint": "The tunnel is up, our check page did not answer",
  "main.healing.quiet": "Refreshing the connection…",
  "main.healing.title": "Restoring the connection",
  "main.healing.hint": "The internet will come back on its own",
  "main.failed.action": "What to do",
  "main.nolink.title": "Not set up",
  "main.nolink.hint": "Add your link — it is one step",
  "main.nolink.action": "Add link",
  "main.shield.off": "Protection is off. Tap to turn it on",
  "main.shield.starting": "Turning protection on",
  "main.shield.on": "Protected. Tap to turn it off",
  "main.shield.unconfirmed": "Protection is on, could not be confirmed",
  "main.shield.healing": "Restoring the connection",
  "main.shield.failed": "Protection is not working",
  "main.truth.details": "Connection details",
  "main.truth.rtt": "{n} ms",
  "main.truth.checked": "checked {age}",
  "main.truth.checking": "checking",
  "main.nav.country": "Country",
  "main.nav.check": "Check",
  "main.nav.more": "More",

  "bar.expiry": "{n} day left|{n} days left",
  "bar.expirySoon":
    "{n} day left. Top up to avoid an interruption|{n} days left. Top up to avoid an interruption",
  "bar.expired": "Your access has expired",

  "ob.step": "Step {n} of {total}",
  "ob.move.title": "Move ProxysVPN to Applications",
  "ob.move.body":
    "From the Downloads folder the system will not allow a secure connection. We will move the app ourselves — it will close and open again right away.",
  "ob.move.action": "Move and restart",
  "ob.move.skip": "I will move it myself",
  "ob.move.failed": "Could not move it. Open Downloads and drag ProxysVPN into Applications",
  "ob.pass.title": "The system is about to ask for your password",
  "ob.pass.body":
    "It is needed once per launch, to route the internet through a secure connection. We never see or store it: macOS asks for it itself.",
  "ob.pass.action": "Got it, continue",
  "ob.ios.title": "Allow the VPN configuration",
  "ob.ios.body":
    "iPhone will ask once and request Face ID or your passcode. It will not ask again.",
  "ob.ios.action": "Allow",
  "ob.waiting": "Waiting for the system…",
  "ob.denied": "Permission was not granted. Without it we cannot turn protection on",
  "ob.showWhere": "Show me where to tap",
  "ob.help": "Need help",

  "pair.title": "One thing left: add your link",
  "pair.body":
    "Open your account on your phone, tap the code icon next to the link and point the camera here.",
  "pair.preparing": "Preparing the code…",
  "pair.countdown": "The code is valid for {time}",
  "pair.waiting": "Waiting for your phone…",
  "pair.paste": "Paste the link from the clipboard",
  "pair.codeFailed": "Could not get the code. Check your internet",
  "pair.expired": "The code has expired",
  "pair.refresh": "Refresh the code",
  "pair.notOurLink": "This is not a ProxysVPN link. Get it in the bot: «📡 My devices»",
  "pair.clipboardEmpty": "There is no link in the clipboard",
  "pair.clipboardFailed": "Could not read the clipboard",
  "pair.done": "Done. Link added",
  "pair.keychain": "The link is kept in the system's secure storage",
  "pair.scan": "Point the camera",

  "details.title": "Details",
  "details.change": "Change",
  "details.edit": "Edit",
  "details.locationAuto": "We will pick one when you turn it on",
  "details.whereRu": "You are in Russia — Russian sites go directly",
  "details.whereAbroad": "You are abroad — all traffic goes through us",
  "details.paidUntil": "Paid until {date} · {days} left",
  "details.expiryUnknown": "Expiry unknown — could not get the data",
  "details.split": "Where traffic goes: through the VPN {via} · directly {direct}",
  "details.splitNote":
    "We do not record which sites you open. Only how many connections went where.",
  "details.checkNow": "Check now",
  "details.checkOk": "Everything works. Data is flowing. Current server — {location}",
  "details.sub": "Subscription and this device",

  "heal.title": "Restoring the connection",
  "heal.body": "The internet will come back on its own. This usually takes a few seconds.",
  "heal.step.checkNode": "Checked the server",
  "heal.step.restartEngine": "Restarted the connection",
  "heal.step.otherNode": "Tried another server",
  "heal.step.otherProto": "Trying another way to connect",
  "heal.step.refreshSub": "Refreshing the subscription settings",
  "heal.step.askService": "Asking the service",
  "heal.cancel": "Cancel and turn protection off",
  "heal.ok": "Connection restored",
  "heal.fail": "Not working yet. Let me show you why",

  "loc.title": "Countries",
  "loc.auto": "Automatic",
  "loc.autoHint": "We pick the fastest one available",
  "loc.current": "Now: {location}",
  "loc.recent": "Recent",
  "loc.all": "All countries",
  "loc.unnamed": "Server #{n}",
  "loc.q.good": "fast",
  "loc.q.ok": "medium",
  "loc.q.poor": "slow",
  "loc.q.blocked": "not getting through on your network right now",
  "loc.q.unknown": "we will check on connect",
  "loc.pinNote": "The chosen country stays for 24 hours. You can return to automatic here.",
  "loc.refreshing": "Refreshing the list…",
  "loc.stale": "List from {time}, could not refresh",
  "loc.refresh": "Refresh",
  "loc.switching": "Switching…",
  "loc.switched": "Done. New server — {location}",
  "loc.pinned": "Pinned for 24 hours",

  "where.title": "Where are you?",
  "where.ru": "I am in Russia",
  "where.ruBody":
    "Russian sites, banks and government services open directly — it is faster and nothing breaks.",
  "where.abroad": "I am abroad",
  "where.abroadBody":
    "All traffic goes through us, Russian sites included. That is how RuTube, banks and calls work.",
  "where.guessRu": "Our guess: you appear to be in Russia.",
  "where.guessAbroad": "Our guess: you appear to be abroad.",
  "where.failed": "Could not apply, left as it was",
  "where.short.ru": "In Russia",
  "where.short.abroad": "Abroad",

  "problem.title": "What happened",
  "problem.check": "Check",
  "problem.timeline": "What happened earlier",
  "problem.working": "Working…",

  "action.addLink": "Add link",
  "action.retry": "Retry",
  "action.openCabinet": "Open account",
  "action.topUp": "Top up",
  "action.diagnose": "Run a check",
  "action.waitAndSee": "Got it",
  "action.contactSupport": "Message support",

  "err.NO_SUBSCRIPTION.title": "No link added",
  "err.NO_SUBSCRIPTION.body": "Without it there is nothing to turn on. This is one step.",
  "err.SUB_MALFORMED.title": "This link does not fit",
  "err.SUB_MALFORMED.body":
    "Looks like the wrong text was copied. Get the link in the bot: «📡 My devices».",
  "err.SUB_UNREACHABLE.title": "Could not fetch the settings",
  "err.SUB_UNREACHABLE.body":
    "We tried every fallback address. This is usually the network you are on: try again, or switch between Wi-Fi and mobile data.",
  "err.SUB_INVALID.title": "The settings came back unreadable",
  "err.SUB_INVALID.body":
    "The server answered, but we could not read the answer. This happens when the network has a middlebox in the way.",
  "err.SUB_EMPTY.title": "The subscription has no locations",
  "err.SUB_EMPTY.body":
    "The service answered with an empty list. Check the subscription in your account — that is usually where the answer is.",
  "err.BALANCE_EMPTY.title": "Out of funds",
  "err.BALANCE_EMPTY.body": "Access resumes right after a top-up.",
  "err.EXPIRED.title": "Your access has expired",
  "err.EXPIRED.body": "Renew it and protection turns on with the same tap.",
  "err.DEVICE_TAKEN.title": "This link is taken by another device",
  "err.DEVICE_TAKEN.body":
    "One link works on one device. To move the subscription here, reset the binding in your account or in the bot.",
  "err.NO_DEVICES.title": "No devices on this account",
  "err.NO_DEVICES.body": "Add a device in your account and the link starts working at once.",
  "err.SUB_NOTICE.title": "A message from the service",
  "err.SUB_NOTICE.body":
    "The service answered instead of sending the list. Here is what it says:",
  "err.PERMISSION_DENIED.title": "The system needs your permission",
  "err.PERMISSION_DENIED.body":
    "Without it we cannot route the internet through a secure connection. Tap Retry and allow it in the system dialog.",
  "err.ENGINE_START_FAILED.title": "The connection did not start",
  "err.ENGINE_START_FAILED.body":
    "The internal part did not come up. Tapping again usually helps.",
  "err.ENGINE_DIED.title": "The connection dropped",
  "err.ENGINE_DIED.body": "We are already bringing it back — there is nothing to do.",
  "err.PORT_BUSY.title": "The previous run has not closed yet",
  "err.PORT_BUSY.body": "An old connection is still holding the app. Tap Retry — we will close it.",
  "err.TUN_FAILED.title": "Could not set up the network",
  "err.TUN_FAILED.body":
    "The system network connection did not come up. Most often another VPN running at the same time is in the way.",
  "err.NETWORK_OFFLINE.title": "No internet",
  "err.NETWORK_OFFLINE.body":
    "This device is offline. We will turn protection on ourselves once the internet is back.",
  "err.NO_ROUTE.title": "The server is unreachable from your network",
  "err.NO_ROUTE.body":
    "Nothing reaches the chosen server — that is what a blocked address looks like. We are already trying others.",
  "err.BLOCKED.title": "There is a connection, but sites do not open",
  "err.BLOCKED.body":
    "The connection is established and no data moves through it. Usually this is filtering by your ISP: another country or another way to connect will help.",
  "err.PROBE_UNCONFIRMED.title": "Could not confirm",
  "err.PROBE_UNCONFIRMED.body":
    "The tunnel is up and data is moving, but our check page did not answer. It may all be fine — we just cannot prove it.",
  "err.PING_NOT_APPLICABLE.title": "Latency is not measured",
  "err.PING_NOT_APPLICABLE.body":
    "This way of connecting gives no round-trip figure. Nothing is broken: protection works, there simply is no number for it.",
  "err.UNKNOWN.title": "Something went wrong",
  "err.UNKNOWN.body":
    "We could not name the cause. Send the report — it has everything needed to work it out.",

  "check.title": "Check",
  "check.device": "On this device",
  "check.service": "On the service side",
  "check.run": "Run the check",
  "check.running": "Checking…",
  "check.again": "Check again",
  "check.notChecked": "not checked",
  "check.row.clock": "The device clock",
  "check.row.internet": "Internet is there",
  "check.row.dns": "Site names resolve",
  "check.row.sub": "Our address answers",
  "check.row.handshake": "The connection comes up",
  "check.row.traffic": "Data is moving",
  "check.row.network": "The network allows VPN",
  "check.row.balance": "Balance and expiry",
  "check.row.profile": "Profile on the servers",
  "check.row.freshness": "Subscription freshness",
  "check.row.nodes": "State of the locations",
  "check.note.fallback": "fallback",
  "check.serviceInCabinet": "The service-side check lives in your account",
  "check.serviceFailed": "Could not check on our side",
  "check.openCabinet": "Open account",
  "check.verdict": "The main thing: {text}",
  "check.allGood": "Everything we can check is fine",
  "check.allGoodNote": "If sites still do not open — send the report and we will look together.",
  "check.fix": "Fix it",
  "check.fixing": "Fixing…",
  "check.report": "Send report",
  "check.verdict.clock": "the device clock is wrong, so the server check cannot pass.",
  "check.verdict.internet": "this device is offline.",
  "check.verdict.dns": "site names do not resolve — usually your ISP's DNS.",
  "check.verdict.sub": "our address does not answer from your network.",
  "check.verdict.handshake": "the connection to the server does not come up.",
  "check.verdict.traffic": "there is a connection and no data moves through it.",
  "check.verdict.network": "this network only allows a handful of sites.",
  "check.verdict.profile": "your profile is missing on one of the servers.",
  "check.verdict.balance": "the subscription is out of funds or time.",
  "check.verdict.freshness": "the subscription settings are out of date.",
  "check.verdict.nodes": "the chosen location is unavailable right now.",

  "report.title": "Support report",
  "report.lead": "This is what will be sent — read it before you send",
  "report.privacy":
    "The report contains no subscription link, no server addresses and none of the sites you opened.",
  "report.size": "Size: {size}",
  "report.openBot": "Open the bot",
  "report.failed": "Could not build the report",

  "sub.title": "Subscription and this device",
  "sub.paidUntil": "Access paid until {date} · {days} left",
  "sub.asOf": "as of {time}",
  "sub.balanceFailed": "Could not get the balance. Expiry is shown from the subscription",
  "sub.device": "This device",
  "sub.deviceLine": "{name} · connected {date}",
  "sub.oneLink":
    "One link works on one device. If you are moving the subscription elsewhere — reset the binding in your account or in the bot.",
  "sub.openCabinet": "Open account",
  "sub.orBot": "or in the bot",
  "sub.topUp": "Top up at {host}",

  "tl.title": "What happened",
  "tl.today": "Today",
  "tl.yesterday": "Yesterday",
  "tl.earlier": "Earlier",
  "tl.empty": "Nothing has happened yet — that is a good sign",
  "tl.report": "Send a report to support",
  "tl.techLog": "Technical log",
  "tl.techLog.more": "Show 200 more lines",
  "tl.techLog.empty": "The log is empty",
  "tl.techLog.failed": "Could not read the log",
  "tl.ev.connected": "Protection on, {location}",
  "tl.ev.disconnected": "Protection off",
  "tl.ev.healed": "Connection restored. New server — {location}",
  "tl.ev.trafficStopped": "Data stopped moving",
  "tl.ev.networkChanged": "The network changed",
  "tl.ev.networkChangedTo": "The network changed to {network}",
  "tl.ev.subRefreshed": "Subscription settings refreshed",
  "tl.ev.subRefreshedFallback": "Subscription settings refreshed (fallback address)",
  "tl.ev.locationSwitched": "New server — {location}",
  "tl.ev.locationUnreadable": "One location could not be read",
  "tl.ev.failed": "Could not connect",
  "tl.net.wifi": "Wi-Fi",
  "tl.net.mobile": "mobile",

  "more.title": "More",
  "more.where": "Where you are: {value}",
  "more.refresh": "Refresh the subscription settings",
  "more.refreshing": "Refreshing…",
  "more.updatedAt": "Refreshed at {time}",
  "more.updatedNever": "not refreshed yet",
  "more.refreshFailed": "Could not refresh",
  "more.lang": "Language",
  "more.theme": "Appearance",
  "more.theme.system": "Follow the system",
  "more.theme.light": "Light",
  "more.theme.dark": "Dark",
  "more.unlink": "Unlink this device from the subscription",
  "more.unlinkConfirm":
    "The link will be removed from this device. The subscription and payment are untouched — you can add it again.",
  "more.unlinkYes": "Unlink",
  "more.cabinet": "Your account",
  "more.geoNote": "Routing lists are already inside the app — nothing is downloaded",

  "demo.title": "Demo mode",
  "demo.hint": "No core attached: the screens play a scenario from the address bar.",
};

const DICTS: Record<Lang, Partial<Record<MsgKey, string>>> = { ru, en };

export type TParams = Record<string, string | number>;

/**
 * `t("key", { n: 3 })`.
 *
 * Resolution order is deliberate: the requested language, then Russian. The
 * key itself is never rendered — `MsgKey` makes an unknown key impossible.
 */
export type Translate = (key: MsgKey, params?: TParams) => string;

/** one / few / many for Russian, one / other for everything else. */
function pluralIndex(lang: Lang, n: number): number {
  if (lang !== "ru") return n === 1 ? 0 : 1;
  const abs = Math.abs(Math.trunc(n));
  const mod10 = abs % 10;
  const mod100 = abs % 100;
  if (mod10 === 1 && mod100 !== 11) return 0;
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) return 1;
  return 2;
}

function fill(template: string, params: TParams | undefined, lang: Lang): string {
  let text = template;
  if (text.includes("|")) {
    const forms = text.split("|");
    const raw = params?.n;
    const n = typeof raw === "number" ? raw : Number(raw ?? 0);
    const index = Math.min(pluralIndex(lang, Number.isFinite(n) ? n : 0), forms.length - 1);
    text = forms[index];
  }
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (whole: string, name: string) => {
    const value = params[name];
    // Leave an unknown placeholder visible: "{days}" during testing is better
    // than a silent gap nobody notices.
    return value === undefined ? whole : String(value);
  });
}

export function makeT(lang: Lang): Translate {
  const primary = DICTS[lang];
  return (key, params) => {
    const translated = primary[key];
    if (translated !== undefined) return fill(translated, params, lang);
    const russian: string | undefined = ru[key];
    // `MsgKey` makes a missing key a compile error, so this branch only fires
    // when the CONTRACT grew a code after this build — errors.rs and types.ts
    // can add one without us. Showing the key is ugly, but a blank screen or
    // a crash on a failure screen is worse.
    if (russian === undefined) return key;
    // The plural rule follows the language the SENTENCE is in, which is
    // Russian whenever we fell back — otherwise "5 минуты назад".
    return fill(russian, params, "ru");
  };
}

// ── Formatting shared by every screen ───────────────────────────────────────

export function formatDate(lang: Lang, unixMs: number): string {
  return new Intl.DateTimeFormat(INTL_LOCALE[lang], {
    day: "numeric",
    month: "long",
  }).format(new Date(unixMs));
}

export function formatTime(lang: Lang, unixMs: number): string {
  return new Intl.DateTimeFormat(INTL_LOCALE[lang], {
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(unixMs));
}

export function formatNumber(lang: Lang, value: number): string {
  return new Intl.NumberFormat(INTL_LOCALE[lang]).format(value);
}

/** "2 КБ" / "1,4 МБ". The size is shown so a person sees it is not megabytes. */
export function formatSize(t: Translate, lang: Lang, bytes: number): string {
  if (bytes < 1024 * 1024) {
    return t("unit.kb", { n: formatNumber(lang, Math.max(1, Math.round(bytes / 1024))) });
  }
  return t("unit.mb", { n: formatNumber(lang, Math.round((bytes / 1024 / 1024) * 10) / 10) });
}

/**
 * "9 секунд назад" → "2 минуты назад". The age of the last proof of life is
 * always on screen, so a stale green is visibly stale instead of silently
 * wrong.
 */
export function formatAge(t: Translate, sinceMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.round((nowMs - sinceMs) / 1000));
  if (seconds < 5) return t("time.justNow");
  if (seconds < 60) return t("time.secondsAgo", { n: seconds });
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return t("time.minutesAgo", { n: minutes });
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t("time.hoursAgo", { n: hours });
  return t("time.daysAgo", { n: Math.round(hours / 24) });
}

/** "4:31" — a countdown a person reads at a glance. */
export function formatCountdown(msLeft: number): string {
  const total = Math.max(0, Math.round(msLeft / 1000));
  const mm = Math.floor(total / 60);
  const ss = total % 60;
  return `${mm}:${String(ss).padStart(2, "0")}`;
}

/** Whole days left, rounded up: half a day left is still "1 день". */
export function daysLeft(expiresAtMs: number, nowMs: number): number {
  return Math.ceil((expiresAtMs - nowMs) / 86_400_000);
}
