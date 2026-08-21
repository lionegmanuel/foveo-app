import type { Easing, Style, ZoomKeyframe } from "../lib/commands";

const BACKGROUND_PRESETS = ["gradient-01", "gradient-02", "gradient-03", "gradient-04", "gradient-05", "gradient-06"];
const EASING_OPTIONS: Easing[] = ["linear", "ease-in-out-cubic", "spring"];

interface StylePanelProps {
  style: Style;
  onStyleChange: (style: Style) => void;
  selectedKeyframe: ZoomKeyframe | null;
  onEasingChange: (easing: Easing) => void;
}

export function StylePanel({ style, onStyleChange, selectedKeyframe, onEasingChange }: StylePanelProps) {
  return (
    <div className="flex flex-col gap-3 rounded border border-neutral-700 bg-neutral-900 p-4 text-sm text-neutral-200">
      <h2 className="font-medium">Estilo</h2>

      <label className="flex flex-col gap-1">
        Fondo
        <select
          className="rounded border border-neutral-700 bg-neutral-800 px-2 py-1"
          value={style.background}
          onChange={(event) => onStyleChange({ ...style, background: event.target.value })}
        >
          {BACKGROUND_PRESETS.map((preset) => (
            <option key={preset} value={preset}>
              {preset}
            </option>
          ))}
        </select>
      </label>

      <label className="flex flex-col gap-1">
        Padding ({Math.round(style.padding * 100)}%)
        <input
          type="range"
          min={0}
          max={0.4}
          step={0.01}
          value={style.padding}
          onChange={(event) => onStyleChange({ ...style, padding: Number(event.target.value) })}
        />
      </label>

      <label className="flex flex-col gap-1">
        Esquinas ({style.corner_radius}px)
        <input
          type="range"
          min={0}
          max={80}
          step={1}
          value={style.corner_radius}
          onChange={(event) => onStyleChange({ ...style, corner_radius: Number(event.target.value) })}
        />
      </label>

      <label className="flex items-center gap-2">
        <input
          type="checkbox"
          checked={style.shadow}
          onChange={(event) => onStyleChange({ ...style, shadow: event.target.checked })}
        />
        Sombra
      </label>

      {selectedKeyframe && (
        <label className="flex flex-col gap-1 border-t border-neutral-700 pt-3">
          Easing del keyframe seleccionado
          <select
            className="rounded border border-neutral-700 bg-neutral-800 px-2 py-1"
            value={selectedKeyframe.easing}
            onChange={(event) => onEasingChange(event.target.value as Easing)}
          >
            {EASING_OPTIONS.map((easing) => (
              <option key={easing} value={easing}>
                {easing}
              </option>
            ))}
          </select>
        </label>
      )}
    </div>
  );
}
