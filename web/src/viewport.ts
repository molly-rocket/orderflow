export interface Layout {
  width: number;
  height: number;
  axisWidth: number;
  timeAxisHeight: number;
  statsHeight: number;
  cvdHeight: number;
  domWidth: number;
}

export interface Band {
  top: number;
  height: number;
}

export interface Panes {
  plotWidth: number;
  main: Band;
  stats: Band;
  cvd: Band;
  timeAxis: Band;
  dom: { x: number; width: number };
}

export interface Camera {
  timeframeUs: number;
  barWidth: number;
  rightEdgeUs: number;
  topRow: number;
  rowHeight: number;
  step: number;
}

export const BAR_WIDTH = { min: 4, max: 420 };
export const ROW_HEIGHT = { min: 2, max: 64 };

export function panes(layout: Layout): Panes {
  const plotWidth = Math.max(0, layout.width - layout.axisWidth - layout.domWidth);
  const timeAxisTop = layout.height - layout.timeAxisHeight;
  const cvdTop = timeAxisTop - layout.cvdHeight;
  const statsTop = cvdTop - layout.statsHeight;
  return {
    plotWidth,
    main: { top: 0, height: Math.max(0, statsTop) },
    stats: { top: statsTop, height: layout.statsHeight },
    cvd: { top: cvdTop, height: layout.cvdHeight },
    timeAxis: { top: timeAxisTop, height: layout.timeAxisHeight },
    dom: { x: plotWidth + layout.axisWidth, width: layout.domWidth },
  };
}

export function xOfTime(cam: Camera, plotWidth: number, us: number): number {
  return plotWidth - ((cam.rightEdgeUs - us) / cam.timeframeUs) * cam.barWidth;
}

export function timeOfX(cam: Camera, plotWidth: number, x: number): number {
  return cam.rightEdgeUs - ((plotWidth - x) / cam.barWidth) * cam.timeframeUs;
}

export function yOfPrice(cam: Camera, price: number): number {
  return (cam.topRow - price / cam.step) * cam.rowHeight;
}

export function priceOfY(cam: Camera, y: number): number {
  return (cam.topRow - y / cam.rowHeight) * cam.step;
}

export function rowOfPrice(cam: Camera, price: number): number {
  return Math.floor(price / cam.step + 1e-9);
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function zoomTime(cam: Camera, plotWidth: number, anchorX: number, factor: number): Camera {
  const anchorUs = timeOfX(cam, plotWidth, anchorX);
  const barWidth = clamp(cam.barWidth * factor, BAR_WIDTH.min, BAR_WIDTH.max);
  const rightEdgeUs = anchorUs + ((plotWidth - anchorX) / barWidth) * cam.timeframeUs;
  return { ...cam, barWidth, rightEdgeUs };
}

export function zoomPrice(cam: Camera, anchorY: number, factor: number): Camera {
  const anchorPrice = priceOfY(cam, anchorY);
  const rowHeight = clamp(cam.rowHeight * factor, ROW_HEIGHT.min, ROW_HEIGHT.max);
  const topRow = anchorPrice / cam.step + anchorY / rowHeight;
  return { ...cam, rowHeight, topRow };
}

export function pan(cam: Camera, dx: number, dy: number): Camera {
  return {
    ...cam,
    rightEdgeUs: cam.rightEdgeUs - (dx / cam.barWidth) * cam.timeframeUs,
    topRow: cam.topRow + dy / cam.rowHeight,
  };
}

export function centerOnPrice(cam: Camera, price: number, height: number): Camera {
  return { ...cam, topRow: price / cam.step + height / cam.rowHeight / 2 };
}
