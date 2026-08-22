export type ChoiceOption<T extends string> = {
  value: T;
  label: string;
  disabled?: boolean;
  title?: string;
};

export function ChoiceGroup<T extends string>({
  value,
  options,
  onChange,
  disabled = false,
}: {
  value: T;
  options: Array<ChoiceOption<T>>;
  onChange: (nextValue: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className={`choice-group ${disabled ? "is-disabled" : ""}`}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={`choice-chip ${value === option.value ? "active" : ""}`}
          aria-pressed={value === option.value}
          disabled={disabled || option.disabled}
          title={option.title}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
