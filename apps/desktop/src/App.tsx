import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { exportProject, listMonitors, startRecording, stopRecording, type MonitorInfo } from "./lib/commands";

type Status =
  | { kind: "idle" }
  | { kind: "recording" }
  | { kind: "stopped"; projectPath: string }
  | { kind: "exporting"; framesDone: number }
  | { kind: "exported"; outputPath: string }
  | { kind: "error"; message: string };

function App() {
  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);
  const [selectedMonitor, setSelectedMonitor] = useState<number | undefined>(undefined);
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  useEffect(() => {
    listMonitors()
      .then((list) => {
        setMonitors(list);
        setSelectedMonitor((current) => current ?? list[0]?.index);
      })
      .catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
  }, []);

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

  const handleRecordToggle = useCallback(async () => {
    try {
      if (status.kind === "recording") {
        const projectPath = await stopRecording();
        setStatus({ kind: "stopped", projectPath });
      } else {
        await startRecording(selectedMonitor);
        setStatus({ kind: "recording" });
      }
    } catch (err) {
      setStatus({ kind: "error", message: String(err) });
    }
  }, [status.kind, selectedMonitor]);

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

  const isRecording = status.kind === "recording";

  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-6 bg-neutral-950 p-8 text-neutral-100">
      <h1 className="text-xl font-semibold">screenzoom</h1>

      <label className="flex flex-col gap-1 text-sm text-neutral-400">
        Monitor
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
      </label>

      <button
        type="button"
        className="rounded-full bg-red-600 px-6 py-3 font-medium text-white transition hover:bg-red-500 disabled:opacity-50"
        onClick={() => void handleRecordToggle()}
        disabled={status.kind === "exporting"}
      >
        {isRecording ? "Detener grabacion" : "Grabar"}
      </button>

      {status.kind === "stopped" && (
        <button
          type="button"
          className="rounded-full bg-blue-600 px-6 py-3 font-medium text-white transition hover:bg-blue-500"
          onClick={() => void handleExport()}
        >
          Exportar
        </button>
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
