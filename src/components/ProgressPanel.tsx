export function ProgressPanel({
  completed,
  total,
  failedCount,
  label,
  currentPath,
  filePercent,
}: {
  completed: number;
  total: number;
  failedCount: number;
  label: string;
  currentPath: string | null;
  /**
   * 現在処理中のファイル内の進捗率。動画は 1 件が数分になるため、
   * 件数のバーだけでは進んでいるか分からない（`D-20`）。
   */
  filePercent?: number | null;
}) {
  return (
    <div className="progress-panel">
      <div className="progress-inline-meta">
        <div className="progress-summary">
          <strong>
            {completed} / {total}
          </strong>
          {failedCount > 0 ? <span className="summary-pill danger">失敗: {failedCount} 件</span> : null}
        </div>
        <span title={currentPath ?? undefined}>{label}</span>
      </div>
      <div className="progress-bar">
        <div
          className="progress-bar-fill"
          style={{
            width: total === 0 ? "0%" : `${Math.round((completed / total) * 100)}%`,
          }}
        />
      </div>
      {filePercent != null ? (
        <div className="progress-bar is-file">
          <div className="progress-bar-fill" style={{ width: `${Math.round(filePercent)}%` }} />
        </div>
      ) : null}
    </div>
  );
}
