import { type ChildProcess, spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test as base, expect } from "@playwright/test";

/** Caminho do scriba.exe de debug; SCRIBA_EXE sobrescreve. */
function exe(): string {
  if (process.env.SCRIBA_EXE) return process.env.SCRIBA_EXE;
  const target = process.env.CARGO_TARGET_DIR ?? join(__dirname, "..", "target");
  return join(target, "debug", "scriba.exe");
}

/** Espera o scriba anunciar a URL de entrada (com o token) no stdout. */
function announced(child: ChildProcess): Promise<string> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("o scriba não anunciou a URL em 30 s")), 30_000);
    let out = "";
    child.stdout!.on("data", (chunk: Buffer) => {
      out += chunk.toString();
      const m = out.match(/Scriba em (http\S+)/);
      if (m) {
        clearTimeout(timer);
        resolve(m[1]);
      }
    });
    child.on("error", reject);
    child.on("exit", (code) => reject(new Error(`o scriba saiu com código ${code}`)));
  });
}

export type Scriba = {
  /** URL de entrada, com o token. */
  url: string;
  origin: string;
};

/**
 * Cada teste ganha o próprio scriba.exe (porta 0, diretório temporário): nenhum estado
 * (tema, jobs) vaza de um teste para outro.
 */
export const test = base.extend<{ scriba: Scriba }>({
  scriba: async ({}, use) => {
    const dataDir = mkdtempSync(join(tmpdir(), "scriba-e2e-"));
    const child = spawn(exe(), ["--no-open", "--port", "0", "--data-dir", dataDir, "--output-dir", join(dataDir, "saidas")], {
      stdio: ["ignore", "pipe", "inherit"],
    });
    try {
      const url = await announced(child);
      await use({ url, origin: new URL(url).origin });
    } finally {
      if (child.exitCode === null) {
        const exited = once(child, "exit");
        child.kill();
        await exited;
      }
      rmSync(dataDir, { recursive: true, force: true, maxRetries: 5 });
    }
  },
});

export { expect };
