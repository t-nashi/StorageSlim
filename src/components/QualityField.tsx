import { useEffect, useState } from "react";
import { roundToStep } from "../lib/format";
import { CustomSlider } from "./CustomSlider";

export function QualityField({
  label,
  value,
  min,
  max,
  step = 1,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (nextValue: number) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(String(value));

  useEffect(() => {
    setDraft(String(value));
  }, [value]);

  function commit(nextRaw: string) {
    const parsed = Number(nextRaw);
    const next = roundToStep(Number.isFinite(parsed) ? parsed : value, min, max, step);
    onChange(next);
    setDraft(String(next));
    setEditing(false);
  }

  return (
    <div className="value-card">
      <div className="value-card-header">
        <span>{label}</span>
        {editing ? (
          <input
            className="value-inline-input"
            type="number"
            min={min}
            max={max}
            step={step}
            value={draft}
            autoFocus
            onChange={(event) => {
              const nextRaw = event.currentTarget.value;
              setDraft(nextRaw);
              if (nextRaw === "") {
                return;
              }
              const parsed = Number(nextRaw);
              if (Number.isFinite(parsed)) {
                onChange(roundToStep(parsed, min, max, step));
              }
            }}
            onBlur={() => commit(draft)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                commit(draft);
              }
              if (event.key === "Escape") {
                setDraft(String(value));
                setEditing(false);
              }
            }}
          />
        ) : (
          <button type="button" className="value-chip" onClick={() => setEditing(true)}>
            {value}
          </button>
        )}
      </div>
      <CustomSlider value={value} min={min} max={max} step={step} onChange={onChange} />
    </div>
  );
}
