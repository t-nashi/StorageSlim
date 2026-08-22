import { useEffect, useState } from "react";
import "./App.css";
import { INITIAL_PROGRESS } from "./lib/progress";
import { ImageMode } from "./modes/ImageMode";
import { VideoMode } from "./modes/VideoMode";
import type {
  AppMode,
  BatchProgress,
  InputEntry,
  ProcessResultItem,
  SkippedItem,
  VideoInputEntry,
  VideoResultItem,
} from "./types";

const MODE_KEY = "storageslim.mode.v1";

function loadStoredMode(): AppMode {
  try {
    return window.localStorage.getItem(MODE_KEY) === "video" ? "video" : "image";
  } catch {
    return "image";
  }
}

/**
 * モードの切り替えだけを持つ器。
 *
 * 入力一覧と結果はここで保持する。モードを切り替えると画面側は
 * アンマウントされるため、ここに置かないと切り替えのたびに消えてしまう
 * （`docs/decision-log.md` の `D-17`）。設定はモードごとに別のキーへ
 * 永続化するので、各モードが自分で読み書きする。
 */
function App() {
  const [mode, setMode] = useState<AppMode>(loadStoredMode);

  const [imageEntries, setImageEntries] = useState<InputEntry[]>([]);
  const [imageSkipped, setImageSkipped] = useState<SkippedItem[]>([]);
  const [imageResults, setImageResults] = useState<ProcessResultItem[]>([]);
  const [imageProgress, setImageProgress] = useState<BatchProgress>({ ...INITIAL_PROGRESS });

  const [videoEntries, setVideoEntries] = useState<VideoInputEntry[]>([]);
  const [videoSkipped, setVideoSkipped] = useState<SkippedItem[]>([]);
  const [videoExcludedCount, setVideoExcludedCount] = useState(0);
  const [videoResults, setVideoResults] = useState<VideoResultItem[]>([]);
  const [videoProgress, setVideoProgress] = useState<BatchProgress>({ ...INITIAL_PROGRESS });

  useEffect(() => {
    window.localStorage.setItem(MODE_KEY, mode);
  }, [mode]);

  if (mode === "video") {
    return (
      <VideoMode
        mode={mode}
        onModeChange={setMode}
        entries={videoEntries}
        setEntries={setVideoEntries}
        skipped={videoSkipped}
        setSkipped={setVideoSkipped}
        excludedCount={videoExcludedCount}
        setExcludedCount={setVideoExcludedCount}
        results={videoResults}
        setResults={setVideoResults}
        progress={videoProgress}
        setProgress={setVideoProgress}
      />
    );
  }

  return (
    <ImageMode
      mode={mode}
      onModeChange={setMode}
      entries={imageEntries}
      setEntries={setImageEntries}
      skipped={imageSkipped}
      setSkipped={setImageSkipped}
      results={imageResults}
      setResults={setImageResults}
      progress={imageProgress}
      setProgress={setImageProgress}
    />
  );
}

export default App;
