import { readFileSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "../fixtures";

// O pipeline falso grava uma transcrição fixa de dois parágrafos (0 s e 8 s) com o módulo
// `transcript` real, numa pasta "AAAA-MM-DD HHmm - <nome>" dentro de `outputDir`.

const file = (name: string) => ({ name, mimeType: "application/octet-stream", buffer: Buffer.alloc(2048, 7) });

const card = (page: Page, name: string) => page.locator("article.job-card", { has: page.getByRole("heading", { name }) });

/** Envia, espera concluir e devolve a pasta da saída. */
async function transcribe(page: Page, outputDir: string, name: string): Promise<string> {
  await page.locator("#arquivo").setInputFiles(file(name));
  await expect(card(page, name).getByText("Concluído")).toBeVisible({ timeout: 15_000 });
  const folders = readdirSync(outputDir);
  expect(folders).toHaveLength(1);
  return join(outputDir, folders[0]);
}

test("ver a transcrição e copiar o texto", async ({ page, context, scriba }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto(scriba.url);
  const folder = await transcribe(page, scriba.outputDir, "reuniao.mp4");

  await card(page, "reuniao.mp4").getByRole("link", { name: "Ver transcrição" }).click();
  await expect(page).toHaveURL(/\/jobs\/[0-9A-Z]{26}$/);
  await expect(page).toHaveTitle("reuniao.mp4 · Scriba");
  await expect(page.getByRole("heading", { level: 1, name: "reuniao.mp4" })).toBeVisible();
  await expect(page.getByText("Bom dia, esta é uma transcrição simulada.")).toBeVisible();
  await expect(page.locator(".transcript-paragraph").nth(1)).toHaveText("00:00:08 O Scriba não ouviu este arquivo.");

  await page.getByRole("button", { name: "Copiar texto" }).click();
  await expect(page.locator(".toast")).toHaveText("Texto copiado");
  await expect(page.locator("#toasts")).toHaveAttribute("aria-live", "polite");
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied).toBe(readFileSync(join(folder, "transcricao.txt"), "utf8"));
  expect(copied).toContain("\r\n[00:00:08] O Scriba não ouviu este arquivo.\r\n");
  await expect(page.locator(".toast")).toBeHidden({ timeout: 5_000 });

  await page.getByRole("link", { name: "Recentes" }).click();
  await expect(page).toHaveURL(`${scriba.origin}/`);
  await expect(card(page, "reuniao.mp4")).toBeVisible();
});

test("baixar o .txt e o .md", async ({ page, context, scriba }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto(scriba.url);
  const folder = await transcribe(page, scriba.outputDir, "reunião.mp4");
  const txt = readFileSync(join(folder, "transcricao.txt"), "utf8");

  // No card.
  let wait = page.waitForEvent("download");
  await card(page, "reunião.mp4").getByRole("link", { name: "Baixar .txt" }).click();
  let download = await wait;
  expect(download.suggestedFilename()).toBe("reunião - transcrição.txt");
  expect(readFileSync(await download.path(), "utf8")).toBe(txt);

  // No leitor: o .txt igual ao copiado; o .md com o título e os mesmos parágrafos.
  await card(page, "reunião.mp4").getByRole("link", { name: "Ver transcrição" }).click();
  await page.getByRole("button", { name: "Copiar texto" }).click();
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  wait = page.waitForEvent("download");
  await page.getByRole("link", { name: "Baixar .txt" }).click();
  download = await wait;
  expect(readFileSync(await download.path(), "utf8")).toBe(copied);

  wait = page.waitForEvent("download");
  await page.getByRole("link", { name: "Baixar .md" }).click();
  download = await wait;
  expect(download.suggestedFilename()).toBe("reunião - transcrição.md");
  const md = readFileSync(await download.path(), "utf8");
  expect(md.startsWith("# reunião.mp4\n")).toBe(true);
  expect(md).toContain("\n**[00:00:00]** Bom dia, esta é uma transcrição simulada.\n");
  expect(md).toContain("\n**[00:00:08]** O Scriba não ouviu este arquivo.\n");
});

test("abrir a pasta, e a pasta apagada fora do app", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  const folder = await transcribe(page, scriba.outputDir, "reuniao.mp4");

  await card(page, "reuniao.mp4").getByRole("button", { name: "Abrir pasta" }).click();
  await expect.poll(scriba.opened).toEqual([folder]);
  await card(page, "reuniao.mp4").getByRole("link", { name: "Ver transcrição" }).click();
  await page.getByRole("button", { name: "Abrir pasta" }).click();
  await expect.poll(scriba.opened).toEqual([folder, folder]);
  await expect(page.locator(".toast")).toHaveCount(0);

  // Apagada: as ações viram toast de erro e o leitor mostra o alerta.
  rmSync(folder, { recursive: true });
  const name = folder.split("\\").pop();
  await page.getByRole("button", { name: "Abrir pasta" }).click();
  await expect(page.locator(".toast")).toHaveText(`Não encontrei a pasta “${name}”. Ela foi movida ou apagada.`);
  await page.getByRole("link", { name: "Baixar .md" }).click();
  await expect(page.locator(".toast")).toHaveText(`Não encontrei a transcrição na pasta “${name}”. Ela foi movida ou apagada.`);
  expect(scriba.opened()).toHaveLength(2);

  await page.reload();
  await expect(page.getByText(`Não encontrei a transcrição em ${folder}.`)).toBeVisible();
  await expect(page.getByRole("button", { name: "Copiar texto" })).toHaveCount(0);
});

test("o leitor de um job fora da lista", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  const res = await page.goto(`${scriba.origin}/jobs/01JAZ5Y3V8K9M2N4P6Q7R8S9T0`);
  expect(res?.status()).toBe(404);
  await expect(page.getByText("Não encontrei esta transcrição na lista de Recentes.")).toBeVisible();
  await expect(page.getByRole("link", { name: "Recentes" })).toBeVisible();
});

for (const scheme of ["light", "dark"] as const) {
  test(`leitor e toast batem com a referência visual (${scheme})`, async ({ page, context, scriba }) => {
    test.skip(!process.env.CI, "referência visual só no CI");
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.goto(scriba.url);
    await transcribe(page, scriba.outputDir, "reuniao.mp4");
    const reader = await card(page, "reuniao.mp4").getByRole("link", { name: "Ver transcrição" }).getAttribute("href");

    // Relógio parado só no leitor (a lista precisa do polling): o toast não some na captura.
    await page.clock.install();
    await page.goto(`${scriba.origin}${reader}`);
    await page.getByRole("button", { name: "Copiar texto" }).click();
    await expect(page.locator(".toast")).toHaveText("Texto copiado");
    await page.evaluate(() => document.fonts.ready);
    // A data do envio muda a cada execução.
    await expect(page).toHaveScreenshot(`leitor-${scheme}.png`, {
      mask: [page.locator(".transcript-date")],
    });
  });
}
