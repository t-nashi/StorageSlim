import { useEffect, useRef } from "react";
import { clamp, roundToStep } from "../lib/format";

export function CustomSlider({
  value,
  min,
  max,
  step = 1,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (nextValue: number) => void;
}) {
  const trackRef = useRef<HTMLDivElement | null>(null);
  const draggingRef = useRef(false);

  useEffect(() => {
    function updateFromClientX(clientX: number) {
      const track = trackRef.current;
      if (!track) {
        return;
      }
      const rect = track.getBoundingClientRect();
      const ratio = clamp((clientX - rect.left) / rect.width, 0, 1);
      const nextValue = roundToStep(min + ratio * (max - min), min, max, step);
      onChange(nextValue);
    }

    function handleMove(event: MouseEvent) {
      if (!draggingRef.current) {
        return;
      }
      updateFromClientX(event.clientX);
    }

    function handleUp() {
      draggingRef.current = false;
    }

    window.addEventListener("mousemove", handleMove);
    window.addEventListener("mouseup", handleUp);
    return () => {
      window.removeEventListener("mousemove", handleMove);
      window.removeEventListener("mouseup", handleUp);
    };
  }, [max, min, onChange, step]);

  const ratio = ((value - min) / (max - min)) * 100;

  return (
    <div
      ref={trackRef}
      className="custom-slider"
      onMouseDown={(event) => {
        draggingRef.current = true;
        const rect = trackRef.current?.getBoundingClientRect();
        if (!rect) {
          return;
        }
        const nextValue = roundToStep(min + ((event.clientX - rect.left) / rect.width) * (max - min), min, max, step);
        onChange(nextValue);
      }}
    >
      <div className="custom-slider-fill" style={{ width: `${ratio}%` }} />
      <div className="custom-slider-thumb" style={{ left: `${ratio}%` }} />
    </div>
  );
}
