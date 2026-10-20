import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "../fixtures";

// O exe roda o pipeline falso (~2 s por etapa; duração de 2 min e "Cerca de 4 min no
// processador" no probe). Pelo nome do arquivo: "lento" fica 10× mais devagar, "falha"
// falha na metade da primeira etapa, "falha-uma-vez" só na primeira tentativa e
// "sem-audio" é recusado no envio.

/** Arquivo de 2 KB: o conteúdo não importa para o pipeline falso. */
const file = (name: string) => ({ name, mimeType: "application/octet-stream", buffer: Buffer.alloc(2048, 7) });

const card = (page: Page, name: string) => page.locator("article.job-card", { has: page.getByRole("heading", { name }) });

async function upload(page: Page, name: string) {
  await page.locator("#arquivo").setInputFiles(file(name));
  await expect(card(page, name)).toBeVisible();
}

test("enviar um arquivo mostra o progresso até concluir", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await upload(page, "reuniao.mp4");
  const job = card(page, "reuniao.mp4");
  await expect(job.getByText(/^(Extraindo áudio|Transcrevendo)… \d+%$/)).toBeVisible();
  await expect(job.getByRole("progressbar", { name: "Progresso de reuniao.mp4" })).toBeVisible();
  await expect(page.getByText("Nada por aqui ainda")).toBeHidden();

  await expect(job.getByText("Concluído")).toBeVisible({ timeout: 15_000 });
  await expect(job.getByRole("progressbar")).toHaveCount(0);
  await expect(page.locator("#avisos")).toHaveText("reuniao.mp4: concluído.");
});

test("soltar um arquivo na área de envio cria o job", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  const dataTransfer = await page.evaluateHandle(() => {
    const dt = new DataTransfer();
    dt.items.add(new File(["x".repeat(1024)], "solto.wav", { type: "audio/wav" }));
    return dt;
  });
  await page.dispatchEvent("#envio", "drop", { dataTransfer });
  await expect(card(page, "solto.wav")).toBeVisible();
});

test("cancelar um job em andamento", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await upload(page, "lento.mkv");
  const job = card(page, "lento.mkv");
  await expect(job.getByText(/^Extraindo áudio… \d+%$/)).toBeVisible();

  await job.getByRole("button", { name: "Cancelar lento.mkv" }).click();
  await expect(job.getByText("Cancelado")).toBeVisible({ timeout: 10_000 });
  await expect(job.getByRole("button", { name: "Remover lento.mkv" })).toBeVisible();
  await expect(job.getByRole("progressbar")).toHaveCount(0);
});

test("o foco no botão do card sobrevive ao polling e ao fim do job", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await upload(page, "lento.mkv");
  const cancel = page.getByRole("button", { name: "Cancelar lento.mkv" });
  await cancel.focus();
  // Cada mudança da barra é um card novo vindo do polling; o htmx devolve o foco pelo id.
  const bar = card(page, "lento.mkv").getByRole("progressbar");
  for (let i = 0; i < 2; i++) {
    const before = await bar.getAttribute("value");
    await expect(bar).not.toHaveAttribute("value", before!);
  }
  await expect(cancel).toBeFocused();

  await page.keyboard.press("Enter");
  const remove = page.getByRole("button", { name: "Remover lento.mkv" });
  await expect(remove).toBeVisible({ timeout: 10_000 });
  await expect(remove).toBeFocused();
});

test("a falha aparece no card", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await upload(page, "falha.wav");
  const job = card(page, "falha.wav");
  await expect(job.getByText("Falhou")).toBeVisible({ timeout: 10_000 });
  await expect(job.getByText("Não foi possível processar este arquivo (falha simulada).")).toBeVisible();
});

test("nome longo trunca em vez de alargar a página estreita", async ({ page, scriba }) => {
  await page.setViewportSize({ width: 420, height: 900 });
  await page.goto(scriba.url);
  await upload(page, `${"Reunião de planejamento trimestral ".repeat(3)}.mp4`);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBe(0);
});

test("formato não suportado é recusado sem criar job", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await page.locator("#arquivo").setInputFiles(file("nota.pdf"));
  await expect(page.getByRole("alert")).toHaveText("Formato não suportado. Use MP4, MOV, MKV, MP3, M4A, WAV ou OGG.");
  await expect(page.locator("article.job-card")).toHaveCount(0);
});

test("arquivo sem áudio é recusado sem criar job", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await page.locator("#arquivo").setInputFiles(file("tela-sem-audio.mp4"));
  await expect(page.getByRole("alert")).toHaveText("Este arquivo não tem áudio. Escolha um vídeo com som.");
  await expect(page.locator("article.job-card")).toHaveCount(0);
});

test("o card mostra duração, tamanho, data e, na fila, a estimativa", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await upload(page, "lento.mkv");
  await upload(page, "depois.mp3");
  const queued = card(page, "depois.mp3");
  await expect(queued.getByText("Na fila")).toBeVisible();
  await expect(queued.getByText(/^2 min · 2 KB · \d{2}\/\d{2}\/\d{4} \d{2}:\d{2}$/)).toBeVisible();
  await expect(queued.getByText("Cerca de 4 min no processador")).toBeVisible();
  // Rodando, a estimativa sai e a etapa entra.
  await expect(card(page, "lento.mkv").getByText("Cerca de")).toHaveCount(0);
});

test("tentar de novo um job que falhou", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await upload(page, "falha-uma-vez.wav");
  const job = card(page, "falha-uma-vez.wav");
  await expect(job.getByText("Falhou")).toBeVisible({ timeout: 10_000 });

  await job.getByRole("button", { name: "Tentar de novo" }).click();
  await expect(job.getByText(/^(Extraindo áudio|Transcrevendo)… \d+%$/)).toBeVisible();
  await expect(job.getByText("Concluído")).toBeVisible({ timeout: 15_000 });
  await expect(job.getByRole("button", { name: "Tentar de novo" })).toHaveCount(0);
  await expect(page.locator("#avisos")).toHaveText("falha-uma-vez.wav: concluído.");
});

test("remover um job concluído pede confirmação e manda a pasta para a Lixeira", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await upload(page, "curto.mp3");
  const job = card(page, "curto.mp3");
  await expect(job.getByText("Concluído")).toBeVisible({ timeout: 15_000 });
  const folders = readdirSync(scriba.outputDir);
  expect(folders).toEqual([expect.stringMatching(/^\d{4}-\d{2}-\d{2} \d{4} - curto$/)]);

  const remove = job.getByRole("button", { name: "Remover curto.mp3" });
  const dialog = page.getByRole("dialog", { name: "Excluir a transcrição?" });
  await remove.click();
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText(`A pasta “${folders[0]}” vai para a Lixeira.`);
  await expect(dialog.getByRole("button", { name: "Cancelar" })).toBeFocused();
  // Esc cancela: nada muda e o foco volta ao Remover.
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(remove).toBeFocused();
  await expect(job).toBeVisible();
  expect(existsSync(join(scriba.outputDir, folders[0]))).toBe(true);

  await remove.click();
  await dialog.getByRole("button", { name: "Excluir" }).click();
  await expect(page.locator("article.job-card")).toHaveCount(0);
  await expect(page.getByText("Nada por aqui ainda")).toBeVisible();
  expect(readdirSync(scriba.outputDir)).toEqual([]);

  await page.reload();
  await expect(page.locator("article.job-card")).toHaveCount(0);
});

test("remover um job cancelado não pede confirmação", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await upload(page, "lento.mkv");
  const job = card(page, "lento.mkv");
  await job.getByRole("button", { name: "Cancelar lento.mkv" }).click();
  await expect(job.getByText("Cancelado")).toBeVisible({ timeout: 10_000 });
  await job.getByRole("button", { name: "Remover lento.mkv" }).click();
  await expect(page.locator("article.job-card")).toHaveCount(0);
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

for (const scheme of ["light", "dark"] as const) {
  test(`cards finais batem com a referência visual (${scheme})`, async ({ page, scriba }) => {
    test.skip(!process.env.CI, "referência visual só no CI");
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.goto(scriba.url);

    await upload(page, "reuniao.mp4");
    await expect(card(page, "reuniao.mp4").getByText("Concluído")).toBeVisible({ timeout: 15_000 });
    await upload(page, "falha.wav");
    await expect(card(page, "falha.wav").getByText("Falhou")).toBeVisible({ timeout: 10_000 });
    await upload(page, "lento.mkv");
    await card(page, "lento.mkv").getByRole("button", { name: "Cancelar lento.mkv" }).click();
    await expect(card(page, "lento.mkv").getByText("Cancelado")).toBeVisible({ timeout: 10_000 });

    await page.evaluate(() => document.fonts.ready);
    // A data do envio muda a cada execução.
    await expect(page.locator(".recentes")).toHaveScreenshot(`recentes-${scheme}.png`, {
      mask: [page.locator(".job-card-date")],
    });
  });
}
