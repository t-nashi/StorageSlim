import type { ReactNode } from "react";
import type { SkippedItem } from "../types";

/**
 * 入力一覧 / 結果に共通する表パネルの外枠。
 *
 * 空状態と読込中のクラス付与、見出し行（タイトル・件数・操作）までを持ち、
 * 表本体は children 側で組む。
 */
export function TablePanel({
  title,
  count,
  empty,
  loading = false,
  summary,
  actions,
  children,
}: {
  title: string;
  count: number;
  empty: boolean;
  loading?: boolean;
  summary?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  const classNames = ["subpanel", empty ? "is-empty" : "has-rows"];
  if (loading) {
    classNames.push("is-loading");
  }

  return (
    <section className={classNames.join(" ")}>
      <div className="subpanel-header">
        <div className="title-inline">
          <h3>{title}</h3>
          <span>{count} 件</span>
          {summary}
        </div>
        {actions}
      </div>
      {children}
    </section>
  );
}

/** 表本体のスクロール領域。空状態でも高さが崩れないようクラスを切り替える。 */
export function TableScroll({ empty, children }: { empty: boolean; children: ReactNode }) {
  return <div className={`table-scroll ${empty ? "is-empty" : "has-rows"}`}>{children}</div>;
}

/** 読込中のインジケータ。処理中ではなく入力読込中に出す。 */
export function InlineLoading({ message }: { message: string }) {
  return (
    <div className="inline-loading" role="status" aria-live="polite">
      <span className="loading-dot" />
      <span>{message}</span>
      <div className="loading-track">
        <div className="loading-track-fill" />
      </div>
    </div>
  );
}

/** 読み込めなかった項目の一覧。パスと理由だけで構成されるためモードに依存しない。 */
export function SkippedList({ items }: { items: SkippedItem[] }) {
  if (items.length === 0) {
    return null;
  }
  return (
    <details className="skip-details" open>
      <summary>読み込めなかった項目: {items.length} 件</summary>
      <div className="skip-list">
        {items.map((item) => (
          <div key={`${item.path}-${item.reason}`} className="skip-item">
            <strong>{item.path.split(/[\\/]/).pop()}</strong>
            <small>{item.path}</small>
            <p>{item.reason}</p>
          </div>
        ))}
      </div>
    </details>
  );
}
