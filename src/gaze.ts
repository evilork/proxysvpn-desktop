// src/gaze.ts
//
// Куда смотрит глаз на марке.
//
// Ровно одно число на две оси, в долях от -1 до 1, и один источник правды о
// том, откуда оно берётся:
//
//   телефон  — наклон корпуса. В приложении на iPhone и iPad - из CoreMotion
//              через ядро (motion.rs, MotionBridge.swift), в браузере - из
//              DeviceOrientationEvent. Зрачок смещается ПРОТИВ наклона,
//              поэтому взгляд остаётся там же, где был, пока поворачивается
//              сам аппарат.
//   десктоп  — положение курсора. Гироскопа нет, а «смотреть на человека» там
//              значит смотреть на его указатель.
//
// Почему не абсолютный угол. Человек держит телефон под своим углом: кто-то
// почти горизонтально в кровати, кто-то вертикально в метро. Абсолютное
// «beta = 0» означало бы, что в кровати глаз всегда закатан вверх. Поэтому
// отсчёт идёт от базового угла, снятого при первом же показании, а сам базовый
// угол очень медленно подтягивается к текущему (постоянная времени около 20 с).
// Быстрый наклон глаз отрабатывает, смену позы - нет.
//
// Разрешение на датчики - только по явному касанию эмблемы (Shield.tsx), и
// никогда само по себе. Раньше вопрос висел на ЛЮБОМ первом жесте, и система
// спрашивала про движение и ориентацию в ответ на кнопку «Разрешить VPN» или
// «Продолжить» - ради украшения, посреди настройки (пункт 5.1.1 правил Apple: просить
// только то, что нужно для дела). В сборках для App Store вопроса нет вовсе:
// веб-датчик не слушается и не запрашивается, а наклон приходит из CoreMotion
// самого приложения - ему разрешение не нужно, и показания не покидают
// устройство (владелец 28.09.2026: «чтобы глаз двигался от наклона, как с
// курсором на маке»).

import { IS_APPSTORE } from "./dist";

/**
 * Насколько зрачок отходит от центра, в единицах холста марки (1720x1804).
 *
 * Не на глаз. Прорезь снята с марки 109 вершинами, и для каждого смещения
 * считалось расстояние от центра зрачка до ближайшей стенки: ход кончается
 * там, где круг радиусом 92 упёрся бы в веко. Зрачок обязан оставаться
 * ЦЕЛЫМ кругом - это требование владельца, поэтому серп на краю не
 * допускается, и ход ограничен вписанным эллипсом, а не габаритом прорези.
 *
 * Числа идут в паре с точкой покоя из Eye.tsx: зрачок стоит в середине
 * видимого белка (468), и оттуда ход симметричен. Пока он стоял в точке из
 * марки (537.4), у правого края коридора, симметричный ход упирался в 51 -
 * глаз топтался в правой половине белка.
 *
 * Запас над веком 4 единицы по всему краю хода: проверено обходом эллипса с
 * шагом в градус. Без запаса зрачок целовал веко на сглаживании.
 */
export const GAZE_RANGE = { x: 92, y: 23 } as const;

/** Наклон, при котором зрачок доходит до края хода, в градусах. */
const FULL_TILT_DEG = 22;

/**
 * Пружина взгляда вместо доли пути за кадр.
 *
 * Было `current += (target - current) * 0.12` - показательное затухание: старт
 * вялый, хвост длинный, глаз словно едет по маслу. Живой глаз ходит
 * саккадами: резко трогается, быстро встаёт, чуть перелетает.
 *
 * Пружина это и даёт. Затухание 0,84 от критического - перелёт есть, но
 * ровно на грани заметности; на 63 % пути глаз приходит примерно за 40 мс
 * вместо прежних 130.
 */
const STIFFNESS = 240;
const DAMPING = 26;

/** Доехали, когда и ошибка, и скорость малы. Иначе цикл крутится вечно. */
const SETTLED = 0.002;
const SETTLED_SPEED = 0.02;

/** Потолок шага. Вкладка была свёрнута - не выстреливать глазом на полкадра. */
const MAX_STEP_SEC = 1 / 30;

/**
 * Постоянная времени, за которую базовый угол догоняет текущий, в секундах.
 *
 * По времени, а не «за показание»: веб-датчик шлёт 60 показаний в секунду, а
 * датчик приложения - 30 и только когда угол сдвинулся (плюс контрольное раз
 * в 200 мс). Доля «за показание» при разной частоте дала бы разную позу.
 */
const BASE_DRIFT_SEC = 20;

/** Потолок шага сползания: после паузы в показаниях не прыгать базой. */
const MAX_DRIFT_STEP_SEC = 0.25;

export interface Gaze {
  /** -1 влево, 1 вправо. */
  x: number;
  /** -1 вверх, 1 вниз. */
  y: number;
}

export const CENTRED: Gaze = { x: 0, y: 0 };

/** Снимок состояния источника - чтобы на телефоне не гадать, а видеть. */
export interface GazeReport {
  secure: boolean;
  needsPermission: boolean;
  asked: boolean;
  granted: boolean | null;
  reducedMotion: boolean;
  tiltEvents: number;
  pointerEvents: number;
  lastTilt: { beta: number; gamma: number } | null;
  listeners: number;
  source: "sensor" | "pointer" | "none";
  /** Датчик приложения: null - не пробовали, false - его нет, true - идёт. */
  native: boolean | null;
}

/**
 * Наклон от самого приложения (CoreMotion через ядро), когда оно есть.
 *
 * Внедряется снаружи (App.tsx), а не импортом моста: модуль остаётся чистым,
 * и тесты гоняют его без Tauri.
 */
export interface NativeTilt {
  /** false - датчика нет (Mac, браузер). */
  start(): Promise<boolean>;
  stop(): Promise<void>;
  subscribe(fn: (reading: { beta: number; gamma: number }) => void): () => void;
}

let nativeTilt: NativeTilt | null = null;

/** Подключить наклон приложения. Вызывать до первого показа глаза. */
export function setNativeTilt(source: NativeTilt | null): void {
  nativeTilt = source;
}

/**
 * Safari на iOS требует разрешения на датчики и выдаёт его только в ответ на
 * действие человека. В типах DOM этого метода нет, поэтому сужаем сами - без
 * `any`, который запрещён в этом проекте.
 */
type MotionGate = typeof DeviceOrientationEvent & {
  requestPermission?: () => Promise<PermissionState | "granted" | "denied">;
};

function motionGate(): MotionGate | null {
  if (typeof DeviceOrientationEvent === "undefined") return null;
  return DeviceOrientationEvent as MotionGate;
}

/**
 * Нужно ли вообще спрашивать. False - датчик или уже открыт, или его нет, или
 * это сборка для App Store, где его не спрашивают никогда.
 */
export function motionNeedsPermission(): boolean {
  if (IS_APPSTORE) return false;
  const gate = motionGate();
  return typeof gate?.requestPermission === "function";
}

/**
 * Спросить разрешение на датчики.
 *
 * Вызывать ТОЛЬКО из обработчика касания или щелчка: вне жеста Safari молча
 * отказывает, и повторно спросить уже нельзя. Возвращает false и там, где
 * спрашивать нечего, - вызывающему это неинтересно, глаз просто останется на
 * указателе.
 */
export async function requestMotionAccess(): Promise<boolean> {
  if (IS_APPSTORE) return false;
  const gate = motionGate();
  if (typeof gate?.requestPermission !== "function") return false;
  try {
    return (await gate.requestPermission()) === "granted";
  } catch {
    // Вне жеста и в небезопасном контексте метод бросает. Это не ошибка
    // приложения: живой взгляд - украшение, без него всё работает.
    return false;
  }
}

let asked = false;
let granted: boolean | null = null;

/**
 * Спросить про датчик - из касания эмблемы, и только оттуда.
 *
 * Спрашивать разрешение можно только изнутри жеста, поэтому вызывать это
 * вправе лишь обработчик касания эмблемы (Shield.tsx): там человек сам
 * тронул глаз. Любой другой жест - кнопка настройки, «Продолжить», шестерёнка
 * - о датчиках не спрашивает; глаз тогда просто следит за касанием.
 *
 * Вызов однократный, ничего не ждёт и ничего не ломает: отказ означает лишь,
 * что взгляд останется на указателе. В сборках для App Store ничего не делает.
 */
export function primeGaze(): void {
  if (asked || !motionNeedsPermission()) return;
  asked = true;
  void requestMotionAccess().then((ok) => {
    granted = ok;
  });
}

/**
 * Защищённое соединение - обязательное условие, а не пожелание.
 *
 * Safari отдаёт наклон и ускорение только в secure context. На
 * `http://192.168.x.x:1425` датчика не будет никогда, сколько ни разрешай:
 * запрос либо бросит, либо ответит отказом, и ни одного события не придёт.
 * Работает localhost и работает https - больше ничего.
 */
export function motionNeedsSecureContext(): boolean {
  return typeof window !== "undefined" && !window.isSecureContext;
}

/**
 * Где сейчас находится сам глаз, в координатах окна.
 *
 * Без этого «следить за стрелкой» невозможно: раньше взгляд считался от
 * ЦЕНТРА ОКНА, то есть глаз смотрел не на курсор, а в сторону той половины
 * окна, где курсор оказался. Стоило эмблеме быть не в середине - и взгляд
 * всегда косил. Якорь ставит сам глаз (Eye.tsx), потому что только он знает
 * свой прямоугольник.
 */
export interface GazeAnchorPoint {
  /** Центр САМОГО глаза в координатах окна, не центр эмблемы. */
  cx: number;
  cy: number;
  /** Расстояние, на котором ход выбран примерно на три четверти. */
  reach: number;
}

type GazeAnchor = () => GazeAnchorPoint | null;

let anchor: GazeAnchor | null = null;

export function setGazeAnchor(fn: GazeAnchor | null): void {
  anchor = fn;
}

/**
 * Курсор - во взгляд.
 *
 * Насыщение через гиперболический тангенс, а не обрезка: у обрезки есть
 * стенка, за которой глаз мёртв, и человек её чувствует. Тангенс не доходит
 * до края никогда, поэтому дальний угол окна всё равно чуть-чуть двигает
 * зрачок, а вблизи ход почти линейный.
 *
 * Мера расстояния приходит вместе с якорем и берётся от размера эмблемы, а не
 * в пикселях: одинаково работает и в узком окне, и на большом экране.
 *
 * @param px,py  курсор в координатах окна
 * @param at     где глаз; null - якоря нет, считаем от центра окна
 */
export function pointerToGaze(
  px: number,
  py: number,
  at: GazeAnchorPoint | null,
  viewport: { width: number; height: number },
): Gaze {
  const cx = at ? at.cx : viewport.width / 2;
  const cy = at ? at.cy : viewport.height / 2;
  const reach = Math.max(
    at ? at.reach : Math.min(viewport.width, viewport.height) / 2,
    1,
  );
  return { x: Math.tanh((px - cx) / reach), y: Math.tanh((py - cy) / reach) };
}

function clamp1(v: number): number {
  return v < -1 ? -1 : v > 1 ? 1 : v;
}

/**
 * Наклон относительно базового угла - в смещение зрачка.
 *
 * Вынесено отдельной чистой функцией, потому что это единственное место, где
 * есть что испортить: знак, оси и поворот экрана. Всё остальное в модуле -
 * подписки и сглаживание.
 *
 * @param base     угол, от которого считаем
 * @param reading  текущий угол датчика
 * @param angle    поворот экрана в градусах (0 / 90 / 180 / 270)
 */
export function tiltToGaze(
  base: { beta: number; gamma: number },
  reading: { beta: number; gamma: number },
  angle: number,
): Gaze {
  const dBeta = reading.beta - base.beta;
  const dGamma = reading.gamma - base.gamma;

  // Оси датчика привязаны к корпусу, а человек смотрит на экран: в альбомной
  // ориентации они развёрнуты друг относительно друга.
  const a = (angle * Math.PI) / 180;
  const cos = Math.cos(a);
  const sin = Math.sin(a);
  const sx = dGamma * cos - dBeta * sin;
  const sy = dBeta * cos + dGamma * sin;

  // Минус и есть вся затея: корпус ушёл вправо - зрачок уходит влево, и
  // взгляд остаётся там же, где был.
  return {
    x: clamp1(-sx / FULL_TILT_DEG),
    y: clamp1(-sy / FULL_TILT_DEG),
  };
}

/** Угол поворота экрана, чтобы наклон читался одинаково в любой ориентации. */
function screenAngle(): number {
  if (typeof screen === "undefined") return 0;
  const angle = screen.orientation?.angle;
  return typeof angle === "number" ? angle : 0;
}

/**
 * Живой источник взгляда.
 *
 * Подписка создаётся один раз на приложение: слушателей движения и кадровый
 * цикл незачем плодить на каждый компонент. `subscribe` возвращает отписку;
 * когда уходит последний подписчик, всё снимается и цикл встаёт.
 */
class GazeSource {
  private listeners = new Set<(g: Gaze) => void>();
  private target: Gaze = { x: 0, y: 0 };
  private current: Gaze = { x: 0, y: 0 };
  /** Скорость зрачка по осям, долей хода в секунду. Это и есть пружина. */
  private vx = 0;
  private vy = 0;
  private lastTs = 0;
  private base: { beta: number; gamma: number } | null = null;
  private frame = 0;
  private fromSensor = false;
  private bound = false;
  private tiltEvents = 0;
  private pointerEvents = 0;
  private lastTilt: { beta: number; gamma: number } | null = null;
  private lastTiltAt = 0;
  /** Отписка от наклона приложения, пока он подключён. */
  private nativeOff: (() => void) | null = null;
  /**
   * Номер последнего запуска датчика. Ответ на `start` может прийти, когда
   * глаз уже закрыли или запустили снова, - тогда он чужой и не считается.
   */
  private nativeRun = 0;
  private nativeOn: boolean | null = null;

  /** Что источник видит на самом деле. Для отладки на чужом устройстве. */
  report(): GazeReport {
    return {
      secure: typeof window !== "undefined" && window.isSecureContext,
      needsPermission: motionNeedsPermission(),
      asked,
      granted,
      reducedMotion: motionIsUnwelcome(),
      tiltEvents: this.tiltEvents,
      pointerEvents: this.pointerEvents,
      lastTilt: this.lastTilt,
      listeners: this.listeners.size,
      source: this.fromSensor ? "sensor" : this.pointerEvents > 0 ? "pointer" : "none",
      native: this.nativeOn,
    };
  }

  subscribe(fn: (g: Gaze) => void): () => void {
    this.listeners.add(fn);
    this.bind();
    fn(this.current);
    return () => {
      this.listeners.delete(fn);
      if (this.listeners.size === 0) this.unbind();
    };
  }

  private bind(): void {
    if (this.bound || typeof window === "undefined") return;
    this.bound = true;
    // Датчик приложения главнее веб-датчика: ему не нужно разрешение. Без
    // приложения (браузер) - веб-датчик, но не в сборке для App Store.
    if (nativeTilt) this.startNative(nativeTilt);
    else if (!IS_APPSTORE) window.addEventListener("deviceorientation", this.onTilt);
    window.addEventListener("pointermove", this.onPointer, { passive: true });
    window.addEventListener("pointerout", this.onPointerOut, { passive: true });
    document.addEventListener("visibilitychange", this.onVisibility);
  }

  private unbind(): void {
    if (!this.bound) return;
    this.bound = false;
    window.removeEventListener("deviceorientation", this.onTilt);
    this.stopNative();
    window.removeEventListener("pointermove", this.onPointer);
    window.removeEventListener("pointerout", this.onPointerOut);
    document.removeEventListener("visibilitychange", this.onVisibility);
    if (this.frame) cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.target = { x: 0, y: 0 };
    this.current = { x: 0, y: 0 };
    this.vx = 0;
    this.vy = 0;
    this.lastTs = 0;
    this.base = null;
    this.lastTiltAt = 0;
    this.fromSensor = false;
  }

  private onVisibility = (): void => {
    // Вернулись в приложение, возможно в другой позе: базовый угол снимаем
    // заново, чтобы глаз не остался увезённым в сторону.
    if (document.visibilityState === "hidden") {
      this.base = null;
      this.lastTiltAt = 0;
      this.target = { x: 0, y: 0 };
      this.run();
      // Свёрнутое приложение датчик не держит: батарея дороже украшения.
      this.stopNative();
    } else if (this.bound && nativeTilt && !this.nativeOff && this.nativeOn !== false) {
      // Вернулись в приложение - датчик снова нужен. Где его нет (Mac), не
      // спрашиваем повторно.
      this.startNative(nativeTilt);
    }
  };

  private startNative(source: NativeTilt): void {
    const run = ++this.nativeRun;
    this.nativeOff = source.subscribe(this.onNativeTilt);
    source
      .start()
      .then((on) => {
        if (run !== this.nativeRun) {
          // Пока ждали ответа, глаз закрыли: датчик больше никому не нужен.
          if (on && !this.nativeOff) void source.stop().catch(() => undefined);
          return;
        }
        this.nativeOn = on;
        if (!on) this.dropNative();
      })
      .catch(() => {
        // Ядро не ответило - глаз просто остаётся на касании и указателе.
        if (run === this.nativeRun) {
          this.nativeOn = false;
          this.dropNative();
        }
      });
  }

  private stopNative(): void {
    if (!this.nativeOff) return;
    this.nativeRun += 1;
    this.dropNative();
    if (nativeTilt) void nativeTilt.stop().catch(() => undefined);
  }

  private dropNative(): void {
    this.nativeOff?.();
    this.nativeOff = null;
  }

  private onNativeTilt = (reading: { beta: number; gamma: number }): void => {
    this.applyTilt(reading.beta, reading.gamma);
  };

  private onTilt = (e: DeviceOrientationEvent): void => {
    const { beta, gamma } = e;
    if (beta === null || gamma === null) return;
    this.applyTilt(beta, gamma);
  };

  /** Общий путь для обоих датчиков: одни углы, одни правила. */
  private applyTilt(beta: number, gamma: number): void {
    this.tiltEvents += 1;
    this.lastTilt = { beta, gamma };
    this.fromSensor = true;

    const now = typeof performance !== "undefined" ? performance.now() : Date.now();
    const dt = this.lastTiltAt ? Math.min((now - this.lastTiltAt) / 1000, MAX_DRIFT_STEP_SEC) : 0;
    this.lastTiltAt = now;

    if (!this.base) {
      this.base = { beta, gamma };
      return;
    }

    this.target = tiltToGaze(this.base, { beta, gamma }, screenAngle());

    // Базовый угол медленно ползёт за текущим: быстрый наклон глаз
    // отрабатывает, смену позы - нет.
    const drift = Math.min(1, dt / BASE_DRIFT_SEC);
    this.base = {
      beta: this.base.beta + (beta - this.base.beta) * drift,
      gamma: this.base.gamma + (gamma - this.base.gamma) * drift,
    };

    this.run();
  }

  private onPointer = (e: PointerEvent): void => {
    this.pointerEvents += 1;
    // Датчик главнее: если он заговорил, мышь больше не вмешивается.
    if (this.fromSensor) return;
    this.target = pointerToGaze(e.clientX, e.clientY, anchor ? anchor() : null, {
      width: window.innerWidth || 1,
      height: window.innerHeight || 1,
    });
    this.run();
  };

  /**
   * Курсор ушёл из окна - глаз возвращается прямо.
   *
   * Иначе он застывает скошенным в ту сторону, куда мышь вышла, и это читается
   * как зависшая анимация, а не как живой взгляд.
   */
  private onPointerOut = (e: PointerEvent): void => {
    if (this.fromSensor) return;
    if (e.relatedTarget !== null) return;
    this.target = { x: 0, y: 0 };
    this.run();
  };

  private run(): void {
    if (this.frame) return;
    this.lastTs = 0;
    const step = (ts: number): void => {
      // Шаг по реальному времени, а не по кадру: на 120-герцовом экране глаз
      // обязан двигаться с той же скоростью, что и на 60-герцовом.
      const dt = this.lastTs ? Math.min((ts - this.lastTs) / 1000, MAX_STEP_SEC) : 1 / 60;
      this.lastTs = ts;

      const dx = this.target.x - this.current.x;
      const dy = this.target.y - this.current.y;
      this.vx += (STIFFNESS * dx - DAMPING * this.vx) * dt;
      this.vy += (STIFFNESS * dy - DAMPING * this.vy) * dt;
      this.current = { x: this.current.x + this.vx * dt, y: this.current.y + this.vy * dt };
      for (const fn of this.listeners) fn(this.current);

      const speed = Math.max(Math.abs(this.vx), Math.abs(this.vy));
      if (Math.abs(dx) < SETTLED && Math.abs(dy) < SETTLED && speed < SETTLED_SPEED) {
        // Доехали. Ставим точно в цель, гасим скорость и отпускаем кадры:
        // иначе цикл крутится вечно ради движения в тысячную долю хода.
        this.current = { ...this.target };
        this.vx = 0;
        this.vy = 0;
        for (const fn of this.listeners) fn(this.current);
        this.frame = 0;
        return;
      }
      this.frame = requestAnimationFrame(step);
    };
    this.frame = requestAnimationFrame(step);
  }
}

export const gazeSource = new GazeSource();

/** Человек попросил систему не анимировать - значит и глаз не шевелится. */
export function motionIsUnwelcome(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}
