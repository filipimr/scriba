import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/** Caminho do scriba.exe de debug; SCRIBA_EXE sobrescreve. */
function exe(): string {
  if (process.env.SCRIBA_EXE) return process.env.SCRIBA_EXE;
  const target = process.env.CARGO_TARGET_DIR ?? join(__dirname, "..", "target");
  return join(target, "debug", "scriba.exe");
}

export default async function globalSetup() {
  const dataDir = mkdtempSync(join(tmpdir(), "scriba-e2e-"));
  const child = spawn(exe(), ["--no-open", "--port", "0", "--data-dir", dataDir, "--output-dir", join(dataDir, "saidas")], {
    stdio: ["ignore", "pipe", "inherit"],
  });

  const url = await new Promise<string>((resolve, reject) => {
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
  process.env.SCRIBA_URL = url;

  return async () => {
    child.kill();
    rmSync(dataDir, { recursive: true, force: true });
  };
}
