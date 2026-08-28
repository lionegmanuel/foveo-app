import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  deleteKeyframe,
  exportProject,
  getProject,
  listMonitors,
  listWindows,
  renderPreviewFrame,
  startRecording,
  stopRecording,
  updateKeyframe,
  updateStyle,
  type Easing,
  type MonitorInfo,
  type Project,
  type Rect,
  type WindowInfo,
  type ZoomKeyframe,
} from "./lib/commands";
import { TimelineOverlay } from "./components/TimelineOverlay";
import { KeyframeTrack } from "./components/KeyframeTrack";
import { StylePanel } from "./components/StylePanel";

type Status =
  | { kind: "idle" }
  | { kind: "recording" }
  | { kind: "stopped"; projectPath: string }
  | { kind: "exporting"; framesDone: number }
  | { kind: "exported"; outputPath: string }
  | { kind: "error"; message: string };

const PREVIEW_WIDTH = 854;
const PREVIEW_DEBOUNCE_MS = 100;

function App() {
  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);
  const [selectedMonitor, setSelectedMonitor] = useState<number | undefined>(undefined);
  const [windows, setWindows] = useState<WindowInfo[]>([]);
  const [sourceKind, setSourceKind] = useState<"monitor" | "window">("monitor");
  const [selectedWindow, setSelectedWindow] = useState<number | undefined>(undefined);
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  const [project, setProject] = useState<Project | null>(null);
  const [selectedKeyframeId, setSelectedKeyframeId] = useState<string | null>(null);
  const [scrubMs, setScrubMs] = useState(0);
  const [previewSrc, setPreviewSrc] = useState<string | null>(null);
  const previewDebounce = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    listMonitors()
      .then((list) => {
        setMonitors(list);
        setSelectedMonitor((current) => current ?? list[0]?.index);
      })
      .catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
  }, []);

  // Las ventanas abiertas cambian mas seguido que los monitores: se
  // re-listan cada vez que el usuario elige capturar una ventana especifica,
  // no solo una vez al montar.
  useEffect(() => {
    if (sourceKind !== "window") {
      return;
    }
    listWindows()
      .then((list) => {
        setWindows(list);
        setSelectedWindow((current) => current ?? list[0]?.index);
      })
      .catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
  }, [sourceKind]);

  useEffect(() => {
    const unlistenProgress = listen<{ frames_done: number }>("export-progress", (event) => {
      setStatus({ kind: "exporting", framesDone: event.payload.frames_done });
    });
    const unlistenFinished = listen<string>("export-finished", (event) => {
      setStatus({ kind: "exported", outputPath: event.payload });
    });
    const unlistenError = listen<{ message: string }>("export-error", (event) => {
      setStatus({ kind: "error", message: event.payload.message });
    });

    return () => {
      void unlistenProgress.then((unlisten) => unlisten());
      void unlistenFinished.then((unlisten) => unlisten());
      void unlistenError.then((unlisten) => unlisten());
    };
  }, []);

  // Carga el proyecto recien grabado apenas hay un projectPath disponible.
  useEffect(() => {
    if (status.kind !== "stopped") {
      return;
    }
    getProject(status.projectPath)
      .then((loaded) => {
        setProject(loaded);
        setSelectedKeyframeId(loaded.zoom_keyframes[0]?.id ?? null);
        setScrubMs(0);
      })
      .catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
  }, [status]);

  // Re-renderiza el preview (debounced) cada vez que cambia el proyecto o el scrubber.
  useEffect(() => {
    if (status.kind !== "stopped" || !project) {
      return;
    }
    if (previewDebounce.current) {
      clearTimeout(previewDebounce.current);
    }
    previewDebounce.current = setTimeout(() => {
      renderPreviewFrame(status.projectPath, scrubMs, PREVIEW_WIDTH)
        .then(setPreviewSrc)
        .catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
    }, PREVIEW_DEBOUNCE_MS);

    return () => {
      if (previewDebounce.current) {
        clearTimeout(previewDebounce.current);
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project, scrubMs]);

  const handleRecordToggle = useCallback(async () => {
    try {
      if (status.kind === "recording") {
        const projectPath = await stopRecording();
        setStatus({ kind: "stopped", projectPath });
      } else {
        const windowIndex = sourceKind === "window" ? selectedWindow : undefined;
        const monitorIndex = sourceKind === "monitor" ? selectedMonitor : undefined;
        await startRecording(monitorIndex, windowIndex);
        setStatus({ kind: "recording" });
      }
    } catch (err) {
      setStatus({ kind: "error", message: String(err) });
    }
  }, [status.kind, sourceKind, selectedMonitor, selectedWindow]);

  const handleExport = useCallback(async () => {
    if (status.kind !== "stopped") {
      return;
    }
    // Sin dialogo de "guardar como" todavia (Fase 1 minima, ver PROMPT_AGENTE_DEV.md):
    // el mp4 final se deja al lado del .szproj.
    const outputPath = status.projectPath.replace(/\.szproj$/, "_export.mp4");
    try {
      await exportProject(status.projectPath, outputPath);
      setStatus({ kind: "exporting", framesDone: 0 });
    } catch (err) {
      setStatus({ kind: "error", message: String(err) });
    }
  }, [status]);

  const selectedKeyframe = project?.zoom_keyframes.find((k) => k.id === selectedKeyframeId) ?? null;

  const persistKeyframe = useCallback(
    (keyframe: ZoomKeyframe) => {
      if (status.kind !== "stopped") {
        return;
      }
      setProject((current) => {
        if (!current) return current;
        const zoom_keyframes = current.zoom_keyframes.map((k) => (k.id === keyframe.id ? keyframe : k));
        return { ...current, zoom_keyframes };
      });
      void updateKeyframe(status.projectPath, keyframe).catch((err: unknown) =>
        setStatus({ kind: "error", message: String(err) }),
      );
    },
    [status],
  );

  const handleRectChange = useCallback(
    (rect: Rect) => {
      if (!selectedKeyframe) return;
      persistKeyframe({ ...selectedKeyframe, target_rect: rect });
    },
    [selectedKeyframe, persistKeyframe],
  );

  const handleEasingChange = useCallback(
    (easing: Easing) => {
      if (!selectedKeyframe) return;
      persistKeyframe({ ...selectedKeyframe, easing });
    },
    [selectedKeyframe, persistKeyframe],
  );

  const handleDeleteKeyframe = useCallback(
    (id: string) => {
      if (status.kind !== "stopped") return;
      setProject((current) =>
        current ? { ...current, zoom_keyframes: current.zoom_keyframes.filter((k) => k.id !== id) } : current,
      );
      if (selectedKeyframeId === id) setSelectedKeyframeId(null);
      void deleteKeyframe(status.projectPath, id).catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
    },
    [status, selectedKeyframeId],
  );

  const handleStyleChange = useCallback(
    (style: Project["style"]) => {
      if (status.kind !== "stopped") return;
      setProject((current) => (current ? { ...current, style } : current));
      void updateStyle(status.projectPath, style).catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
    },
    [status],
  );

  const isRecording = status.kind === "recording";

  return (
    <main className="flex min-h-screen flex-col items-center gap-6 bg-neutral-950 p-8 text-neutral-100">
      <h1 className="text-xl font-semibold">screenzoom</h1>

      <div className="flex flex-col gap-1 text-sm text-neutral-400">
        Fuente
        <div className="flex gap-2">
          <button
            type="button"
            className={`rounded px-3 py-2 ${sourceKind === "monitor" ? "bg-neutral-700 text-white" : "bg-neutral-900 text-neutral-400"}`}
            onClick={() => setSourceKind("monitor")}
            disabled={isRecording}
          >
            Pantalla completa
          </button>
          <button
            type="button"
            className={`rounded px-3 py-2 ${sourceKind === "window" ? "bg-neutral-700 text-white" : "bg-neutral-900 text-neutral-400"}`}
            onClick={() => setSourceKind("window")}
            disabled={isRecording}
          >
            Una ventana
          </button>
        </div>

        {sourceKind === "monitor" ? (
          <select
            className="rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-neutral-100"
            value={selectedMonitor ?? ""}
            onChange={(event) => setSelectedMonitor(Number(event.target.value))}
            disabled={isRecording || monitors.length === 0}
          >
            {monitors.map((monitor) => (
              <option key={monitor.index} value={monitor.index}>
                {monitor.name} ({monitor.width}x{monitor.height})
              </option>
            ))}
          </select>
        ) : (
          <select
            className="rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-neutral-100"
            value={selectedWindow ?? ""}
            onChange={(event) => setSelectedWindow(Number(event.target.value))}
            disabled={isRecording || windows.length === 0}
          >
            {windows.map((win) => (
              <option key={win.index} value={win.index}>
                {win.title}
              </option>
            ))}
          </select>
        )}
      </div>

      <button
        type="button"
        className="rounded-full bg-red-600 px-6 py-3 font-medium text-white transition hover:bg-red-500 disabled:opacity-50"
        onClick={() => void handleRecordToggle()}
        disabled={status.kind === "exporting"}
      >
        {isRecording ? "Detener grabacion" : "Grabar"}
      </button>

      {status.kind === "stopped" && project && (
        <div className="flex w-full max-w-4xl flex-col gap-4">
          <TimelineOverlay
            previewSrc={previewSrc}
            rect={selectedKeyframe?.target_rect ?? { x: 0, y: 0, w: 1, h: 1 }}
            onRectChange={handleRectChange}
          />

          <input
            type="range"
            min={0}
            max={project.raw_take.duration_ms}
            step={16}
            value={scrubMs}
            onChange={(event) => setScrubMs(Number(event.target.value))}
            className="w-full"
          />

          <KeyframeTrack
            keyframes={project.zoom_keyframes}
            durationMs={project.raw_take.duration_ms}
            selectedId={selectedKeyframeId}
            onSelect={setSelectedKeyframeId}
            onChange={persistKeyframe}
            onDelete={handleDeleteKeyframe}
          />

          <StylePanel
            style={project.style}
            onStyleChange={handleStyleChange}
            selectedKeyframe={selectedKeyframe}
            onEasingChange={handleEasingChange}
          />

          <button
            type="button"
            className="rounded-full bg-blue-600 px-6 py-3 font-medium text-white transition hover:bg-blue-500"
            onClick={() => void handleExport()}
          >
            Exportar
          </button>
        </div>
      )}

      {status.kind === "exporting" && (
        <p className="text-sm text-neutral-400">Exportando... {status.framesDone} frames procesados</p>
      )}
      {status.kind === "exported" && <p className="text-sm text-green-400">Listo: {status.outputPath}</p>}
      {status.kind === "error" && <p className="text-sm text-red-400">Error: {status.message}</p>}
    </main>
  );
}

export default App;
