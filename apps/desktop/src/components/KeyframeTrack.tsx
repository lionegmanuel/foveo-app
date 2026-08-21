import { useCallback, useRef } from "react";
import type { ZoomKeyframe } from "../lib/commands";

interface KeyframeTrackProps {
  keyframes: ZoomKeyframe[];
  durationMs: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onChange: (keyframe: ZoomKeyframe) => void;
  onDelete: (id: string) => void;
}

export function KeyframeTrack({ keyframes, durationMs, selectedId, onSelect, onChange, onDelete }: KeyframeTrackProps) {
  const trackRef = useRef<HTMLDivElement>(null);

  const handleDrag = useCallback(
    (keyframe: ZoomKeyframe, mode: "move" | "resize") => (event: React.PointerEvent) => {
      event.stopPropagation();
      const track = trackRef.current;
      if (!track || durationMs <= 0) {
        return;
      }
      const bounds = track.getBoundingClientRect();
      const msPerPixel = durationMs / Math.max(bounds.width, 1);
      const startClientX = event.clientX;
      const startMs = keyframe.start_ms;
      const startDuration = keyframe.duration_ms;

      const onMove = (moveEvent: PointerEvent) => {
        const deltaMs = (moveEvent.clientX - startClientX) * msPerPixel;
        if (mode === "move") {
          const nextStart = Math.max(0, Math.min(startMs + deltaMs, durationMs - startDuration));
          onChange({ ...keyframe, start_ms: Math.round(nextStart) });
        } else {
          const nextDuration = Math.max(100, Math.min(startDuration + deltaMs, durationMs - startMs));
          onChange({ ...keyframe, duration_ms: Math.round(nextDuration) });
        }
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [durationMs, onChange],
  );

  return (
    <div ref={trackRef} className="relative h-12 w-full rounded bg-neutral-900">
      {keyframes.map((keyframe) => (
        <div
          key={keyframe.id}
          className={`absolute top-1 h-10 cursor-move rounded border px-1 text-xs text-white ${
            keyframe.id === selectedId ? "border-blue-400 bg-blue-600/70" : "border-neutral-600 bg-neutral-700/70"
          }`}
          style={{
            left: `${(keyframe.start_ms / durationMs) * 100}%`,
            width: `${(keyframe.duration_ms / durationMs) * 100}%`,
          }}
          onClick={() => onSelect(keyframe.id)}
          onPointerDown={handleDrag(keyframe, "move")}
          onDoubleClick={() => onDelete(keyframe.id)}
          title="Arrastrar para mover, doble click para borrar"
        >
          <div className="absolute right-0 top-0 h-full w-2 cursor-ew-resize" onPointerDown={handleDrag(keyframe, "resize")} />
        </div>
      ))}
    </div>
  );
}
