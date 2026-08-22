export function ProgressPanel({
  completed,
  total,
  failedCount,
  label,
  currentPath,
}: {
  completed: number;
  total: number;
  failedCount: number;
  label: string;
  currentPath: string | null;
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
    </div>
  );
}
