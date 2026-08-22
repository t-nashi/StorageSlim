import { ChoiceGroup, type ChoiceOption } from "./ChoiceGroup";
import type { AppMode } from "../types";

const modeOptions: Array<ChoiceOption<AppMode>> = [
  { value: "image", label: "画像圧縮" },
  { value: "video", label: "動画圧縮" },
];

export function AppHeader({
  iconUrl,
  tagline,
  version,
  mode,
  onModeChange,
  switchDisabled,
}: {
  iconUrl: string;
  tagline: string;
  version: string | null;
  mode: AppMode;
  onModeChange: (next: AppMode) => void;
  /** 処理中・読込中はモードを切り替えさせない（`D-17`）。 */
  switchDisabled: boolean;
}) {
  return (
    <section className="app-header panel">
      <div className="app-identity">
        {/* 見出しが製品名を読み上げるので、アイコンは装飾として alt を空にする */}
        <img className="app-logo" src={iconUrl} alt="" width={56} height={56} />
        <div className="app-identity-text">
          <h1>StorageSlim</h1>
          <p className="app-tagline">{tagline}</p>
        </div>
      </div>
      <div className="app-header-aside">
        <ChoiceGroup
          value={mode}
          options={modeOptions}
          onChange={onModeChange}
          disabled={switchDisabled}
        />
        {version ? <span className="app-version">v{version}</span> : null}
      </div>
    </section>
  );
}
