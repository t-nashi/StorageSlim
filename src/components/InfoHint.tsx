import { useId, useRef, useState } from "react";

/**
 * 項目ラベルの横に置く補足アイコン。
 *
 * 補足文を項目の下に並べると、設定パネルの列幅が注記の長さで決まってしまい、
 * 幅が足りない環境で他の項目へ重なる。読みたいときだけ出す形にして、
 * レイアウトは操作部の幅だけで決まるようにする。
 */
export function InfoHint({ text, label }: { text: string; label: string }) {
  const id = useId();
  const [hovered, setHovered] = useState(false);
  // タップでも読めるように、クリックは開いたままにする。
  const [pinned, setPinned] = useState(false);
  // 右端の項目では、そのまま出すとパネルの外へ出るので左へ寄せる。
  const [alignEnd, setAlignEnd] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);

  const BUBBLE_WIDTH = 280;
  const updateAlignment = () => {
    const rect = ref.current?.getBoundingClientRect();
    if (rect) {
      setAlignEnd(rect.left + BUBBLE_WIDTH > window.innerWidth - 24);
    }
  };

  const show = hovered || pinned;

  return (
    <span className="info-hint" ref={ref}>
      <button
        type="button"
        className={`info-hint-button ${show ? "is-open" : ""}`}
        aria-label={`${label}の補足`}
        aria-describedby={show ? id : undefined}
        aria-expanded={pinned}
        onMouseEnter={() => {
          updateAlignment();
          setHovered(true);
        }}
        onMouseLeave={() => setHovered(false)}
        onFocus={() => {
          updateAlignment();
          setHovered(true);
        }}
        onBlur={() => {
          setHovered(false);
          setPinned(false);
        }}
        onClick={() => {
          updateAlignment();
          setPinned((current) => !current);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            setPinned(false);
            setHovered(false);
          }
        }}
      >
        i
      </button>
      {show ? (
        <span id={id} role="tooltip" className={`info-hint-bubble ${alignEnd ? "is-align-end" : ""}`}>
          {text}
        </span>
      ) : null}
    </span>
  );
}
