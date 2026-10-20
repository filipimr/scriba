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

// Remover um job concluído (hx-confirm) pede confirmação no <dialog> do design, não no
// window.confirm. O showModal prende o foco, o Esc cancela e o foco volta ao botão.
const dialog = document.getElementById("confirmar");
if (dialog) {
  document.body.addEventListener("htmx:confirm", (e) => {
    if (!e.detail.question) return;
    e.preventDefault();
    dialog.querySelector("#confirmar-texto").textContent = e.detail.question;
    dialog.returnValue = "";
    dialog.addEventListener(
      "close",
      () => {
        if (dialog.returnValue === "excluir") e.detail.issueRequest(true);
      },
      { once: true },
    );
    dialog.showModal();
  });
}

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
