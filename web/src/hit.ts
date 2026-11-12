import type { BubbleShape, CellShape, ColumnShape, CvdPointShape, LadderRowShape, Scene } from "./scene";

export type Hit =
  | { kind: "bubble"; shape: BubbleShape }
  | { kind: "cell"; column: ColumnShape; cell: CellShape }
  | { kind: "bar"; column: ColumnShape }
  | { kind: "cvd"; point: CvdPointShape }
  | { kind: "ladder"; row: LadderRowShape };

const BUBBLE_SLOP_PX = 2;

function within(y: number, top: number, height: number): boolean {
  return y >= top && y < top + height;
}

function columnAt(scene: Scene, x: number): ColumnShape | undefined {
  return scene.columns.find((c) => x >= c.x && x < c.x + c.width);
}

export function hitTest(scene: Scene, x: number, y: number): Hit | null {
  const { panes } = scene;
  if (scene.ladder && x >= panes.dom.x && x < panes.dom.x + panes.dom.width && within(y, panes.main.top, panes.main.height)) {
    const row = scene.ladder.rows.find((r) => y >= r.y && y < r.y + r.height);
    return row ? { kind: "ladder", row } : null;
  }
  if (x < 0 || x >= panes.plotWidth) return null;
  if (within(y, panes.main.top, panes.main.height)) {
    for (let i = scene.bubbles.length - 1; i >= 0; i--) {
      const shape = scene.bubbles[i];
      if (shape && Math.hypot(shape.x - x, shape.y - y) <= shape.radius + BUBBLE_SLOP_PX) {
        return { kind: "bubble", shape };
      }
    }
    const column = columnAt(scene, x);
    const cell = column?.cells.find((c) => y >= c.y && y < c.y + c.height);
    return column && cell ? { kind: "cell", column, cell } : null;
  }
  if (within(y, panes.stats.top, panes.stats.height)) {
    const column = columnAt(scene, x);
    return column ? { kind: "bar", column } : null;
  }
  if (scene.cvd && within(y, panes.cvd.top, panes.cvd.height)) {
    let best: CvdPointShape | null = null;
    for (const point of scene.cvd.points) {
      if (Math.abs(point.x - x) <= point.column.width / 2 && (!best || Math.abs(point.x - x) < Math.abs(best.x - x))) {
        best = point;
      }
    }
    return best ? { kind: "cvd", point: best } : null;
  }
  return null;
}
