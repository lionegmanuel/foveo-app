#!/usr/bin/env node
// Descarga el binario de ffmpeg (build LGPL-only, sin libx264/libx265 — ver
// ARQUITECTURA.md seccion 3.6 y 7) y lo deja listo como sidecar de Tauri en
// sidecars/ffmpeg/ffmpeg-<target-triple>.exe (convencion de Tauri v2: el
// nombre base declarado en bundle.externalBin + "-<target-triple>.exe",
// ver tauri-utils::resources::external_binaries).
//
// Verifica el SHA256 del zip descargado contra el valor pineado ANTES de
// extraer nada (CLAUDE.md regla 5 / ARQUITECTURA.md seccion 5).
//
// Uso: pnpm setup:ffmpeg
// Override opcional via env (ver .env.example): FFMPEG_DOWNLOAD_URL, FFMPEG_EXPECTED_SHA256.

import { createHash } from "node:crypto";
import { copyFileSync, createWriteStream, existsSync, mkdirSync, readdirSync, rmSync, statSync } from "node:fs";
import { execFileSync } from "node:child_process";
import https from "node:https";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Build estatico LGPL-only de BtbN/FFmpeg-Builds (n8.1, pineado el 2026-08-16
// via GitHub Releases API). Confirmado con `ffmpeg -version`: --disable-libx264
// --disable-libx265, --enable-libopenh264 + NVENC/AMF/QSV/MediaFoundation.
const DEFAULT_URL =
  "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-win64-lgpl-8.1.zip";
const DEFAULT_SHA256 = "fc9d307266bddb972755379eee4a5ce417d1336d3d8a2dd6db6a8864638b9c6c";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sidecarsDir = path.join(repoRoot, "sidecars", "ffmpeg");

function fail(message) {
  console.error(`\n[setup-ffmpeg] ERROR: ${message}\n`);
  process.exit(1);
}

function targetTriple() {
  try {
    const output = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
    const match = output.match(/host: (\S+)/);
    if (match) return match[1];
  } catch {
    // rustc no disponible en PATH; cae al default de abajo.
  }
  return "x86_64-pc-windows-msvc"; // unico target soportado hoy (Windows, ver CLAUDE.md)
}

function downloadWithRedirects(url, destPath, maxRedirects = 5) {
  return new Promise((resolve, reject) => {
    const request = (currentUrl, redirectsLeft) => {
      https
        .get(currentUrl, (res) => {
          if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
            if (redirectsLeft <= 0) return reject(new Error("demasiadas redirecciones"));
            res.resume();
            request(res.headers.location, redirectsLeft - 1);
            return;
          }
          if (res.statusCode !== 200) {
            reject(new Error(`HTTP ${res.statusCode} descargando ${currentUrl}`));
            res.resume();
            return;
          }

          const hash = createHash("sha256");
          const file = createWriteStream(destPath);
          const total = Number(res.headers["content-length"] || 0);
          let downloaded = 0;
          let lastLoggedDecile = -1;

          res.on("data", (chunk) => {
            hash.update(chunk);
            downloaded += chunk.length;
            if (total > 0) {
              const decile = Math.floor((downloaded / total) * 10);
              if (decile !== lastLoggedDecile) {
                lastLoggedDecile = decile;
                console.log(`  descargando... ${decile * 10}%`);
              }
            }
          });
          res.pipe(file);
          file.on("finish", () => {
            file.close(() => resolve(hash.digest("hex")));
          });
          file.on("error", reject);
        })
        .on("error", reject);
    };
    request(url, maxRedirects);
  });
}

function findFileRecursive(dir, filename) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      const found = findFileRecursive(full, filename);
      if (found) return found;
    } else if (entry.name.toLowerCase() === filename.toLowerCase()) {
      return full;
    }
  }
  return null;
}

async function main() {
  if (process.platform !== "win32") {
    fail(
      "este script solo soporta Windows por ahora (ver CLAUDE.md: 'no implementar soporte macOS sin que se pida explicitamente'). Descarga manual: https://ffmpeg.org/download.html",
    );
  }

  const downloadUrl = process.env.FFMPEG_DOWNLOAD_URL || DEFAULT_URL;
  const expectedSha256 = (process.env.FFMPEG_EXPECTED_SHA256 || DEFAULT_SHA256).toLowerCase();
  const triple = targetTriple();
  const finalPath = path.join(sidecarsDir, `ffmpeg-${triple}.exe`);

  if (existsSync(finalPath)) {
    console.log(`[setup-ffmpeg] ya existe ${finalPath}, nada que hacer (borralo a mano para forzar re-descarga).`);
    return;
  }

  mkdirSync(sidecarsDir, { recursive: true });

  const tmpDir = path.join(os.tmpdir(), `screenzoom-ffmpeg-${Date.now()}`);
  mkdirSync(tmpDir, { recursive: true });
  const zipPath = path.join(tmpDir, "ffmpeg.zip");
  const extractDir = path.join(tmpDir, "extracted");
  mkdirSync(extractDir, { recursive: true });

  try {
    console.log(`[setup-ffmpeg] descargando ${downloadUrl}`);
    const actualSha256 = await downloadWithRedirects(downloadUrl, zipPath);

    if (actualSha256 !== expectedSha256) {
      fail(
        `SHA256 no coincide — se descarto el binario, NO se instalo.\n  esperado: ${expectedSha256}\n  obtenido: ${actualSha256}\n` +
          "Esto significa que el binario descargado no es el que se pineo (build distinto, mirror comprometido, o el release cambio de contenido). No continuar sin confirmar la fuente.",
      );
    }
    console.log(`[setup-ffmpeg] SHA256 verificado OK (${actualSha256})`);

    console.log("[setup-ffmpeg] extrayendo ffmpeg.exe...");
    // Usar el bsdtar nativo de Windows (System32) explicito: si el `tar` de
    // PATH resuelve al GNU tar de Git Bash, interpreta "C:\..." como un
    // remote-spec tipo ssh ("host C, ruta \...") y falla con "Cannot connect
    // to C: resolve failed".
    const windowsTar = path.join(process.env.SystemRoot || "C:\\Windows", "System32", "tar.exe");
    execFileSync(existsSync(windowsTar) ? windowsTar : "tar", ["-xf", zipPath, "-C", extractDir], {
      stdio: "inherit",
    });

    const extractedExe = findFileRecursive(extractDir, "ffmpeg.exe");
    if (!extractedExe) fail("no se encontro ffmpeg.exe dentro del zip descargado (estructura inesperada).");

    // copyFileSync en vez de renameSync: el temp dir del OS y el repo suelen
    // estar en discos distintos (EXDEV, "cross-device link not permitted").
    copyFileSync(extractedExe, finalPath);
    const sizeMb = (statSync(finalPath).size / (1024 * 1024)).toFixed(1);
    console.log(`[setup-ffmpeg] listo: ${finalPath} (${sizeMb} MB)`);
  } finally {
    rmSync(tmpDir, { recursive: true, force: true });
  }
}

main().catch((err) => fail(err.stack || String(err)));
