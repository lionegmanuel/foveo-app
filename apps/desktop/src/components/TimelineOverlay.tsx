import { useCallback, useRef, useState } from "react";
import type { Rect } from "../lib/commands";

interface TimelineOverlayProps {
  previewSrc: string | null;
  rect: Rect;
  onRectChange: (rect: Rect) => void;
}

type DragMode = "move" | "nw" | "ne" | "sw" | "se" | null;

const MIN_SIZE = 0.05;

export function TimelineOverlay({ previewSrc, rect, onRectChange }: TimelineOverlayProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [dragMode, setDragMode] = useState<DragMode>(null);
  const dragStart = useRef<{ pointerX: number; pointerY: number; rect: Rect } | null>(null);

  const handlePointerDown = useCallback(
    (mode: DragMode) => (event: React.PointerEvent) => {
      event.stopPropagation();
      (event.currentTarget as Element).setPointerCapture(event.pointerId);
      setDragMode(mode);
      dragStart.current = { pointerX: event.clientX, pointerY: event.clientY, rect };
    },
    [rect],
  );

  const handlePointerMove = useCallback(
    (event: React.PointerEvent) => {
      if (!dragMode || !dragStart.current || !containerRef.current) {
        return;
      }
      const bounds = containerRef.current.getBoundingClientRect();
      const dx = (event.clientX - dragStart.current.pointerX) / bounds.width;
      const dy = (event.clientY - dragStart.current.pointerY) / bounds.height;
      const start = dragStart.current.rect;

      let next: Rect = start;
      if (dragMode === "move") {
        next = { ...start, x: start.x + dx, y: start.y + dy };
      } else if (dragMode === "se") {
        next = { ...start, w: start.w + dx, h: start.h + dy };
      } else if (dragMode === "nw") {
        next = { x: start.x + dx, y: start.y + dy, w: start.w - dx, h: start.h - dy };
      } else if (dragMode === "ne") {
        next = { ...start, y: start.y + dy, w: start.w + dx, h: start.h - dy };
      } else if (dragMode === "sw") {
        next = { ...start, x: start.x + dx, w: start.w - dx, h: start.h + dy };
      }

      const w = Math.min(Math.max(next.w, MIN_SIZE), 1);
      const h = Math.min(Math.max(next.h, MIN_SIZE), 1);
      const x = Math.min(Math.max(next.x, 0), 1 - w);
      const y = Math.min(Math.max(next.y, 0), 1 - h);
      onRectChange({ x, y, w, h });
    },
    [dragMode, onRectChange],
  );

  const handlePointerUp = useCallback(() => {
    setDragMode(null);
    dragStart.current = null;
  }, []);

  const handleClasses: Record<"nw" | "ne" | "sw" | "se", string> = {
    nw: "-left-1.5 -top-1.5 cursor-nwse-resize",
    se: "-bottom-1.5 -right-1.5 cursor-nwse-resize",
    ne: "-right-1.5 -top-1.5 cursor-nesw-resize",
    sw: "-bottom-1.5 -left-1.5 cursor-nesw-resize",
  };

  return (
    <div ref={containerRef} className="relative aspect-video w-full select-none overflow-hidden rounded bg-black">
      {previewSrc && (
        <img
          src={`data:image/png;base64,${previewSrc}`}
          alt="preview"
          className="pointer-events-none h-full w-full object-contain"
        />
      )}
      <div
        className="absolute cursor-move border-2 border-blue-500 bg-blue-500/10"
        style={{ left: `${rect.x * 100}%`, top: `${rect.y * 100}%`, width: `${rect.w * 100}%`, height: `${rect.h * 100}%` }}
        onPointerDown={handlePointerDown("move")}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
      >
        {(Object.keys(handleClasses) as Array<keyof typeof handleClasses>).map((corner) => (
          <div
            key={corner}
            className={`absolute h-3 w-3 rounded-full border border-blue-500 bg-white ${handleClasses[corner]}`}
            onPointerDown={handlePointerDown(corner)}
            onPointerMove={handlePointerMove}
            onPointerUp={handlePointerUp}
          />
        ))}
      </div>
    </div>
  );
}
