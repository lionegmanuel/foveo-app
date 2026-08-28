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

export interface WindowInfo {
  index: number;
  title: string;
}

export function listWindows(): Promise<WindowInfo[]> {
  return invoke("list_windows");
}

/** `windowIndex`, si esta presente, gana sobre `monitorIndex` (ver el comando `start_recording`). */
export function startRecording(monitorIndex: number | undefined, windowIndex: number | undefined): Promise<void> {
  return invoke("start_recording", { monitorIndex, windowIndex });
}

/** Devuelve la ruta del `.szproj` generado. */
export function stopRecording(): Promise<string> {
  return invoke("stop_recording");
}

export function exportProject(projectPath: string, outputPath: string): Promise<void> {
  return invoke("export_project", { projectPath, outputPath });
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type Easing = "linear" | "ease-in-out-cubic" | "spring";
export type KeyframeSource = "auto" | "manual";

export interface ZoomKeyframe {
  id: string;
  start_ms: number;
  duration_ms: number;
  target_rect: Rect;
  easing: Easing;
  source: KeyframeSource;
}

export interface Style {
  background: string;
  padding: number;
  corner_radius: number;
  shadow: boolean;
  cursor_smoothing: boolean;
  motion_blur: boolean;
}

export interface Resolution {
  width: number;
  height: number;
}

export interface RawTake {
  path: string;
  fps: number;
  resolution: Resolution;
  duration_ms: number;
}

export interface InputLog {
  path: string;
}

export type ExportResolution = "1080p" | "1440p" | "4k";
export type Codec = "h264" | "h265";

export interface ExportSettings {
  resolution: ExportResolution;
  fps: number;
  codec: Codec;
  hw_accel: string;
}

export interface Project {
  version: number;
  raw_take: RawTake;
  input_log: InputLog;
  zoom_keyframes: ZoomKeyframe[];
  style: Style;
  export_settings: ExportSettings;
}

export function getProject(projectPath: string): Promise<Project> {
  return invoke("get_project", { projectPath });
}

/** Devuelve un PNG en base64 (sin el prefijo `data:image/png;base64,`). */
export function renderPreviewFrame(projectPath: string, tMs: number, maxWidth: number): Promise<string> {
  return invoke("render_preview_frame", { projectPath, tMs, maxWidth });
}

export function updateKeyframe(projectPath: string, keyframe: ZoomKeyframe): Promise<void> {
  return invoke("update_keyframe", { projectPath, keyframe });
}

export function addKeyframe(projectPath: string, keyframe: ZoomKeyframe): Promise<void> {
  return invoke("add_keyframe", { projectPath, keyframe });
}

export function deleteKeyframe(projectPath: string, keyframeId: string): Promise<void> {
  return invoke("delete_keyframe", { projectPath, keyframeId });
}

export function updateStyle(projectPath: string, style: Style): Promise<void> {
  return invoke("update_style", { projectPath, style });
}
