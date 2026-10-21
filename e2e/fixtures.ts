import { type ChildProcess, spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test as base, expect } from "@playwright/test";

/** Caminho do scriba.exe de debug; SCRIBA_EXE sobrescreve. */
function exe(): string {
  if (process.env.SCRIBA_EXE) return process.env.SCRIBA_EXE;
  const target = process.env.CARGO_TARGET_DIR ?? join(__dirname, "..", "target");
  return join(target, "debug", "scriba.exe");
}

/** Junta o stdout do processo. */
function collect(child: ChildProcess): () => string {
  let out = "";
  child.stdout!.on("data", (chunk: Buffer) => {
    out += chunk.toString();
  });
  return () => out;
}

/** Espera o scriba anunciar a URL de entrada (com o token) no stdout. */
function announced(child: ChildProcess, stdout: () => string): Promise<string> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("o scriba não anunciou a URL em 30 s")), 30_000);
    child.stdout!.on("data", () => {
      const m = stdout().match(/Scriba em (http\S+)/);
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
  /** Pasta das saídas (`--output-dir`). */
  outputDir: string;
  /** As pastas que o "Abrir pasta" pediu, na ordem (o pipeline falso só as imprime). */
  opened: () => string[];
  /** Os arquivos que o "Mostrar no Explorer" pediu, na ordem. */
  revealed: () => string[];
  /** O que o próximo "Salvar como" responde: um caminho, ou `null` para cancelar. */
  saveAs: (path: string | null) => void;
  /** Os "Salvar como" abertos, na ordem: a pasta inicial e o nome sugerido. */
  asked: () => { dir: string; name: string }[];
  /** Os arquivos temporários em `uploads/`. */
  uploads: () => string[];
  /** O roteiro do áudio falso para a próxima gravação (`perder-mic`, `sem-microfone`…). */
  audioScript: (text: string) => void;
  /**
   * Mata o exe (TerminateProcess, como um crash) e abre outro na mesma pasta de dados.
   * `url`, `origin` e o stdout passam a ser os do novo.
   */
  restart: () => Promise<void>;
};

/** Mata o processo e espera ele sair. */
async function kill(child: ChildProcess) {
  if (child.exitCode === null) {
    const exited = once(child, "exit");
    child.kill();
    await exited;
  }
}

/**
 * Cada teste ganha o próprio scriba.exe (porta 0, diretório temporário): nenhum estado
 * (tema, jobs) vaza de um teste para outro. O exe roda o pipeline falso (`--fake-pipeline`,
 * build com `--features test-support`), que não lê o arquivo enviado, o áudio falso
 * (`--fake-audio`: tons no lugar do microfone e do som do PC), e o "Salvar como" responde o
 * que o teste mandar (`saveAs`), sem abrir janela.
 */
export const test = base.extend<{ scriba: Scriba }>({
  scriba: async ({}, use) => {
    const dataDir = mkdtempSync(join(tmpdir(), "scriba-e2e-"));
    const outputDir = join(dataDir, "saidas");
    const launch = () => {
      const child = spawn(
        exe(),
        ["--no-open", "--no-tray", "--fake-pipeline", "--fake-audio", "--port", "0", "--data-dir", dataDir, "--output-dir", outputDir],
        { stdio: ["ignore", "pipe", "inherit"] },
      );
      return { child, stdout: collect(child) };
    };
    let { child, stdout } = launch();
    const lines = (prefix: string) => [...stdout().matchAll(new RegExp(`^${prefix}: (.+?)\\r?$`, "gm"))].map((m) => m[1]);
    const opened = () => lines("abrir pasta");
    const revealed = () => lines("mostrar arquivo");
    const asked = () =>
      lines("salvar como").map((line) => {
        const [dir, name] = line.split("|");
        return { dir, name };
      });
    const saveAs = (path: string | null) => writeFileSync(join(dataDir, "salvar-como.txt"), path ?? "");
    const uploads = () => readdirSync(join(dataDir, "uploads"));
    const audioScript = (text: string) => writeFileSync(join(dataDir, "audio-falso.txt"), text);
    try {
      const url = await announced(child, stdout);
      const scriba: Scriba = {
        url,
        origin: new URL(url).origin,
        outputDir,
        opened,
        revealed,
        saveAs,
        asked,
        uploads,
        audioScript,
        restart: async () => {
          await kill(child);
          ({ child, stdout } = launch());
          scriba.url = await announced(child, stdout);
          scriba.origin = new URL(scriba.url).origin;
        },
      };
      await use(scriba);
    } finally {
      await kill(child);
      rmSync(dataDir, { recursive: true, force: true, maxRetries: 5 });
    }
  },
});

export { expect };
