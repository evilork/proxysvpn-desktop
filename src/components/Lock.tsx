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

/** Shield silhouette shrunk by the width of its dark rim: the body is clipped
    to this so it never paints over the rim. */
const SHIELD_INNER = "M 836 1.499 C 830.511 4.634, 820.004 7.270, 786.500 13.917 C 767.250 17.737, 741.150 23.356, 728.500 26.405 C 715.850 29.454, 696.500 33.546, 685.500 35.499 C 658.738 40.250, 636.921 45.061, 614.811 51.088 C 584.754 59.280, 574.681 61.653, 549.500 66.473 C 510.661 73.908, 474.041 81.737, 450.500 87.640 C 438.400 90.674, 414.100 96.060, 396.500 99.610 C 378.900 103.160, 354.825 108.501, 343 111.479 C 315.956 118.291, 282.761 125.663, 255 131.023 C 217.284 138.305, 170.169 148.841, 148 154.951 C 122.510 161.976, 111.038 164.562, 81.028 170.043 C 68.669 172.301, 49.994 176.110, 39.528 178.508 C 29.063 180.906, 16.900 183.378, 12.500 184.002 C 8.100 184.626, 3.488 185.339, 2.250 185.587 C -0.631 186.165, -1.412 838.136, 1.451 852.977 C 2.249 857.115, 3.351 867.025, 3.900 875 C 12.599 1001.363, 73.911 1162.022, 164.757 1296.500 C 215.984 1372.331, 300.748 1467.659, 364.863 1521.545 C 376.613 1531.420, 392.588 1544.832, 400.363 1551.348 C 415.861 1564.337, 488.138 1619.035, 502.500 1628.644 C 529.205 1646.510, 572.583 1673.031, 611 1694.980 C 647.160 1715.638, 651.841 1718.126, 676 1729.527 C 687.275 1734.848, 709.775 1745.638, 726 1753.506 C 745.294 1762.861, 768.273 1772.906, 792.418 1782.537 C 812.724 1790.636, 831.849 1798.778, 834.918 1800.629 C 843.829 1806.002, 877.464 1805.831, 886.500 1800.366 C 889.800 1798.371, 899.925 1793.925, 909 1790.486 C 957.286 1772.189, 992.168 1756.090, 1065.500 1718.259 C 1236.021 1630.289, 1367.290 1527.084, 1488.110 1386 C 1545.672 1318.784, 1603.549 1228.453, 1640.132 1148.736 C 1678.608 1064.895, 1705.506 973.275, 1716.399 888.959 C 1717.220 882.612, 1718.478 876.087, 1719.195 874.459 C 1720.258 872.049, 1720.553 807.907, 1720.785 528.435 L 1721.071 185.370 1711.785 184.150 C 1698.733 182.435, 1678.349 178.507, 1658.848 173.948 C 1649.690 171.808, 1633.490 168.270, 1622.848 166.088 C 1612.207 163.906, 1593.375 159.603, 1581 156.527 C 1553.455 149.680, 1514.468 141.050, 1495 137.491 C 1475.188 133.870, 1396.963 116.466, 1367 109.014 C 1336.587 101.449, 1197.250 70.466, 1171.500 65.542 C 1161.050 63.543, 1143.275 59.485, 1132 56.523 C 1112.143 51.306, 1073.020 42.411, 1050.500 37.993 C 1044.450 36.806, 1032.525 34.103, 1024 31.987 C 1015.475 29.870, 997.700 26.050, 984.500 23.498 C 971.300 20.945, 953.525 17.347, 945 15.502 C 936.475 13.657, 921.400 10.735, 911.500 9.008 C 897.999 6.654, 892.023 5.136, 887.590 2.935 C 880.606 -0.533, 841.465 -1.623, 836 1.499 M 0.494 517 C 0.494 699.325, 0.609 773.913, 0.750 682.750 C 0.891 591.588, 0.891 442.413, 0.750 351.250 C 0.609 260.088, 0.494 334.675, 0.494 517";

const BODY = { x: 573, y: 1225, w: 575, h: 465, r: 26, edge: 30 };
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
      <defs>
        <clipPath id="lock-inner">
          <path fillRule="evenodd" d={SHIELD_INNER} />
        </clipPath>
      </defs>

      <g className="lock-shackle" data-open={open ? "true" : "false"}>
        <path d={SPINE} fill="none" stroke={INK} strokeWidth={SH.ink} />
        <path d={SPINE} fill="none" stroke={SAGE} strokeWidth={SH.sage} />
      </g>

      {/* The body sits over the legs, so they disappear into it. */}
      <g clipPath="url(#lock-inner)">
        <rect x={BODY.x} y={BODY.y} width={BODY.w} height={BODY.h} rx={BODY.r} fill={INK} />
        <rect
          x={BODY.x + BODY.edge}
          y={BODY.y + BODY.edge}
          width={BODY.w - BODY.edge * 2}
          height={BODY.h - BODY.edge * 2}
          rx={10}
          fill={SAGE}
        />
        <circle cx={BODY.x + BODY.w / 2} cy={1420} r={52} fill={INK} />
        <path
          d={`M ${BODY.x + BODY.w / 2 - 26} 1460 L ${BODY.x + BODY.w / 2 + 26} 1460 L ${BODY.x + BODY.w / 2 + 16} 1552 L ${BODY.x + BODY.w / 2 - 16} 1552 Z`}
          fill={INK}
        />
      </g>
    </svg>
  );
}
