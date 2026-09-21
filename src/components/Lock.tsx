// src/components/Lock.tsx
//
// The padlock from our mark, drawn live so its shackle can open.
//
// WHY IT IS REDRAWN AND NOT CUT OUT. In the original artwork the shackle's
// outline is the same colour as the shield's dark half, and on that side the
// two are a single region — there is no boundary between them to cut along.
// So the lock is authored from the measurements the trace gave us: the body's
// inner area is x 603..1118 / y 1255..1660 and the shackle's band is
// x 668..1049 / y 971..1204 on a 1720x1804 canvas. `emblem-base.svg` is the
// same mark with the old lock painted out, and this draws the new one on top.
//
// The shackle is ONE centre line stroked twice — wide in ink, narrow in sage
// over it — which is how the original reads: a sage band with an outline on
// both sides. Stroked that way it is a single shape, so it turns as one.
//
// Open when the tunnel is down, shut when a byte has crossed. That is the
// state said in a shape, which is the one channel that survives colour
// blindness and "Increase contrast".

// Замерено с оригинала: тёмный в марке тёплый, а не нейтральный.
const INK = "#251f1e";
const SAGE = "#709080";

// Корпус замка по замерам исходной марки (холст 1720x1804). Низ у него - НЕ
// обрезка щитом, как я считал сперва, а собственный шеврон: срез по столбцу
// x=860 показывает, что шалфей кончается на y=1660, а срез по строке y=1620 -
// что он там шириной всего 750..960, вдвое уже щита.
const BODY = {
  /** Тёмный контур: прямоугольник, переходящий в шеврон. */
  ink: { x0: 560, x1: 1160, top: 1208, shoulder: 1570, tip: 1715 },
  /** Шалфейная начинка: то же, отступя на толщину контура (44). */
  sage: { x0: 604, x1: 1116, top: 1255, shoulder: 1548, tip: 1664 },
};

/** Скважина: центр круга и его радиус сняты с оригинала. */
const KEYHOLE = { cx: 860, cy: 1403, r: 64 };

/** Прямоугольник со скруглённым верхом, переходящий книзу в шеврон. */
function bodyPath(b: { x0: number; x1: number; top: number; shoulder: number; tip: number }): string {
  const r = 26;
  const cx = (b.x0 + b.x1) / 2;
  return (
    `M ${b.x0} ${b.top + r} A ${r} ${r} 0 0 1 ${b.x0 + r} ${b.top} ` +
    `L ${b.x1 - r} ${b.top} A ${r} ${r} 0 0 1 ${b.x1} ${b.top + r} ` +
    `L ${b.x1} ${b.shoulder} L ${cx} ${b.tip} L ${b.x0} ${b.shoulder} Z`
  );
}
const SH = { cx: 860, top: 1161, legX: 168, legBottom: 1300, ink: 104, sage: 44 };

/** Base of the left leg: a real shackle turns about the leg that stays in. */
export const PIVOT = { x: SH.cx - SH.legX, y: SH.legBottom - 40 };

const SPINE =
  `M ${SH.cx - SH.legX} ${SH.legBottom} L ${SH.cx - SH.legX} ${SH.top} ` +
  `A ${SH.legX} ${SH.legX} 0 0 1 ${SH.cx + SH.legX} ${SH.top} ` +
  `L ${SH.cx + SH.legX} ${SH.legBottom}`;

export default function Lock({ open }: { open: boolean }) {
  return (
    <svg className="lock-svg" viewBox="0 0 1720 1804" aria-hidden="true" focusable="false">
      <g className="lock-shackle" data-open={open ? "true" : "false"}>
        <path d={SPINE} fill="none" stroke={INK} strokeWidth={SH.ink} />
        <path d={SPINE} fill="none" stroke={SAGE} strokeWidth={SH.sage} />
      </g>

      {/* Корпус поверх ножек: они уходят внутрь него, как у настоящего замка.
          Форма самодостаточна и целиком лежит внутри щита, поэтому никакой
          обрезки ей не нужно - прежняя обрезка и тянула шалфей до края щита. */}
      <g>
        <path d={bodyPath(BODY.ink)} fill={INK} />
        <path d={bodyPath(BODY.sage)} fill={SAGE} />
        {/* Скважина. Не круг с сужающейся ножкой, как я нарисовал сперва, а
            круг с расширением книзу: замер даёт ширину 127 на y=1400, талию
            71 на 1465, снова 87 на 1510 и конец на 1546. */}
        <circle cx={KEYHOLE.cx} cy={KEYHOLE.cy} r={KEYHOLE.r} fill={INK} />
        <path
          d={
            `M ${KEYHOLE.cx - 36} ${KEYHOLE.cy + 57} ` +
            `C ${KEYHOLE.cx - 48} ${KEYHOLE.cy + 100} ${KEYHOLE.cx - 44} ${KEYHOLE.cy + 134} ${KEYHOLE.cx} ${KEYHOLE.cy + 143} ` +
            `C ${KEYHOLE.cx + 44} ${KEYHOLE.cy + 134} ${KEYHOLE.cx + 48} ${KEYHOLE.cy + 100} ${KEYHOLE.cx + 36} ${KEYHOLE.cy + 57} Z`
          }
          fill={INK}
        />
      </g>
    </svg>
  );
}
