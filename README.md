<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/brand/logo-white.svg">
    <img src="docs/brand/logo.svg" width="96" height="96" alt="Logo do Scriba">
  </picture>
</p>

<h1 align="center">Scriba</h1>

<p align="center">Grava e transcreve — offline, num único <code>.exe</code> para Windows.</p>

<p align="center">
  <a href="https://github.com/filipimr/scriba/actions/workflows/ci.yml"><img src="https://github.com/filipimr/scriba/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/filipimr/scriba/releases/latest"><img src="https://img.shields.io/github/v/release/filipimr/scriba" alt="Última release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/filipimr/scriba" alt="Licença GPL-3.0"></a>
</p>

> **Em desenvolvimento.** O app já sobe a interface local e transcreve os arquivos enviados para `.txt` e `.md` na pasta Documentos\Scriba, na GPU dedicada ou no processador. Na tela, cada transcrição concluída abre no leitor, com Copiar texto, Baixar .txt e .md e Abrir pasta. O andamento fica na [tabela de status do ROADMAP](ROADMAP.md#status-das-fases).

## O que o Scriba faz

- **Transcreve** áudio e vídeo em português do Brasil. O modelo de IA (Whisper large-v3-turbo) vem dentro do executável: você nunca escolhe nem baixa um modelo.
- **Grava** o microfone e o som do PC em trilhas separadas. A transcrição sai rotulada "Você" / "Outros".
- **Descarta a mídia** depois do texto; se você quiser, guarda o áudio num `.m4a` na pasta que escolher.
- **Entrega o texto:** um `transcricao.txt` ou um `transcricao.md` com marcações de tempo.

## Privacidade

Tudo roda no seu computador e funciona sem internet. A interface abre no navegador, mas é servida pelo próprio executável em `127.0.0.1`, com um token de sessão e checagem de Host e Origin, para que nenhum site aberto no navegador consiga acionar o app.

## Requisitos

- Windows 10 ou 11, 64 bits.
- Opcional: GPU com driver Vulkan, que acelera bastante a transcrição. Sem GPU, a transcrição roda na CPU.
- Nada para instalar: é um `.exe` só.

## Como usar

1. Baixe o `scriba.exe` da [última release](https://github.com/filipimr/scriba/releases/latest). As releases atuais são marcos de desenvolvimento e ainda não transcrevem arquivos.
2. Confira o checksum com o `scriba.exe.sha256` publicado junto:

   ```powershell
   (Get-FileHash .\scriba.exe -Algorithm SHA256).Hash.ToLower()
   ```

3. Execute o `scriba.exe`. A interface abre no navegador padrão, em `http://127.0.0.1:47800`. Executar de novo reabre a mesma instância.

| Opção | O que faz |
| --- | --- |
| `--port <n>` | Porta local preferida (padrão 47800). Se estiver ocupada, usa outra livre; `0` deixa o sistema escolher |
| `--no-open` | Não abre o navegador ao iniciar |
| `--data-dir <pasta>` | Dados internos do app (padrão `%LOCALAPPDATA%\Scriba`) |
| `--output-dir <pasta>` | Onde ficam as transcrições e mídias geradas (padrão `Documentos\Scriba`) |
| `--self-test` | Transcreve uma amostra embutida e codifica um clipe curto, para conferir o executável |

## Desenvolvimento

O build é nativo e estático (FFmpeg, whisper.cpp e o CRT dentro do exe), por isso o ambiente tem alguns passos únicos:

1. Em um PowerShell de administrador, rode `powershell -ExecutionPolicy Bypass -File scripts\setup.ps1`. Ele instala Rust, VS Build Tools, LLVM, CMake, vcpkg e Vulkan SDK, e define `CARGO_TARGET_DIR=C:\dev\t\scriba` (o build do whisper.cpp estoura o limite de 260 caracteres de caminho dentro do repositório).
2. Compile o FFmpeg estático uma vez (cerca de 12–20 min; depois fica em cache):

   ```powershell
   C:\dev\vcpkg\vcpkg.exe install --triplet x64-windows-static-release --x-install-root=vcpkg_installed
   ```

3. Baixe os modelos com `cargo xtask model` (ou só o pequeno, usado nos testes, com `--only base`).

Comandos do dia a dia:

| Tarefa | Comando |
| --- | --- |
| Rodar o app | `cargo run` |
| Testes | `cargo test --workspace --features spikes/vulkan` |
| Lint | `cargo fmt --all --check` e `cargo clippy --workspace --all-targets --features spikes/vulkan -- -D warnings` |
| E2E (Playwright no Edge) | `cargo build`, depois `npm ci` e `npx playwright test` em `e2e/` |
| CSS a partir do design system | `cargo xtask css` |

Testes que dependem de GPU ou de dispositivos de áudio reais ficam fora do CI e rodam localmente com `-- --ignored`. Todo PR passa pelo CI (formatação, clippy, testes com cobertura mínima, E2E e `cargo deny`) antes do merge.

Documentos do projeto:

- [`ROADMAP.md`](ROADMAP.md): decisões de produto e arquitetura, fases com checklists, política de testes e registro de decisões.
- [`DESIGN-scriba.md`](DESIGN-scriba.md): o design system da interface, com os tokens que viram CSS.
- [`CLAUDE.md`](CLAUDE.md): notas de trabalho e comandos para o agente que ajuda no desenvolvimento.

## Arquitetura

Um monólito em Rust, distribuído como um único executável:

- **Interface:** servidor Axum local com htmx, Tailwind compilado e a fonte Inter, tudo embutido no binário.
- **Transcrição:** whisper-rs (whisper.cpp) com aceleração Vulkan e o modelo carregado da memória.
- **Mídia:** ffmpeg-next sobre um FFmpeg estático com x264.
- **Captura:** cpal (WASAPI), com microfone e som do sistema gravados em trilhas separadas.

## Licença

GPL-3.0-or-later (veja [`LICENSE`](LICENSE)); o executável inclui o x264, que é GPL. Cada release publica o `THIRD_PARTY_LICENSES.html` com as licenças das dependências.

O Scriba se apoia em [whisper.cpp](https://github.com/ggml-org/whisper.cpp), no modelo [Whisper](https://github.com/openai/whisper) da OpenAI, em [FFmpeg](https://ffmpeg.org) e [x264](https://www.videolan.org/developers/x264.html), na fonte [Inter](https://rsms.me/inter/), em [htmx](https://htmx.org), nos ícones [Lucide](https://lucide.dev) e no [Tailwind CSS](https://tailwindcss.com).
