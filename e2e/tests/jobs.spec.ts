import type { Page } from "@playwright/test";
import { expect, test } from "../fixtures";

// O app usa o pipeline falso nesta fase (~2 s por etapa). Pelo nome do arquivo:
// "lento" fica 10× mais devagar e "falha" falha na metade da primeira etapa.

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

test("sem nenhuma opção marcada o envio é recusado", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await page.getByText("Transcrever", { exact: true }).click();
  await expect(page.getByRole("checkbox", { name: "Transcrever" })).not.toBeChecked();
  await page.locator("#arquivo").setInputFiles(file("reuniao.mp4"));
  await expect(page.getByRole("alert")).toHaveText("Escolha Transcrever, Compactar ou os dois.");
  await expect(page.locator("article.job-card")).toHaveCount(0);
});

test("remover um job concluído volta ao estado vazio", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await upload(page, "curto.mp3");
  const job = card(page, "curto.mp3");
  await expect(job.getByText("Concluído")).toBeVisible({ timeout: 15_000 });

  await job.getByRole("button", { name: "Remover curto.mp3" }).click();
  await expect(page.locator("article.job-card")).toHaveCount(0);
  await expect(page.getByText("Nada por aqui ainda")).toBeVisible();

  await page.reload();
  await expect(page.locator("article.job-card")).toHaveCount(0);
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
    await expect(page.locator(".recentes")).toHaveScreenshot(`recentes-${scheme}.png`);
  });
}
