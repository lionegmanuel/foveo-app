import { invoke } from "@tauri-apps/api/core";

export interface MonitorInfo {
  index: number;
  name: string;
  width: number;
  height: number;
}

export function listMonitors(): Promise<MonitorInfo[]> {
  return invoke("list_monitors");
}

export function startRecording(monitorIndex: number | undefined): Promise<void> {
  return invoke("start_recording", { monitorIndex });
}

/** Devuelve la ruta del `.szproj` generado. */
export function stopRecording(): Promise<string> {
  return invoke("stop_recording");
}

export function exportProject(projectPath: string, outputPath: string): Promise<void> {
  return invoke("export_project", { projectPath, outputPath });
}
