/**
 * 入力先 / 出力先のパス指定行。
 *
 * `load` を渡すと `読込` ボタンが増え、行が 3 列（triple-line）になる。
 * 渡さない場合は 2 列（double-line）。
 */
export function PathPickerField({
  label,
  value,
  placeholder,
  onChange,
  onBrowse,
  onReset,
  load,
}: {
  label: string;
  value: string;
  placeholder: string;
  onChange: (nextValue: string) => void;
  onBrowse: () => void;
  onReset: () => void;
  load?: {
    disabled: boolean;
    onLoad: () => void;
  };
}) {
  return (
    <div className="field">
      <div className="field-inline-head">
        <span>{label}</span>
        <button type="button" className="ghost micro-button" onClick={onReset}>
          既定値へ戻す
        </button>
      </div>
      <div className={`inline-picker ${load ? "triple-line" : "double-line"}`}>
        <input
          value={value}
          placeholder={placeholder}
          onChange={(event) => onChange(event.currentTarget.value)}
        />
        <button type="button" className="ghost" onClick={onBrowse}>
          参照
        </button>
        {load ? (
          <button type="button" className="ghost" disabled={load.disabled} onClick={load.onLoad}>
            読込
          </button>
        ) : null}
      </div>
    </div>
  );
}
