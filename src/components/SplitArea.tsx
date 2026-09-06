import { useCallback, useRef, useState, type CSSProperties, type ReactNode } from "react";

/** 左右カラムの間隔。CSS の gap と同じ値を使う。 */
const GAP_PX = 14;
/** 中央位置。起動時はここに戻す。 */
const CENTER = 0.5;
/** どちらかのカラムが潰れないようにする可動域。 */
const MIN_RATIO = 0.28;
const MAX_RATIO = 0.72;
/** 中央付近で吸着させる幅。ドラッグ中はこの範囲に入ると中央へ寄る。 */
const SNAP_RANGE = 0.02;
/** キーボード操作 1 回あたりの移動量。 */
const KEY_STEP = 0.02;

function clamp(value: number): number {
  return Math.min(MAX_RATIO, Math.max(MIN_RATIO, value));
}

/**
 * 入力側 / 出力側の 2 カラムを包み、その境目をドラッグで動かせるようにする領域。
 *
 * 比率は state に持つだけで保存しない。アプリを起動し直すと中央へ戻る。
 * 中央付近ではスナップし、境目が中央にあることを見た目でも分かるようにする。
 */
export function SplitArea({ children }: { children: ReactNode }) {
  const areaRef = useRef<HTMLDivElement>(null);
  const [ratio, setRatio] = useState(CENTER);
  const [dragging, setDragging] = useState(false);

  const snapped = ratio === CENTER;

  /** ポインタの x 座標から比率を求める。境目はカラム間の余白の中心にある。 */
  const ratioFromClientX = useCallback((clientX: number) => {
    const area = areaRef.current;
    if (!area) {
      return null;
    }
    const rect = area.getBoundingClientRect();
    const track = rect.width - GAP_PX;
    if (track <= 0) {
      return null;
    }
    const raw = (clientX - rect.left - GAP_PX / 2) / track;
    return Math.abs(raw - CENTER) < SNAP_RANGE ? CENTER : clamp(raw);
  }, []);

  function handlePointerDown(event: React.PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) {
      return;
    }
    event.currentTarget.setPointerCapture(event.pointerId);
    setDragging(true);
  }

  function handlePointerMove(event: React.PointerEvent<HTMLDivElement>) {
    if (!dragging) {
      return;
    }
    const next = ratioFromClientX(event.clientX);
    if (next != null) {
      setRatio(next);
    }
  }

  function endDrag(event: React.PointerEvent<HTMLDivElement>) {
    if (!dragging) {
      return;
    }
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    setDragging(false);
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      const delta = event.key === "ArrowLeft" ? -KEY_STEP : KEY_STEP;
      setRatio((current) => {
        const next = clamp(current + delta);
        return Math.abs(next - CENTER) < SNAP_RANGE / 2 ? CENTER : next;
      });
      return;
    }
    if (event.key === "Home" || event.key === "Escape") {
      event.preventDefault();
      setRatio(CENTER);
    }
  }

  const style = { "--split-ratio": ratio } as CSSProperties;
  const className = ["split-area", dragging ? "is-dragging" : "", snapped ? "is-snapped" : ""]
    .filter(Boolean)
    .join(" ");

  return (
    <div ref={areaRef} className={className} style={style}>
      {children}
      <div
        className="split-handle"
        role="separator"
        aria-orientation="vertical"
        aria-label="入力側と出力側の幅を変更"
        aria-valuenow={Math.round(ratio * 100)}
        aria-valuemin={Math.round(MIN_RATIO * 100)}
        aria-valuemax={Math.round(MAX_RATIO * 100)}
        tabIndex={0}
        title="ドラッグで幅を変更 / ダブルクリックで中央へ戻す"
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onDoubleClick={() => setRatio(CENTER)}
        onKeyDown={handleKeyDown}
      >
        <span className="split-handle-grip" aria-hidden="true" />
      </div>
    </div>
  );
}
