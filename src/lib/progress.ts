import type { BatchProgress } from "../types";

/**
 * 進捗表示の初期状態。アプリ起動直後と「結果をクリア」後で同じ値になるよう、
 * 両方でこれを使う。`state` は未設定にしておくことで待機中の表示に戻る。
 */
export const INITIAL_PROGRESS: BatchProgress = {
  completed: 0,
  total: 0,
  currentPath: null,
};
