import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

/**
 * ウィンドウ全体をドロップ対象にする。
 *
 * Tauri の webview イベントからドロップされたパスを受け取り、
 * あわせて DOM 側の drag イベントでハイライト状態を管理する。
 * DOM 側はパスを取得できないため、ハイライトの制御にのみ使う。
 *
 * onPaths は毎レンダーで作り直される想定なので、ref 経由で最新のものを呼ぶ。
 */
export function useDropTarget(onPaths: (paths: string[]) => void | Promise<void>): boolean {
  const [dropActive, setDropActive] = useState(false);
  const onPathsRef = useRef(onPaths);
  const dropDepthRef = useRef(0);

  useEffect(() => {
    onPathsRef.current = onPaths;
  }, [onPaths]);

  useEffect(() => {
    let active = true;
    let unlistenDrop: (() => void) | undefined;

    async function bind() {
      try {
        const webview = getCurrentWebview();
        unlistenDrop = await webview.onDragDropEvent(async (event) => {
          if (event.payload.type === "enter" || event.payload.type === "over") {
            if (active) {
              setDropActive(true);
            }
            return;
          }
          if (event.payload.type === "leave") {
            if (active) {
              setDropActive(false);
            }
            return;
          }
          if (event.payload.type === "drop") {
            if (active) {
              setDropActive(false);
            }
            await onPathsRef.current(event.payload.paths);
          }
        });
      } catch (error) {
        console.warn("StorageSlim: Tauri drag-drop binding is unavailable in this environment.", error);
      }
    }

    void bind();

    const handleDragEnter = (event: DragEvent) => {
      event.preventDefault();
      dropDepthRef.current += 1;
      setDropActive(true);
    };

    const handleDragOver = (event: DragEvent) => {
      event.preventDefault();
      setDropActive(true);
    };

    const handleDragLeave = (event: DragEvent) => {
      event.preventDefault();
      dropDepthRef.current = Math.max(0, dropDepthRef.current - 1);
      if (dropDepthRef.current === 0) {
        setDropActive(false);
      }
    };

    const handleDrop = (event: DragEvent) => {
      event.preventDefault();
      dropDepthRef.current = 0;
      setDropActive(false);
    };

    window.addEventListener("dragenter", handleDragEnter);
    window.addEventListener("dragover", handleDragOver);
    window.addEventListener("dragleave", handleDragLeave);
    window.addEventListener("drop", handleDrop);

    return () => {
      active = false;
      unlistenDrop?.();
      window.removeEventListener("dragenter", handleDragEnter);
      window.removeEventListener("dragover", handleDragOver);
      window.removeEventListener("dragleave", handleDragLeave);
      window.removeEventListener("drop", handleDrop);
    };
  }, []);

  return dropActive;
}
