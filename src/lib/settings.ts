import type { BatchSettings, ResizeMode, ResizeSettings, ResizeUnit } from "../types";
import type { ChoiceOption } from "../components/ChoiceGroup";

export const resizeModeOptions: Array<ChoiceOption<ResizeMode>> = [
  { value: "none", label: "変更なし" },
  { value: "width", label: "幅" },
  { value: "height", label: "高さ" },
  { value: "longEdge", label: "長辺" },
];

export const resizeUnitOptions: Array<ChoiceOption<ResizeUnit>> = [
  { value: "px", label: "px" },
  { value: "percent", label: "%" },
];

/**
 * 画像モードのメタデータ選択肢。
 *
 * 「撮影日のみ」は、クラウドの写真サービスへ上げ直しても撮影日で並ぶようにしつつ、
 * GPS を残さないための選択肢。既定にはせず、明示的に選ばせる。
 */
export const imageMetadataOptions: Array<ChoiceOption<BatchSettings["metadataMode"]>> = [
  { value: "strip", label: "削除する" },
  { value: "dateOnly", label: "撮影日のみ" },
  { value: "keep", label: "保持する" },
];

/** 動画モードのメタデータ選択肢。撮影日のみの絞り込みは画像だけの機能。 */
export const videoMetadataOptions: Array<ChoiceOption<BatchSettings["metadataMode"]>> = [
  { value: "strip", label: "削除する" },
  { value: "keep", label: "保持する" },
];

/** リサイズ値の上限。% 指定では拡大しないので 100 まで。 */
export function resizeValueMaxFor(unit: ResizeUnit): number {
  return unit === "percent" ? 100 : 100000;
}

/**
 * リサイズ基準を選んだのに値が入っていない状態。
 * 実行不可の条件と入力欄の必須表示の両方で使う。
 */
export function isResizeValueMissing(resize: ResizeSettings): boolean {
  if (resize.mode === "none") {
    return false;
  }
  return resize.value == null || resize.value <= 0;
}
