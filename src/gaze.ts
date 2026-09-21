// src/gaze.ts
//
// Куда смотрит глаз на марке.
//
// Ровно одно число на две оси, в долях от -1 до 1, и один источник правды о
// том, откуда оно берётся:
//
//   телефон  — наклон корпуса (DeviceOrientationEvent). Зрачок смещается
//              ПРОТИВ наклона, поэтому взгляд остаётся там же, где был, пока
//              поворачивается сам аппарат.
//   десктоп  — положение курсора. Гироскопа нет, а «смотреть на человека» там
//              значит смотреть на его указатель.
//
// Почему не абсолютный угол. Человек держит телефон под своим углом: кто-то
// почти горизонтально в кровати, кто-то вертикально в метро. Абсолютное
// «beta = 0» означало бы, что в кровати глаз всегда закатан вверх. Поэтому
// отсчёт идёт от базового угла, снятого при первом же показании, а сам базовый
// угол очень медленно подтягивается к текущему (постоянная времени около 20 с).
// Быстрый наклон глаз отрабатывает, смену позы - нет.

/** Насколько зрачок отходит от центра, в единицах холста марки (1720x1804). */
export const GAZE_RANGE = { x: 38, y: 16 } as const;

/** Наклон, при котором зрачок доходит до края хода, в градусах. */
const FULL_TILT_DEG = 22;

/** Доля пути к цели за кадр. При 60 к/с это примерно четверть секунды. */
const FOLLOW = 0.12;

/** Меньше этого считаем, что доехали, и останавливаем цикл. */
const SETTLED = 0.002;

/** Скорость сползания базового угла за кадр: 20 с при 60 к/с. */
const BASE_DRIFT = 1 / (20 * 60);

export interface Gaze {
  /** -1 влево, 1 вправо. */
  x: number;
  /** -1 вверх, 1 вниз. */
  y: number;
}

export const CENTRED: Gaze = { x: 0, y: 0 };

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

/** Нужно ли вообще спрашивать. False - датчик или уже открыт, или его нет. */
export function motionNeedsPermission(): boolean {
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

/**
 * Разбудить датчик при первом касании.
 *
 * Спрашивать разрешение можно только из жеста, а единственный жест, который
 * человек тут точно делает, - касание эмблемы. Вызов однократный, ничего не
 * ждёт и ничего не ломает: отказ означает лишь, что на телефоне взгляд
 * останется неподвижным.
 */
export function primeGaze(): void {
  if (asked || !motionNeedsPermission()) return;
  asked = true;
  void requestMotionAccess();
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
  private base: { beta: number; gamma: number } | null = null;
  private frame = 0;
  private fromSensor = false;
  private bound = false;

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
    window.addEventListener("deviceorientation", this.onTilt);
    window.addEventListener("pointermove", this.onPointer, { passive: true });
    document.addEventListener("visibilitychange", this.onVisibility);
  }

  private unbind(): void {
    if (!this.bound) return;
    this.bound = false;
    window.removeEventListener("deviceorientation", this.onTilt);
    window.removeEventListener("pointermove", this.onPointer);
    document.removeEventListener("visibilitychange", this.onVisibility);
    if (this.frame) cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.target = { x: 0, y: 0 };
    this.current = { x: 0, y: 0 };
    this.base = null;
    this.fromSensor = false;
  }

  private onVisibility = (): void => {
    // Вернулись в приложение, возможно в другой позе: базовый угол снимаем
    // заново, чтобы глаз не остался увезённым в сторону.
    if (document.visibilityState === "hidden") {
      this.base = null;
      this.target = { x: 0, y: 0 };
      this.run();
    }
  };

  private onTilt = (e: DeviceOrientationEvent): void => {
    const { beta, gamma } = e;
    if (beta === null || gamma === null) return;
    this.fromSensor = true;

    if (!this.base) {
      this.base = { beta, gamma };
      return;
    }

    this.target = tiltToGaze(this.base, { beta, gamma }, screenAngle());

    // Базовый угол медленно ползёт за текущим: быстрый наклон глаз
    // отрабатывает, смену позы - нет.
    this.base = {
      beta: this.base.beta + (beta - this.base.beta) * BASE_DRIFT,
      gamma: this.base.gamma + (gamma - this.base.gamma) * BASE_DRIFT,
    };

    this.run();
  };

  private onPointer = (e: PointerEvent): void => {
    // Датчик главнее: если он заговорил, мышь больше не вмешивается.
    if (this.fromSensor) return;
    const w = window.innerWidth || 1;
    const h = window.innerHeight || 1;
    this.target = {
      x: clamp1(((e.clientX - w / 2) / (w / 2)) * 1.2),
      y: clamp1(((e.clientY - h / 2) / (h / 2)) * 1.2),
    };
    this.run();
  };

  private run(): void {
    if (this.frame) return;
    const step = (): void => {
      const dx = this.target.x - this.current.x;
      const dy = this.target.y - this.current.y;
      this.current = { x: this.current.x + dx * FOLLOW, y: this.current.y + dy * FOLLOW };
      for (const fn of this.listeners) fn(this.current);
      if (Math.abs(dx) < SETTLED && Math.abs(dy) < SETTLED) {
        this.current = { ...this.target };
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
