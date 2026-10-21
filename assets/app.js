// Aplica no <html> o tema que o servidor acabou de salvar (evento HX-Trigger "scriba-theme").
document.body.addEventListener("scriba-theme", (e) => {
  const mode = e.detail.mode;
  if (mode === "system") {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = mode;
  }
});

// Soltar um arquivo fora da dropzone não pode abrir o arquivo na aba e fechar o app.
window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("drop", (e) => e.preventDefault());

// Toast no rodapé central: o novo substitui o anterior. A confirmação some em 3 s; o erro
// fica 6 s, para dar tempo de ler.
const toasts = document.getElementById("toasts");
let toastTimer;
const toast = (message, ms = 3000) => {
  const el = document.createElement("p");
  el.className = "toast";
  el.textContent = message;
  toasts.replaceChildren(el);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.remove(), ms);
};
const toastError = (message) => toast(message, 6000);

// "Copiar texto": o conteúdo do .txt vem de um <template> (a CSP proíbe script inline). O
// parser do HTML troca CRLF por LF; o CRLF volta para o texto copiado ser igual ao arquivo.
document.body.addEventListener("click", async (e) => {
  const button = e.target.closest("[data-copiar]");
  if (!button) return;
  const source = document.getElementById(button.dataset.copiar);
  const text = source.content.textContent.replace(/\r?\n/g, "\r\n");
  try {
    await navigator.clipboard.writeText(text);
    toast("Texto copiado");
  } catch {
    toastError("Não consegui copiar o texto. Selecione e copie com Ctrl+C.");
  }
});

// "Baixar .txt" e "Baixar .md": baixa pelo fetch para um erro (pasta apagada fora do app)
// virar toast em vez de um download falho. Sem o JS, o link baixa sozinho.
document.body.addEventListener("click", async (e) => {
  const link = e.target.closest("a[download]");
  if (!link || e.button !== 0 || e.ctrlKey || e.shiftKey || e.metaKey || e.altKey) return;
  e.preventDefault();
  try {
    const res = await fetch(link.href);
    if (!res.ok) return toastError(await res.text());
    const url = URL.createObjectURL(await res.blob());
    const a = document.createElement("a");
    a.href = url;
    a.download = link.getAttribute("download");
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 10_000);
  } catch {
    toastError("Não consegui baixar a transcrição. Tente de novo.");
  }
});

// Erro de uma ação (Abrir pasta, Remover): o texto da resposta vira toast. O polling (GET)
// fica de fora.
document.body.addEventListener("htmx:responseError", (e) => {
  if (e.detail.requestConfig?.verb === "get") return;
  toastError(e.detail.xhr.responseText || "Algo deu errado. Tente de novo.");
});

// Erro que chega pelo polling (HX-Trigger "scriba-erro"), como o "Guardar áudio" que falhou.
document.body.addEventListener("scriba-erro", (e) => toastError(e.detail.value));

// Remover um job concluído e cancelar uma gravação (hx-confirm) pedem confirmação no <dialog>
// do design, não no window.confirm. O título e o botão vêm dos data-confirmar-* de quem pediu.
// O showModal prende o foco, o Esc cancela e o foco volta ao botão.
const dialog = document.getElementById("confirmar");
if (dialog) {
  const title = dialog.querySelector("#confirmar-titulo");
  const action = dialog.querySelector(".btn-danger");
  const defaults = { title: title.textContent, action: action.textContent };
  document.body.addEventListener("htmx:confirm", (e) => {
    if (!e.detail.question) return;
    e.preventDefault();
    const source = e.detail.elt;
    title.textContent = source.dataset.confirmarTitulo ?? defaults.title;
    action.textContent = source.dataset.confirmarAcao ?? defaults.action;
    dialog.querySelector("#confirmar-texto").textContent = e.detail.question;
    dialog.returnValue = "";
    dialog.addEventListener(
      "close",
      () => {
        if (dialog.returnValue !== "excluir") return;
        // O polling pode ter trocado o card enquanto o diálogo estava aberto: o pedido sai do
        // botão de agora, com o mesmo id.
        const current = source.id && document.getElementById(source.id);
        if (current && current !== source) {
          const card = current.closest("article");
          htmx.ajax(e.detail.verb.toUpperCase(), e.detail.path, { source: current, target: card, swap: "outerHTML" });
        } else {
          e.detail.issueRequest(true);
        }
      },
      { once: true },
    );
    dialog.showModal();
  });
}

// O card de uma gravação chega fora de banda (Parar, ou o polling da barra quando ela para
// sozinha); se as duas respostas chegarem, fica só o card de cima.
document.body.addEventListener("htmx:oobAfterSwap", () => {
  const seen = new Set();
  for (const card of document.querySelectorAll("#jobs > article.job-card")) {
    if (seen.has(card.id)) card.remove();
    else seen.add(card.id);
  }
});

// Gravação: o tempo e os medidores vêm do SSE (~10 por segundo) enquanto a barra existe; o
// estado (início, fim, avisos) vem do polling da barra. O stream termina com o evento "fim".
let levels;
const syncLevels = () => {
  const recording = document.querySelector("#barra-gravacao[data-gravando]");
  if (recording && !levels) {
    levels = new EventSource("/api/record/levels");
    levels.onmessage = (e) => {
      const data = JSON.parse(e.data);
      for (const el of document.querySelectorAll("[data-tempo]")) el.textContent = data.tempo;
      for (const meter of document.querySelectorAll("meter[data-nivel]")) meter.value = data.niveis[meter.dataset.nivel];
    };
    const stop = () => {
      levels?.close();
      levels = undefined;
    };
    levels.addEventListener("fim", stop);
    levels.onerror = stop;
  } else if (!recording && levels) {
    levels.close();
    levels = undefined;
  }
};
syncLevels();
document.body.addEventListener("htmx:afterSettle", syncLevels);
document.body.addEventListener("htmx:oobAfterSwap", syncLevels);

// Ctrl+Shift+R: Gravar / Parar em qualquer página (no lugar do recarregar do navegador).
document.addEventListener("keydown", (e) => {
  if (!(e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey && e.code === "KeyR")) return;
  e.preventDefault();
  if (e.repeat) return;
  const button = document.querySelector("[data-gravar]") ?? document.querySelector("[data-parar]");
  if (button) {
    if (!button.disabled) button.click();
  } else {
    htmx.ajax("POST", "/api/record/start", { source: document.body, swap: "none" });
  }
});

// Envio de arquivos: soltar na dropzone ou escolher no botão. O htmx faz o POST multipart; aqui
// só validamos antes de enviar (formatos do `accept`, mensagens vindas do servidor) e
// mostramos o progresso do upload.
const form = document.getElementById("envio");
if (form) {
  const input = form.querySelector("#arquivo");
  const errorBox = form.querySelector("#erro-envio");
  const fileName = form.querySelector(".dropzone-file");
  const bar = form.querySelector(".dropzone-uploading progress");
  const formats = input.accept.split(",");

  const showError = (message) => {
    const alert = document.getElementById("modelo-erro").content.firstElementChild.cloneNode(true);
    alert.querySelector("span").textContent = message;
    errorBox.replaceChildren(alert);
  };

  const send = (files) => {
    if (form.classList.contains("is-uploading") || files.length === 0) return;
    const supported = [...files].every((f) => formats.some((ext) => f.name.toLowerCase().endsWith(ext)));
    if (!supported) {
      input.value = "";
      return showError(form.dataset.erroFormato);
    }
    if (files !== input.files) input.files = files;
    errorBox.replaceChildren();
    fileName.textContent = files.length === 1 ? files[0].name : `${files.length} arquivos`;
    bar.value = 0;
    form.classList.add("is-uploading");
    form.requestSubmit();
  };

  input.addEventListener("change", () => send(input.files));

  form.addEventListener("dragover", (e) => {
    e.preventDefault();
    form.classList.add("dropzone-dragover");
  });
  form.addEventListener("dragleave", (e) => {
    if (!form.contains(e.relatedTarget)) form.classList.remove("dropzone-dragover");
  });
  form.addEventListener("drop", (e) => {
    e.preventDefault();
    form.classList.remove("dropzone-dragover");
    send(e.dataTransfer.files);
  });

  form.addEventListener("htmx:xhr:progress", (e) => {
    if (e.detail.lengthComputable && e.detail.total > 0) {
      bar.value = Math.round((e.detail.loaded / e.detail.total) * 100);
    }
  });
  form.addEventListener("htmx:afterRequest", () => {
    form.classList.remove("is-uploading");
    input.value = "";
  });
}
