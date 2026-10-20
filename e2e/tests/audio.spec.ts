import { existsSync, mkdirSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "../fixtures";

// "Guardar o áudio?" no fim de cada transcrição. O pipeline falso grava um texto fixo no
// lugar do .m4a, e o "Salvar como" responde o que o teste mandar (`scriba.saveAs`).

const file = (name: string) => ({ name, mimeType: "application/octet-stream", buffer: Buffer.alloc(2048, 7) });

const card = (page: Page, name: string) => page.locator("article.job-card", { has: page.getByRole("heading", { name }) });

/** Envia, espera concluir e devolve a pasta da saída. */
async function transcribe(page: Page, outputDir: string, name: string): Promise<string> {
  await page.locator("#arquivo").setInputFiles(file(name));
  await expect(card(page, name).getByText("Guardar o áudio?")).toBeVisible({ timeout: 15_000 });
  const folders = readdirSync(outputDir).filter((f) => f.endsWith(name.replace(/\.[^.]+$/, "")));
  expect(folders).toHaveLength(1);
  return join(outputDir, folders[0]);
}

test("descartar apaga o temporário", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await transcribe(page, scriba.outputDir, "reuniao.mp4");
  expect(scriba.uploads()).toHaveLength(1);

  const job = card(page, "reuniao.mp4");
  await job.getByRole("button", { name: "Descartar áudio de reuniao.mp4" }).click();
  await expect(job.getByText("Guardar o áudio?")).toBeHidden();
  await expect(page.locator("#avisos")).toHaveText("Áudio de reuniao.mp4 descartado.");
  expect(scriba.uploads()).toEqual([]);
  // A transcrição continua.
  await expect(job.getByRole("link", { name: "Ver transcrição" })).toBeVisible();

  await page.reload();
  await expect(card(page, "reuniao.mp4").getByText("Guardar o áudio?")).toHaveCount(0);
});

test("guardar o áudio: janela cancelada e depois um arquivo", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  const folder = await transcribe(page, scriba.outputDir, "reuniao.mp4");
  const job = card(page, "reuniao.mp4");
  const save = job.getByRole("button", { name: "Guardar áudio de reuniao.mp4" });

  // Cancelar a janela mantém a pergunta e o temporário.
  scriba.saveAs(null);
  await save.click();
  await expect.poll(scriba.asked).toHaveLength(1);
  await expect(save).toBeVisible();
  const name = folder.split("\\").pop();
  expect(scriba.asked()[0]).toEqual({ dir: folder, name: `${name}.m4a` });
  expect(scriba.uploads()).toHaveLength(1);

  const audios = join(scriba.outputDir, "..", "Áudios");
  mkdirSync(audios);
  const chosen = join(audios, "Reunião.m4a");
  scriba.saveAs(chosen);
  await save.click();
  await expect(job.getByText(`Áudio guardado em ${chosen}`)).toBeVisible({ timeout: 10_000 });
  await expect(page.locator("#avisos")).toHaveText("Áudio de reuniao.mp4 guardado.");
  expect(readFileSync(chosen, "utf8")).toBe("áudio simulado de reuniao.mp4");
  expect(scriba.uploads()).toEqual([]);

  await job.getByRole("button", { name: "Mostrar no Explorer o áudio de reuniao.mp4" }).click();
  await expect.poll(scriba.revealed).toEqual([chosen]);

  // A próxima janela abre na pasta escolhida.
  await transcribe(page, scriba.outputDir, "aula.mp3");
  scriba.saveAs(null);
  await card(page, "aula.mp3").getByRole("button", { name: "Guardar áudio de aula.mp3" }).click();
  await expect.poll(() => scriba.asked().length).toBe(3);
  expect(scriba.asked()[2].dir).toBe(audios);
});

test("cancelar a gravação do áudio volta à pergunta", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await transcribe(page, scriba.outputDir, "audio-demorado.mp4");
  const job = card(page, "audio-demorado.mp4");
  const chosen = join(scriba.outputDir, "demorado.m4a");
  scriba.saveAs(chosen);
  await job.getByRole("button", { name: "Guardar áudio de audio-demorado.mp4" }).click();
  await expect(job.getByText(/Guardando áudio… \d+%/)).toBeVisible({ timeout: 10_000 });
  await expect(job.getByRole("progressbar", { name: "Áudio de audio-demorado.mp4" })).toBeVisible();

  const cancel = job.getByRole("button", { name: "Cancelar o áudio de audio-demorado.mp4" });
  await cancel.click();
  await expect(job.getByText("Guardar o áudio?")).toBeVisible({ timeout: 10_000 });
  // O botão de ação mantém o id e volta a ser o Remover.
  await expect(job.getByRole("button", { name: "Remover audio-demorado.mp4" })).toBeVisible();
  expect(existsSync(chosen)).toBe(false);
  expect(readdirSync(scriba.outputDir).filter((f) => f.endsWith(".parte"))).toEqual([]);
  expect(scriba.uploads()).toHaveLength(1);
});

test("erro ao guardar o áudio vira toast e a pergunta volta", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await transcribe(page, scriba.outputDir, "audio-ruim.mp3");
  const job = card(page, "audio-ruim.mp3");
  scriba.saveAs(join(scriba.outputDir, "ruim.m4a"));
  await job.getByRole("button", { name: "Guardar áudio de audio-ruim.mp3" }).click();
  await expect(page.locator(".toast")).toHaveText("Não consegui guardar o áudio (falha simulada).", { timeout: 10_000 });
  await expect(job.getByText("Guardar o áudio?")).toBeVisible();
  expect(scriba.uploads()).toHaveLength(1);
});

for (const scheme of ["light", "dark"] as const) {
  test(`pergunta e áudio guardado batem com a referência visual (${scheme})`, async ({ page, scriba }) => {
    test.skip(!process.env.CI, "referência visual só no CI");
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.goto(scriba.url);

    await transcribe(page, scriba.outputDir, "reuniao.mp4");
    scriba.saveAs(join(scriba.outputDir, "reuniao.m4a"));
    await card(page, "reuniao.mp4").getByRole("button", { name: "Guardar áudio de reuniao.mp4" }).click();
    await expect(card(page, "reuniao.mp4").getByText("Áudio guardado em")).toBeVisible({ timeout: 10_000 });
    await transcribe(page, scriba.outputDir, "aula.mp3");

    await page.evaluate(() => document.fonts.ready);
    // A data do envio e a pasta temporária mudam a cada execução.
    await expect(page.locator(".recentes")).toHaveScreenshot(`audio-${scheme}.png`, {
      mask: [page.locator(".job-card-date"), page.locator(".audio-prompt-path")],
    });
  });
}
