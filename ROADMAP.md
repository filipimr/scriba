# Scriba — Roadmap

App local para Windows, distribuído como um único `.exe`, que **grava** (microfone + som do PC) e **transcreve tudo o que foi falado** em vídeos, áudios e gravações, entregando o **texto**. A mídia é descartada depois, salvo se o usuário pedir para guardar o áudio. Este documento reúne as decisões de produto e de arquitetura, as fases de desenvolvimento com checklists, a política de testes, os riscos e o registro de decisões.

Última atualização: 2026-10-21.

## Status das fases

| Fase | Tema | Status |
| --- | --- | --- |
| 0 | Fundação e spikes de risco | Concluída |
| 1 | CI/CD no GitHub | Concluída |
| 2 | Esqueleto do app e design system | Concluída |
| 3 | Fila de jobs e upload | Concluída |
| 3.5 | Build nativo mais rápido | Concluída |
| 4 | Transcrição de arquivos (MVP) | Concluída |
| 5 | Guardar áudio | Concluída |
| 6 | Gravação ao vivo | Concluída |
| 7 | Robustez, empacotamento e entrega final | Em andamento |

Valores possíveis: Não iniciada, Em andamento, Concluída.

## Como vamos trabalhar

- **Uma fase por vez, separadamente.** Cada fase começa com um plano detalhado próprio, aprovado antes da execução, e só termina quando os critérios "Pronto quando" e os testes passam. Só então a próxima começa.
- **Ao abrir uma fase:** status "Em andamento" na tabela acima.
- **Ao fechar uma fase:**
  - Marcar os checklists.
  - Mudar o status para "Concluída".
  - Registrar as decisões e os números medidos no [Registro de decisões](#registro-de-decisões).
  - Atualizar o aviso de estado do `README.md` (o que o app já faz).
- **A partir da Fase 1, uma branch e um PR por fase**, com o CI verde antes do merge. Quando o plano da fase a divide em etapas, cada etapa tem a própria branch e o próprio PR, em sequência.
- **Definition of Done:** veja [Qualidade e testes](#qualidade-e-testes).
- Se um spike ou um teste derrubar uma decisão (por exemplo, se o exe único se mostrar inviável), este roadmap é revisado antes de seguir.

## Visão e escopo

Decisões de produto:

- **Objetivo:** gravar e transcrever. A mídia é descartável: depois do texto, gravações e áudios são lixo, a menos que o usuário peça para guardar o áudio.
- **Fluxo padrão:** receber um arquivo ou gravar → extrair o áudio → transcrever → perguntar "Guardar o áudio?". "Descartar" apaga o temporário na hora; "Guardar áudio" gera um `.m4a` (AAC mono) e abre a janela "Salvar como" do Windows para o usuário escolher a pasta. Sem resposta, o temporário espera até a resposta ou a remoção do job, por no máximo 24 h (o card passa a dizer "O áudio foi descartado depois de 24 h sem resposta."), e é descartado ao fechar o app. O arquivo original enviado nunca é alterado.
- **Saída:** a transcrição de tudo que foi falado, venha de áudio, de vídeo ou da gravação ao vivo, em `.txt` e `.md`. O vídeo **não** recebe legendas.
- **Uso não comercial:** o libx264 (GPL) é permitido.
- **Modelo de IA invisível:** o usuário nunca vê, escolhe nem baixa o modelo. Ele vem embutido no executável e funciona offline.
- **Código no GitHub, com CI/CD.**
- **Toda funcionalidade com testes automatizados.**

O que fica de fora da v1 está no [Backlog](#backlog-fora-da-v1).

## Ambiente de referência

Levantamento de 2026-10-16 da máquina de desenvolvimento:

- Windows 11 Pro pt-BR, i7-13650HX (14 núcleos), 16 GB de RAM, RTX 3050 6 GB (drivers Vulkan e CUDA), 220 GB livres em disco.
- Voz SAPI "Microsoft Maria" (pt-BR), usada para gerar as fixtures de teste.
- Já instalados: Git, winget e Node.
- Instalados na Fase 0 por `scripts/setup.ps1`: Rust, VS Build Tools (C++), LLVM, CMake, vcpkg, FFmpeg (CLI, só para fixtures) e Vulkan SDK.
- GPUs Vulkan: dispositivo 0 = Intel integrada, dispositivo 1 = RTX 3050.

## Análise do PRD original

O PRD original (codinome "MediaPipe Core") propõe um monólito em Rust: Axum + HTMX, whisper.cpp e FFmpeg via FFI e captura de áudio com cpal, em 5 sprints.

### O que foi mantido

- Monólito Rust, sem Electron nem Python.
- whisper.cpp e FFmpeg via bindings, sem chamar processos externos.
- Fila em memória com canal Tokio.
- UI servida pelo próprio binário e aberta no navegador.

### O que foi ajustado

1. **Saída:** sai a legenda embutida no vídeo (Tarefa 3.2) e entra o `transcricao.txt`. Isso elimina libass e o mux de legenda.
2. **Tailwind e HTMX via CDN contradiziam "100% contido no executável".** O CDN exige internet, e o do Tailwind não é para produção. → CSS compilado com o Tailwind CLI standalone; CSS, htmx e fonte embutidos no binário. Funciona offline.
3. **O maior risco é o build nativo no Windows, não a UI** (FFmpeg estático, whisper.cpp e bindgen). → **Fase 0** com spikes antes de qualquer tela.
4. **Fatias verticais.** O PRD só fechava o ciclo (status e download) na Sprint 4. → Cada fase entrega um fluxo ponta a ponta. A primeira entrega de valor é arquivo → texto.
5. **Estados do job.** PENDING/PROCESSING/DONE não bastam. → Etapa, % de progresso, FALHOU e CANCELADO.
6. **Upload.** O Axum limita o corpo a 2 MB por padrão, e "processar pela memória" um vídeo de vários GB estoura a RAM. → Upload em streaming direto para o disco.
7. **Whisper e FFmpeg bloqueiam CPU e GPU.** → Worker em thread dedicada, fora do runtime Tokio. Modelo carregado uma vez. Callbacks de progresso e cancelamento (existem no whisper-rs 0.16).
8. **Gravação.** Mixar em tempo real fontes com relógios diferentes é frágil, e o loopback WASAPI **não entrega dados durante silêncio**. → **Duas trilhas** (mic e sistema) gravadas no disco, silêncio tocando na saída (técnica do OBS), mixagem só no fim. Bônus: transcrição rotulada **"Você" / "Outros"**.
9. **Segurança.** Um servidor local que liga o microfone pode ser acionado por qualquer site aberto no navegador (CSRF ou DNS rebinding). → Bind em 127.0.0.1, token de sessão, cookie SameSite=Strict e validação de Host/Origin.
10. **Alucinações do Whisper no silêncio.** Em pt-BR aparecem frases como "Legendas pela comunidade Amara.org" e "Obrigado por assistir". → VAD Silero, idioma fixo `pt`, `no_context` e filtro de frases-fantasma.
11. **Ciclo de vida.** O app precisa de ícone na bandeja (Abrir/Sair), instância única e encerramento limpo.
12. **Nome.** "MediaPipe" é um produto do Google. → **Scriba**. O arquivo de design é o `DESIGN-scriba.md`, e não `design.md`.
13. **Qualidade.** O PRD não tratava de testes nem de CI. → Testabilidade desde o desenho e CI/CD na Fase 1, antes de qualquer funcionalidade.

### Ordem das fases

- A transcrição vem antes da gravação, porque a gravação depende dela.
- "Guardar áudio" vem logo depois da transcrição e antes da gravação: é o fim do fluxo dos envios, e a gravação reaproveita a mesma pergunta e o mesmo encoder AAC.

## Decisões de arquitetura

| Tema | Decisão | Consequência |
| --- | --- | --- |
| Plataforma | Windows 10/11 x64 na v1 | Loopback WASAPI suportado pelo cpal. macOS/Linux ficam no backlog |
| Transcrição | `whisper-rs` 0.16 com o modelo **large-v3-turbo q5_0 (~0,6 GB) embutido no .exe**, carregado da memória (`WhisperContext::new_from_buffer_with_params`) | Invisível e offline. O .exe fica com ~0,65 GB |
| Aceleração | Backend **Vulkan** (o `vulkan-1.dll` vem com o driver da GPU), com fallback para CPU | CUDA descartado: exigiria DLLs de centenas de MB ao lado do exe |
| Mídia | `ffmpeg-next` 9 + FFmpeg 9 **estático** via vcpkg com baseline fixado no repo: `ffmpeg[core,avcodec,avformat,swresample,swscale,x264,gpl,nvcodec]` | Sem DLLs do FFmpeg. O binário distribuído é GPL (veja a Fase 1) |
| Captura | `cpal` (WASAPI): mic e loopback em trilhas separadas, WAV mono 16-bit a 32 kHz gravado no disco (`<data-dir>/gravacoes/<id>/`) | À prova de crash: uma gravação interrompida volta ao abrir o app. Ocupa ~230 MB/h por trilha; as trilhas ficam até a resposta a "Guardar o áudio?" |
| Web | `axum` 0.8 + `askama` + htmx 2 + Tailwind v4 compilado + fonte Inter (substituta do SF Pro indicada pelo design doc), tudo embutido | Nenhum arquivo solto. Funciona offline |
| Estado | Fila em memória (`Arc<RwLock<HashMap>>` + `mpsc`), mais um manifesto `job.json` por saída | O histórico sobrevive a reinícios sem precisar de banco |
| Saídas | `Documentos\Scriba\AAAA-MM-DD HHmm - nome\` (`transcricao.txt`, `transcricao.md`, `audio.m4a`/`video.mp4`). Temporários em `%LOCALAPPDATA%\Scriba` | Todos os caminhos são configuráveis, o que deixa os testes isolados |
| Guardar áudio | No fim de cada transcrição, "Guardar o áudio?": AAC `.m4a` mono (64 kbps, até 32 kHz) recodificado do original, gravado onde o usuário escolher no "Salvar como" do Windows; senão o temporário é apagado (ao descartar, ao fechar o app ou depois de 24 h sem resposta) | Mídia descartável; sem compactação de vídeo. O x264 continua no FFmpeg só pelo `--self-test` (avaliar tirar) |
| Testabilidade | Traits `Pipeline` (Fase 3), `Opener` e `SaveDialog` (o Explorer e o "Salvar como", Fases 4–5), `Transcriber`, `MediaEngine`, `AudioSource` e `Clock`. Feature `test-support` com fakes e as flags `--fake-pipeline` e `--fake-audio`, fora do release | Toda a lógica roda no CI, sem placa de som, GPU nem janela do Windows |

## Arquitetura

```text
Navegador (HTMX)  ⇄  Axum @127.0.0.1 [token + Host/Origin]
                        │ upload em streaming → %LOCALAPPDATA%\Scriba\uploads
                        ▼
                 JobStore (HashMap em memória) ──mpsc──▶ Worker (thread dedicada, 1 job por vez)
                        ▲                                   ├─ media::extract_audio  (FFmpeg → 16 kHz mono f32, num temporário em disco)
                        │                                   ├─ asr::transcribe       (whisper + VAD em blocos de ~30 min, progresso/cancelar)
                        │                                   ├─ transcript::write     (.txt e .md com [hh:mm:ss] e Você/Outros)
                        │                                   └─ media::encode_audio   (.m4a, só se o usuário pedir)
                 Recorder (AudioSource: cpal mic + loopback + silêncio na saída | fake nos testes)
                   └─ ring buffers → thread de escrita → mic.wav / sistema.wav ──(Parar)──▶ novo job
```

Estrutura prevista (workspace Cargo):

```text
src/                     main.rs (startup, instância única, bandeja), web/, jobs/, media/, asr/, capture/, transcript/
crates/scriba-model/     modelo + VAD via include_bytes! (feature `embedded-model`, usada só no release)
templates/ ui/ assets/   askama, fonte do Tailwind + tokens gerados do DESIGN-scriba.md, assets embutidos
xtask/                   cargo xtask css | model | fixtures | test-local | sandbox-test | dist
tests/, tests/fixtures/  integração Rust; fixtures pequenas versionadas (geradas com a voz Maria)
e2e/                     Playwright (Node, só dev/CI): fluxos no navegador contra o binário real
.github/                 workflows ci.yml e release.yml, ação composta setup-native, dependabot, template de PR
vcpkg.json, deny.toml, scripts/setup.ps1 (ambiente local idempotente)
```

Em desenvolvimento, o modelo é carregado de `models/`, baixado por `cargo xtask model`, para o build continuar rápido. Só o release o embute.

## Qualidade e testes

Vale para todas as fases.

- **Definition of Done:** uma funcionalidade só está pronta com testes automatizados no nível certo e com o CI verde. Todo bug corrigido ganha um teste de regressão. O template de PR traz esse checklist.
- **Camadas de teste:**
  - **Unitários** (lógica pura) rodam no CI.
  - **Integração** roda no CI:
    - Router via `tower::oneshot`.
    - FFmpeg e Whisper com fixtures reais e um modelo pequeno (`base` q5_1).
    - Captura com `AudioSource` falso, com lacunas, jitter e erro de dispositivo.
  - **E2E** roda no CI: Playwright contra o binário real, com diretórios temporários e `--fake-audio`. Inclui comparação visual das telas (fidelidade ao design), com a referência gerada no próprio CI.
  - **Hardware** roda em `cargo xtask test-local`, automatizado e obrigatório antes de cada release, porque os runners do GitHub não têm placa de som nem GPU. Cobre mic e loopback reais, Vulkan, NVENC, desempenho com 1 h de áudio e o exe no Windows Sandbox (`cargo xtask sandbox-test`).
- **Ferramentas:**
  - `cargo nextest` (relatório JUnit no PR).
  - `cargo llvm-cov` com piso de cobertura: começa em 70% e sobe a cada fase.
  - `insta` (snapshots de texto e HTML) e `proptest` (alinhamento de trilhas, mescla Você/Outros).
- **Fixtures:** pequenas e versionadas em `tests/fixtures/`, regeneráveis com `cargo xtask fixtures`. A voz Maria lê textos conhecidos, e os testes verificam palavras-chave. Os runners não têm voz pt-BR, por isso os arquivos ficam no repo.

## Fases

### Fase 0 — Fundação e spikes de risco

Objetivo: provar, antes de qualquer tela, que o build nativo (FFmpeg estático, whisper.cpp e captura WASAPI) funciona nesta máquina e cabe em um único exe.

**Entregas:**

- [x] `scripts/setup.ps1` (idempotente) instala via winget: Rustup (MSVC), VS 2022 Build Tools (C++), LLVM, CMake e Vulkan SDK.
- [x] vcpkg com baseline fixado no repositório e FFmpeg estático compilado (~30–60 min).
- [x] Variáveis de ambiente do build em `.cargo/config.toml [env]`.
- [x] `git init`, workspace Cargo e `rust-toolchain.toml`.
- [x] `cargo xtask model`: baixa os modelos e confere o SHA-256.
- [x] `cargo xtask fixtures`: gera as fixtures de fala pt-BR com a voz Maria.
- [x] **Spike A, whisper:** transcrever uma fixture pt-BR. Medir CPU contra Vulkan e testar `new_from_buffer_with_params`.
- [x] **Spike B, FFmpeg:** extrair o áudio de um MP4 para 16 kHz mono e codificar H.264 com x264 e com NVENC. `dumpbin /dependents` não deve listar DLLs do FFmpeg.
- [x] **Spike C, captura:** gravar 30 s de mic + loopback, com 10 s de silêncio no meio. Os dois WAVs devem ter a mesma duração (±100 ms).
- [x] **Spike D, exe único:** crate do modelo com `include_bytes!`. Medir build, tamanho e carga. Tentar CRT estático.

**Testes:**

- [x] Cada spike vira um teste de integração. Os que dependem de hardware entram na suíte local.

**Pronto quando:**

- [x] Os 4 spikes passam nesta máquina.
- [x] As versões fixadas e as decisões estão no [Registro de decisões](#registro-de-decisões).

Observação: as instalações pedem confirmação de administrador (UAC). Na prática foram ~15–25 GB entre ferramentas, FFmpeg compilado, modelos (~0,6 GB) e builds; `setup.ps1 -LimparVcpkg` apaga os temporários do vcpkg.

### Fase 1 — CI/CD no GitHub

Objetivo: a partir daqui, todo PR passa por verificação automática e toda tag gera uma release testada.

**Entregas:**

- [x] Repositório no GitHub, com nome e visibilidade definidos pelo mantenedor.
- [x] Proteção da `main` exigindo CI verde.
- [x] Template de PR com o checklist de testes.
- [x] Dependabot para cargo, npm (`e2e/`) e GitHub Actions.
- [x] `ci.yml` (push e PR, com `concurrency` cancelando execuções antigas):
  - Job `checks` (windows-latest): `fmt --check`, `clippy -D warnings`, nextest (unit + integração), Playwright E2E e cobertura com piso.
  - Job `deny` (ubuntu, mais barato): `cargo deny` para vulnerabilidades (RustSec) e licenças compatíveis com GPL.
- [x] Ação composta `setup-native`:
  - Rust fixado.
  - vcpkg com cache de binários (`VCPKG_BINARY_SOURCES` + `actions/cache`).
  - `Swatinem/rust-cache`.
  - Cache do modelo de teste por SHA-256.
- [x] `release.yml` (tag `v*`):
  - Confere se a tag bate com a versão do Cargo e faz o build de release.
  - **Smoke test** do exe (`scriba.exe --self-test`: carrega o modelo, transcreve uma amostra embutida e codifica um clipe curto).
  - Gera o SHA-256 e publica a GitHub Release com notas automáticas e `THIRD_PARTY_LICENSES` (`cargo about`).
- [x] `LICENSE` GPL-3.0, exigida ao distribuir o exe com x264.

**Pronto quando:**

- [x] Um PR com teste quebrado fica bloqueado.
- [x] Um PR verde roda em menos de 20 min com cache. A 1ª execução compila o FFmpeg e leva ~1 h.
- [x] A tag `v0.1.0` publica uma release com o exe do esqueleto e o checksum.

Observação: em repositório privado, os minutos do GitHub Actions têm cota, e os runners Windows consomem mais. Cache, `concurrency` e filtros de caminho reduzem esse consumo.

### Fase 2 — Esqueleto do app e design system

Base no PRD: Sprint 1. Objetivo: o exe sobe o servidor local seguro e abre no navegador a tela base, fiel ao design.

**Entregas:**

- [x] Axum em 127.0.0.1 (porta preferida e fallback), com encerramento limpo.
- [x] Middleware de segurança: token na URL → cookie SameSite=Strict; validação de Host e Origin.
- [x] Assets embutidos (CSS, htmx, Inter, ícones) com cache.
- [x] Layout askama (`app-header`, painel de gravação e dropzone, "Recentes", `footer`) seguindo o `DESIGN-scriba.md`.
- [x] Logo vetorizado a partir de `docs/brand/logo-original.jpg`: SVG do ícone, versão branca e versão simplificada de 16 px (favicon e bandeja).
- [x] `cargo xtask css`: gera os tokens a partir do YAML (tema claro em `:root`, escuro em `[data-theme="dark"]` e em `prefers-color-scheme`) e compila o Tailwind.
- [x] Alternância de tema no header (Sistema → Claro → Escuro), salva na configuração e renderizada pelo servidor no `<html>`, sem piscar.
- [x] Instância única via `instance.json`.
- [x] Flags `--no-open`, `--port`, `--data-dir` e `--output-dir`.

**Testes:**

- [x] Unitários: token, Host e Origin; fallback de porta; lockfile; gerador de tokens CSS (snapshot).
- [x] Integração: rotas com 200/401/403; content-type e cache dos assets; segunda instância.
- [x] E2E: a página carrega e bate com a referência visual nos dois temas; a alternância de tema persiste após recarregar.
- [x] Unitário: contraste AA de todo par texto/fundo dos componentes, nos dois temas, calculado a partir do YAML.

**Pronto quando:**

- [x] `cargo run` abre a tela fiel ao design.
- [x] Requisição sem token retorna 401, e Host estranho retorna 403.
- [x] A segunda execução reaproveita a instância aberta.

### Fase 3 — Fila de jobs e upload

Base no PRD: Sprint 2 e o polling da Sprint 4. Objetivo: arquivos entram por upload, viram jobs em uma fila em memória e a UI acompanha o andamento, ainda com um pipeline falso.

**Entregas:**

- [x] Modelo de job: id ULID, tipo, opções (transcrever/compactar), etapa, %, erro e saídas.
- [x] Worker em `std::thread` com `blocking_recv`, `catch_unwind` por job e cancelamento por `AtomicBool`.
- [x] Upload multipart em streaming para o disco (limite de corpo desativado só nessa rota).
- [x] Rotas `POST /api/jobs`, `GET /api/jobs`, `GET /api/jobs/{id}`, `POST /api/jobs/{id}/cancel` e `DELETE /api/jobs/{id}`.
- [x] Polling de 1 s só para jobs ativos; a resposta 286 encerra o polling.
- [x] Pipeline falso atrás do trait, para validar a UI antes dos motores.

**Testes:**

- [x] Unitários: máquina de estados (proptest) e semântica de cancelamento.
- [x] Integração: upload grande gravado no disco em streaming, sem acumular na memória; panic no pipeline vira FALHOU.
- [x] E2E: upload → progresso → concluído; cancelar; falha exibida.

**Pronto quando:**

- [x] Um upload de 2 GB mantém a RAM estável.
- [x] Progresso, cancelamento e falha aparecem corretos na UI.

### Fase 3.5 — Build nativo mais rápido

Objetivo: cortar o build frio do FFmpeg quase pela metade, no CI e na máquina de desenvolvimento, e manter o cache do CI vivo.

**Contexto:** os dois builds frios de 2026-10-19, com a mesma configuração e runners diferentes, mediram onde vai o tempo do passo `setup-native`:

| Etapa | PR #11 | `main` |
| --- | --- | --- |
| Clone e bootstrap do vcpkg | ~40 s | ~42 s |
| x264, debug + release | 3,8 min | 2,4 min |
| FFmpeg, versão release | 11,3 min | 7,1 min |
| FFmpeg, versão debug | 11,3 min | 6,8 min |
| `setup-native` total | 29,5 min | 18,9 min |

A versão debug, cerca de 38% do passo, nunca é usada: o Rust linka sempre o CRT de release, e o `ffmpeg-sys-next`, com `FFMPEG_DIR`, procura as libs só em `<dir>/lib`. As libs debug ocupam 211 dos 430 MB do `vcpkg_installed` local. O cache também é frágil: o GitHub apaga um cache que fica 7 dias sem acesso, e a `main` não lê caches criados em PR, então mudar a chave custa um build frio na branch e outro na `main`.

**Entregas:**

- [x] Triplet `x64-windows-static-release`, versionado como overlay em `triplets/`, no lugar do `x64-windows-static`. Atualiza `.cargo/config.toml` (`FFMPEG_DIR`, `VCPKGRS_TRIPLET`), a ação `setup-native`, o README e o `CLAUDE.md`. Quem já tem o ambiente reinstala o FFmpeg uma vez.
- [x] Chave do cache do FFmpeg com o nome do triplet e o hash de `vcpkg.json` e `triplets/`.
- [x] CI semanal agendado na `main` (`schedule` no `ci.yml`). Ele mantém todos os caches vivos (FFmpeg, Rust, modelo, npm) e ainda pega quebras vindas de atualizações do runner e avisos novos do RustSec.
- [x] O `CLAUDE.md` passa a explicar o custo de mudar o `vcpkg.json` ou o triplet: um build frio na branch e outro na `main`.

**Testes:**

- [x] CI verde com cache frio (a chave nova força o build) e o tempo medido.
- [x] Localmente: instalar com o novo triplet, rodar `cargo test --workspace --features spikes/vulkan` e o `scriba --self-test` do build de release, e conferir no `dumpbin /dependents` que o exe não depende de DLLs do FFmpeg.

**Pronto quando:**

- [x] O log do vcpkg no CI não tem mais build de debug, e o build frio cai ~40%. Hoje leva de 19 a 30 min, conforme o runner; a estimativa é de 11 a 17 min.
- [x] O cache do FFmpeg cai de 77 MB para ~40 MB. Medido: 61 MB. A estimativa partia do tamanho em disco, que caiu pela metade (de 428 para 217 MB), mas no cache comprimido as libs debug pesavam menos.
- [x] O CI agendado roda na `main` sem erro.

### Fase 4 — Transcrição de arquivos (MVP)

Base no PRD: Sprint 3.1 e Sprint 4. Objetivo: qualquer áudio ou vídeo vira texto em pt-BR, em `.txt` e `.md`. É a primeira entrega de valor.

O plano da fase a divide em seis etapas, cada uma com o próprio PR: 4.1 whisper otimizado no release e investigação da CPU; 4.2 `media` e `transcript`; 4.3 `asr` e benchmark small × base; 4.4 pipeline real no app; 4.5 UI do resultado; 4.6 release, medições de 1 h e fechamento.

**Entregas:**

- [x] `media::probe` e `media::extract_audio`: duração, taxa de bits e resolução do original; melhor faixa de áudio → 16 kHz mono f32, sem decodificar o vídeo, com progresso por pts e erros amigáveis (ex.: "arquivo sem áudio").
- [x] `asr` (etapa 4.3; entra no app na 4.4):
  - Contexto carregado sob demanda e liberado após 10 min ocioso.
  - Idioma `pt`, VAD Silero e `no_context`.
  - Threads = núcleos − 2, salvo se a investigação da CPU mostrar uma regra melhor.
  - Progresso, cancelamento e filtro de frases-fantasma.
- [x] Escolha do backend (4.3: escolha e benchmark; 4.4: backend no rodapé e estimativa no card) (achado do spike A: só CPU com o modelo grande leva ~8 h para 1 h de áudio):
  - Usar a GPU **dedicada** quando houver, e não o dispositivo Vulkan 0, que costuma ser a integrada. Sem dedicada, a CPU: a integrada ficou de fora porque, no benchmark da 4.3, foi mais lenta que a CPU com todos os modelos.
  - Sem GPU dedicada, transcrever na CPU com um **modelo menor embutido**: o `small` (benchmark da 4.3: 8,7% de WER contra 15,9% do `base`, RTF 0,105). O custo é o exe crescer ~190 MB.
  - Antes de fechar o fallback, investigar a lentidão da CPU (~87 s para 10,9 s), comparando com o whisper.cpp oficial e testando threads e OpenMP. No planejamento, a leitura do `CMakeCache.txt` mostrou que o whisper.cpp do release compila sem `/O2` (veja o Registro de decisões).
  - Mostrar na UI qual backend está em uso e uma estimativa de tempo antes de começar.
- [x] `transcript`: parágrafos (pausa > 1,5 s quebra), `[hh:mm:ss]` em cada parágrafo e cabeçalho (origem, data, duração). Gera `transcricao.txt`, `transcricao.md` e `job.json`.
- [x] Fluxo automático: o envio deixa de ter as opções "Transcrever" e "Compactar". Todo job extrai o áudio e transcreve; a pergunta "Guardar o áudio?" entra na Fase 5.
- [x] UI: leitor da transcrição, Copiar texto, Baixar .txt, Baixar .md e Abrir pasta (etapa 4.5).
- [x] Reuniões longas (etapa 4.6): o áudio extraído vai para um temporário em disco (`uploads/<id>.f32`) e a transcrição lê blocos de ~30 min, cortados no meio de uma pausa do VAD perto do fim do bloco; os tempos voltam ao original, e o parágrafo na emenda quebra como sem blocos. A RAM acompanha o bloco, não a duração.
- [x] Release (etapa 4.6): o exe do CI embute o modelo da CPU (`small`), o VAD e a amostra do self-test (`embedded-cpu-model`) e transcreve no processador; a GPU e o large entram na release completa da Fase 7.
- [x] Herdado da Fase 3 (etapa 4.4):
  - O pipeline real substitui o falso no app; o falso vai para a feature `test-support`, com a flag `--fake-pipeline` para o E2E.
  - "Tentar de novo" no job que falhou (o envio fica guardado até a nova tentativa).
  - Remover um job que tem saídas passa pelo `confirm-dialog`, e a pasta vai para a Lixeira.
  - Card com duração e data (probe do FFmpeg e hora local, a mesma da pasta `AAAA-MM-DD HHmm`).

**Testes:**

- [x] Unitários: parágrafos, formato de tempo, filtro de frases-fantasma, cabeçalho e o `.md` (snapshots).
- [x] Integração com o modelo pequeno:
  - Fixtures MP4, MP3, M4A, OGG e WAV → palavras-chave.
  - Silêncio puro → texto vazio, sem frases-fantasma.
  - Cancelar no meio interrompe a transcrição.
  - Arquivo sem áudio → erro amigável.
- [x] E2E: upload → texto visível → copiar → .txt e .md baixados com o conteúdo certo.
- [x] Unitário: a escolha do backend prefere GPU dedicada (a de mais memória), depois CPU com o modelo menor; a integrada fica de fora (com uma lista de dispositivos falsa).
- [x] Local: benchmark em cada backend (RTX, Intel integrada e só CPU), com 45,5 min de fala real do FLEURS pt_br no lugar de 1 h (etapa 4.3).
- [x] Local: pico de RAM e tempo com 1 h e 3 h de áudio, com a transcrição em blocos de ~30 min (meta: ≤ ~1,5 GB em qualquer duração): pico máximo de 680 MB na RTX e 1.139 MB na CPU, em 1 h e em 3 h 13 min (`uma_hora` e `tres_horas` em `tests/engine.rs`). (O item "compactar antes acelera?" saiu com a compactação.)

**Pronto quando:**

- [x] Arquivos reais (MP4, MP3, M4A, WAV, OGG) viram texto pt-BR correto, em `.txt` e `.md`: trechos de 10 min de uma audiência pública da Câmara, pelo exe de release.
- [x] O tempo para transcrever 1 h de áudio na RTX 3050 está registrado: 2 min 9 s.
- [x] Metas para 1 h de áudio: ≤ 10 min na RTX 3050 e ≤ 1 h só na CPU (com o modelo menor). Os tempos de 1 h nos dois backends da lista (RTX e só CPU com o `small`) ficam no registro de decisões; a integrada, fora da lista desde a 4.3, fica com os números do benchmark.

### Fase 5 — Guardar áudio

Objetivo: no fim de cada transcrição, o usuário decide se o áudio vira lixo ou se é guardado num `.m4a`, na pasta que ele escolher. A compactação de vídeo saiu do produto (decisão de 2026-10-19).

O plano da fase a divide em duas etapas, cada uma com o próprio PR: 5.1 `media::encode_audio` (o motor, ainda fora do app) e 5.2 a pergunta no app (estado, "Salvar como", rotas, card, design e E2E).

**Entregas:**

- [x] `media::encode_audio` (etapa 5.1): o áudio do original → `.m4a` AAC-LC mono a 64 kbps, na taxa do original até 32 kHz, com progresso e cancelamento. Um vídeo vira só o som. O mix da gravação entrou na etapa 6.2 (`media::encode_mix`).
- [x] Card concluído com "Guardar o áudio?": "Descartar" (apaga o temporário) e "Guardar áudio"; depois, "Áudio guardado em …" e "Mostrar no Explorer" (o arquivo selecionado na pasta).
- [x] "Guardar áudio" abre a janela "Salvar como" nativa do Windows pelo exe, na frente do navegador (lembra a última pasta), e grava o `.m4a` lá, com progresso e cancelar no card; cancelar a janela mantém a pergunta, e um erro vira toast.
- [x] O temporário fica até a resposta ou a remoção do job e é apagado ao fechar o app e ao iniciar (sobras).

**Testes:**

- [x] Integração: os formatos das fixtures saem AAC mono com a duração do original (±0,1 s); cancelar interrompe. Também o `Engine` real guarda o `.m4a` de um vídeo concluído.
- [x] Rotas: descartar apaga o temporário; guardar com um diálogo falso grava no caminho escolhido; diálogo cancelado não apaga nada.
- [x] E2E: transcrição → "Descartar" e "Guardar áudio" (diálogo falso com `--fake-pipeline`), cancelar a gravação, erro em toast e a referência visual.
- [x] Local: a janela real abre em primeiro plano e topmost, com outra janela na frente (`#[ignore = "desktop"]`).

**Pronto quando:**

- [x] Uma gravação de 1 h do Teams vira texto e, se pedido, um `.m4a` na pasta escolhida que abre no Media Player. Medido com o MP4 de 1 h da audiência da Câmara (H.264 720p + AAC, como uma gravação do Teams), pelo exe de release.
- [x] O mesmo `.m4a` toca no celular. Sem aparelho, conferido por compatibilidade:
  - o formato é o que Android e iOS tocam (AAC-LC mono a 32 kHz, contêiner MPEG-4 com a marca `M4A ` e o índice no começo);
  - um decodificador fora do FFmpeg do Scriba, o `<audio>` do Edge, toca o arquivo de 1 h do começo ao fim.

  O teste num celular de verdade fica para a primeira versão de uso.

### Fase 6 — Gravação ao vivo

Base no PRD: Sprint 5. Objetivo: um clique grava o microfone e o som do PC; ao parar, sai a transcrição rotulada "Você" / "Outros", e as trilhas são descartadas, salvo se o usuário guardar o áudio.

O plano da fase a divide em quatro etapas, cada uma com o próprio PR: 6.1 captura e `Recorder` (ainda fora do app); 6.2 job de gravação e transcrição rotulada; 6.3 UI e rotas; 6.4 "Pronto quando" no hardware e fechamento.

**Entregas:**

- [x] Lista de dispositivos (padrões marcados), com escolha nos ajustes (etapa 6.3: o `<details>` "Ajustes" do painel, salvo no `config.json`, com o aviso de microfone Bluetooth).
- [x] Recorder com três streams (etapa 6.1, `src/capture/`):
  - Mic.
  - Loopback: stream de entrada aberto no dispositivo de saída com `default_output_config`.
  - Silêncio tocando na mesma saída.

  Os callbacks só copiam para ring buffers SPSC (`rtrb`).
- [x] Thread de escrita (etapa 6.1): downmix para mono, reamostragem para 32 kHz, conversão para i16, WAV via `hound` com `flush` a cada 5 s. Alinhamento por timestamp. Medidores RMS.
- [x] Rotas `POST /api/record/start`, `POST /api/record/stop` e `GET /api/record/status` (etapa 6.3, `src/web/record.rs`), mais o SSE `GET /api/record/levels` e os Ajustes `GET`/`POST /api/record/devices`.
- [x] UI: tile escuro com botão grande e barra fixa "● Gravando 00:12:34 · Parar" (etapa 6.3: `record-panel` com os medidores, `recording-bar` em todas as páginas, card de gravação, rótulo no leitor e Ctrl+Shift+R).
- [x] Job de gravação (etapa 6.2: `Media::Recording`, `Job::recording`, `Jobs::submit_recording`):
  - Mixdown só se o usuário escolher "Guardar áudio", com o `media::encode_mix` (as duas trilhas somadas, entregues ao mesmo encoder AAC da Fase 5, sem arquivo temporário).
  - Transcrição de cada trilha, mesclada por tempo com os rótulos Você/Outros (`transcript::merge`).
  - Remoção do eco e geração do `transcricao.txt` e do `transcricao.md`.
  - As trilhas ficam até a resposta a "Guardar o áudio?", como o envio (Fase 5): Descartar, Guardar áudio, Remover ou fechar o app com a pergunta pendente as apagam; uma gravação ainda não transcrita (Ctrl+C, crash) é retomada ao abrir o app (`app::resume_recordings`).
- [x] Robustez (etapa 6.1, no `scriba-gravacao`; o Ctrl+C real conferido na 6.4):
  - Dispositivo removido: salva o que foi gravado.
  - Troca da saída padrão: reabre o loopback.
  - Checagem de espaço em disco e teto de 12 h (o limite de 4 GB do WAV a 32 kHz seria ~18 h).

**Testes:**

- [x] Unitários: downmix e i16; alinhamento e preenchimento; mescla e dedupe de eco (proptest); teto de duração (etapas 6.1 e 6.2).
- [x] Integração com `AudioSource` falso (as rotas da etapa 6.3 também, em `tests/web.rs`):
  - [x] Lacunas e jitter → trilhas alinhadas (±10 ms) (etapa 6.1, com deriva de +500 ppm e taxas diferentes).
  - [x] Erro de dispositivo → arquivo finalizado (etapa 6.1; a saída invalidada é reaberta).
  - [x] Fixtures "Você"/"Outros" + eco → texto rotulado e sem duplicatas (etapa 6.2, `tests/engine.rs`).
- [x] E2E com `--fake-audio`: Gravar → timer → Parar → transcrição rotulada (etapa 6.3, `e2e/tests/gravacao.spec.ts`, com o recarregar no meio, o fim automático, o exe morto e reaberto, a confirmação do descarte e os Ajustes).
- [x] Local: mic e loopback reais com 15 s de silêncio no meio (etapa 6.1, `tests/capture.rs`; também gravando com a CPU ocupada por uma transcrição e por 1 h).

**Pronto quando:**

- [x] Falar com música ou reunião tocando, clicar em Parar e receber o texto com as duas vozes rotuladas (etapa 6.4, exe de release: a Câmara nos alto-falantes e a "Você" pelo método híbrido; uma pessoa falando de verdade fica para a 1ª versão de uso).
- [x] 15 s de silêncio no meio da gravação não desalinham as trilhas (etapa 6.4: os marcadores antes e depois do silêncio diferem ≤ 0,1 ms).

### Fase 7 — Robustez, empacotamento e entrega final

Objetivo: o `scriba.exe` final roda em qualquer Windows sem instalar nada e se comporta como um app de bandeja.

O plano da fase a divide em cinco etapas, cada uma com o próprio PR: 7.1 logs e panic hook; 7.2 app de bandeja (subsistema, encerramento único, instância única, `winresource`); 7.3 histórico, `config.json` v1 e limpeza; 7.4 release completa (`vulkan-1.dll` com delay-load, `--self-test`, `cargo xtask dist`, `release.yml`); 7.5 Windows Sandbox e entrega (README, versão 1.0.0).

**Entregas:**

- [x] `windows_subsystem = "windows"`, ícone na bandeja (Abrir/Sair), ícone e metadados do exe (`winresource`) (etapa 7.2: `src/tray/`, `src/console.rs`, `src/shutdown.rs`, a instância única com mutex nomeado, `build.rs` e `cargo xtask icon`).
- [x] Logs com rotação em `%LOCALAPPDATA%\Scriba\logs` e panic hook (etapa 7.1, `src/logs.rs`).
- [x] Histórico reconstruído a partir dos `job.json`, `config.json` e limpeza de temporários antigos (etapa 7.3: `src/jobs/history.rs`, `Config` v1, `Clock`, `Audio::Expired` com o zelador e `src/cleanup.rs`).
- [ ] `release.yml` completo: Vulkan SDK no runner, modelo embutido e `--self-test` exercitando também o fallback para CPU (o runner não tem GPU).
- [ ] `cargo xtask dist` faz o mesmo build localmente.
- [ ] README com o uso final (bandeja e release completa; ele existe desde a Fase 3) e `CLAUDE.md` com os comandos reais.

**Testes:**

- [x] Unitários: config (carga, defaults e migração); limpeza por idade com `Clock` falso; ações do menu da bandeja via canal (esta feita na etapa 7.2, com `quit_warning` e o ícone pelo tema; o resto na 7.3).
- [ ] Integração: histórico a partir de `job.json` de fixture (feito na etapa 7.3, `tests/historico.rs`); `--self-test`.
- [ ] Local: `cargo xtask sandbox-test` roda o exe no Windows Sandbox (máquina limpa) e lê o resultado.

**Pronto quando:**

- [ ] A tag gera uma release que passa no smoke test do CI e no Sandbox, com o fluxo completo de enviar ou gravar e transcrever.

## Backlog (fora da v1)

- Transcrição ao vivo durante a gravação.
- Diarização real (vários falantes em "Outros").
- Resumo ou ata via LLM.
- Gravação de tela.
- HEVC/AV1.
- Exportar .srt e .docx.
- Atalho global para gravar.
- macOS/Linux.
- Runner self-hosted na máquina de desenvolvimento para rodar a suíte de hardware no CI (só com repositório privado, por segurança).
- FFmpeg enxuto: um port em overlay só com os decoders, encoders e formatos usados encurta o build e diminui o exe, mas pode recusar arquivos que hoje abrem. Avaliar junto com o tamanho do exe.

## Riscos e mitigações

| Risco | Mitigação |
| --- | --- |
| Build nativo falha (bindgen, CRT misto, FFmpeg) | A Fase 0 vem primeiro, com versões fixadas. Plano B: DLLs do FFmpeg ao lado do exe, distribuído em zip |
| 1ª execução do CI compila o FFmpeg e demora ~1 h | Cache de binários do vcpkg, cache aquecido na `main` e timeout do job ajustado |
| Runners sem placa de som nem GPU | `AudioSource` falso e `--fake-audio` no CI. Hardware coberto pela suíte local obrigatória antes da release |
| Testes E2E instáveis | Porta 0 e diretórios temporários por teste. Esperar eventos do HTMX em vez de `sleep`. Retry só no CI, com relatório |
| `ffmpeg-next` está em modo manutenção | Fixar a 9.x junto com o FFmpeg 9 do vcpkg. Alternativa: `rsmpeg` |
| `include_bytes!` de ~0,6 GB deixa o build lento | Crate separado, compilado só no release. Plano B: modelo anexado ao fim do exe e lido via mmap |
| O exe não abre em máquina sem Vulkan | Confirmado na Fase 0: o exe importa `vulkan-1.dll` diretamente. `--self-test` no runner e no Sandbox. Plano B: carregar o Vulkan com delay-load ou variante `scriba-cpu.exe` |
| Loopback mudo no silêncio ou deriva de relógio | Silêncio tocando na saída, alinhamento por timestamp e testes de lacuna |
| Eco (sem fone de ouvido) duplica falas | Dedupe por sobreposição e similaridade, mais uma dica na UI |
| Site malicioso liga o microfone | Token, SameSite=Strict, Host/Origin e bind em 127.0.0.1, todos com testes de 401/403 |

## Verificação

- **No CI, a cada PR:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo nextest run`, `npx playwright test` (em `e2e/`), `cargo llvm-cov` com piso e `cargo deny check`.
- **Localmente, antes de cada release:** `cargo xtask test-local` (hardware, GPU e desempenho) e `cargo xtask sandbox-test`.
- **Na release:** o smoke test do `scriba.exe --self-test` no runner precisa passar antes da publicação.
- **Em cada fase:** só fecha com o critério "Pronto quando" atendido e o CI verde.

## Registro de decisões

| Data | Fase | Decisão | Motivo |
| --- | --- | --- | --- |
| 2026-10-16 | Planejamento | A saída é a transcrição em texto; o vídeo não recebe legendas | O objetivo é ter o texto de tudo que foi falado |
| 2026-10-16 | Planejamento | Windows 10/11 x64 na v1 | Loopback WASAPI suportado pelo cpal; é o ambiente de uso |
| 2026-10-16 | Planejamento | Modelo large-v3-turbo q5_0 embutido no exe | Modelo invisível ao usuário e funcionamento offline |
| 2026-10-16 | Planejamento | Vulkan em vez de CUDA | Preserva o exe único: o loader do Vulkan vem com o driver |
| 2026-10-16 | Planejamento | FFmpeg estático com x264 (GPL) | Uso não comercial e melhor compressão. Exige `LICENSE` GPL-3.0 ao distribuir |
| 2026-10-16 | Planejamento | Tailwind compilado e assets embutidos, sem CDN | Funcionar offline, com tudo dentro do executável |
| 2026-10-16 | Planejamento | Gravação em duas trilhas, com mixagem só no fim | Robustez do loopback e rótulos Você/Outros |
| 2026-10-16 | Planejamento | CI/CD no GitHub Actions e testes automatizados obrigatórios | O código será hospedado no GitHub; Definition of Done |
| 2026-10-16 | Planejamento | `DESIGN-scriba.md` reescrito como design system de app: Inter embutida, componentes do Scriba, cores de status como exceção ao acento único, marca monocromática | O arquivo original descrevia o site de marketing da Apple; o logo define uma marca preto e branco em traço fino |
| 2026-10-16 | Planejamento | Temas claro e escuro, alternáveis no header; padrão segue o Windows | Pedido do usuário; tokens `themes.dark` no `DESIGN-scriba.md` |
| 2026-10-16 | Fase 0 | Versões fixadas: Rust 1.99.1 (`rust-toolchain.toml`), vcpkg baseline `7d5f3c6` com FFmpeg 9.0.2, `whisper-rs` 0.16.0 (`whisper-rs-sys` 0.15.0), `ffmpeg-next` 9.0.0, `cpal` 0.18.2, `hound` 3.5.1. Ferramentas: VS 2022 Build Tools 17.14, LLVM 23.1.3, CMake 4.4.4, Vulkan SDK 1.4.363.0 | Reprodutibilidade do build |
| 2026-10-16 | Fase 0 | Triplet `x64-windows-static` + `+crt-static` (também `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded` para o whisper.cpp). O exe depende só de DLLs do Windows e de `vulkan-1.dll` | Spike D: com `static-md` o exe exigia o VC++ Redistributable |
| 2026-10-16 | Fase 0 | `build.rs` declara as libs de link do FFmpeg estático: `libx264`, `secur32`, `crypt32`, `ncrypt`, `bcrypt`, `ole32`, `user32`, `mfuuid`, `strmiids`, `mfplat` | Com `FFMPEG_DIR`, o `ffmpeg-sys-next` não as declara |
| 2026-10-16 | Fase 0 | whisper.cpp com `GGML_NATIVE=OFF` e base AVX/AVX2/FMA/F16C | Com MSVC, o `GGML_NATIVE` não detecta a CPU e compila sem SIMD |
| 2026-10-16 | Fase 0 | `CARGO_TARGET_DIR=C:\dev\t\scriba` e vcpkg em `C:\dev\vcpkg` | O build Vulkan do whisper.cpp estoura MAX_PATH (260) dentro do repositório |
| 2026-10-16 | Fase 0 | Spikes em `crates/spikes` (testes de hardware com `#[ignore = "hardware"]`). CLI do FFmpeg (`Gyan.FFmpeg.Essentials`) só para gerar fixtures | Mantém o app limpo e o FFmpeg embutido enxuto |
| 2026-10-16 | Fase 0 | Medições: FFmpeg estático compila em 20,7 min (`static-md`) e 11,7 min (`static`, com cache do x264). Exe com modelo embutido: 604 MB; o modelo (574 MB) carrega da memória em ~2,8 s; rebuild só do crate do modelo, 19 s | Spike D |
| 2026-10-16 | Fase 0 | Medições, large-v3-turbo q5_0, fixture de 10,9 s (release): RTX 3050 via Vulkan ~0,9 s (~12× tempo real); a 1ª execução compila os shaders e leva ~11 s. Intel integrada ~15 s. CPU com 18 threads ~87 s (0,13× tempo real), escalando com as threads | Spike A |
| 2026-10-16 | Fase 0 | Achados para a Fase 4: (1) o Vulkan enumera a GPU integrada como dispositivo 0, então o app precisa escolher a GPU dedicada (tipo discreto); (2) o fallback de CPU com o modelo grande é inviável para áudios longos: investigar threads/OpenMP ou usar modelo menor sem GPU; (3) whisper em testes paralelos disputa a CPU (3 testes base: 498 s), então os testes de whisper devem rodar em série no CI; (4) o silêncio puro gera "[MÚSICA DE FUNDO]" sem VAD | Spike A |
| 2026-10-16 | Fase 0 | Política de backend da Fase 4: GPU dedicada → GPU integrada → CPU com modelo menor embutido (`small` ou `base`, por benchmark). Metas para 1 h de áudio: ≤ 10 min na RTX 3050 e ≤ 1 h na CPU | Estimativa linear do spike A: 1 h de áudio levaria ~5 min na RTX, ~80 min na integrada e ~8 h na CPU com o modelo grande |
| 2026-10-16 | Fase 0 | Medições: extração de áudio de MP4/MP3/M4A/OGG dá 10,93 s, igual ao WAV; x264 e NVENC codificam 2 s de vídeo; o exe de teste não lista DLLs do FFmpeg | Spike B |
| 2026-10-16 | Fase 0 | Medições: mic 29,930 s e loopback 30,010 s em 30 s sem som no PC (diferença de 80 ms, limite de 100 ms), com o stream de silêncio na saída. WASAPI compartilhado entrega f32 48 kHz estéreo | Spike C. A folga é pequena: o alinhamento por timestamp da Fase 5 é necessário |
| 2026-10-17 | Fase 1 | Repositório `filipimr/scriba` público | Proteção da `main` e Actions sem cota no plano Free; o código é GPL |
| 2026-10-17 | Fase 1 | `scriba --self-test` em CPU com o modelo `base` q5_1 e `fala.wav` embutidos (feature `embedded-test-model`); exe de 83 MB. O modelo grande e o Vulkan na release ficam para a Fase 7 | O runner não tem GPU nem Vulkan SDK; prova carga da memória, whisper e x264 |
| 2026-10-17 | Fase 1 | CI sem a feature `vulkan` (clippy e testes); testes de whisper em série via grupo do nextest; piso de cobertura de 70% só nos crates do app (spikes e xtask fora) | Runner sem Vulkan; achado (3) do spike A |
| 2026-10-17 | Fase 1 | Cache do FFmpeg = `vcpkg_installed/` inteiro, chaveado pelo hash do `vcpkg.json`, salvo mesmo se o job falhar. A versão do `vcpkg.json` fica fixa: mudar o arquivo recompila o FFmpeg | Simples e sem servidor de binários |
| 2026-10-17 | Fase 1 | CI roda em todo PR (sem `paths-ignore`); só o push na `main` ignora `.md` e `docs/` | Com checks obrigatórios, um PR só de documentação nunca receberia os checks e ficaria bloqueado |
| 2026-10-17 | Fase 1 | E2E: projeto Playwright em `e2e/` com um teste pulado até a Fase 2 trazer o servidor | O job já existe e roda no CI |
| 2026-10-17 | Fase 1 | Medições no runner windows-latest: sem cache do FFmpeg, 34–36 min (compilação do FFmpeg ~30 min, abaixo da estimativa de 1 h); com cache, `checks` 3 min 48 s e `deny` ~40 s; release sem cache 33 min; o self-test transcreve os 10,9 s da amostra em 48 s em CPU | O cache é por ref: PRs leem o da `main`, aquecido a cada push nela |
| 2026-10-17 | Fase 2 | Versões: `axum` 0.8.9, `askama` 0.16.1, `tokio` 1.53, htmx 2.0.11, Tailwind CLI standalone 4.3.3 (SHA-256 fixado no xtask), Inter 4.1 (variável, woff2), Lucide 1.59.0 (só os ícones usados, inline) | Reprodutibilidade; tudo embutido, sem CDN |
| 2026-10-17 | Fase 2 | `ui/tokens.css` e `assets/app.css` gerados e **versionados**; o CI só confere com `cargo xtask css --check` | O `cargo build` e o CI não dependem do Tailwind; o diff do CSS aparece no PR |
| 2026-10-17 | Fase 2 | Tokens no `@theme static` do Tailwind (todas as cores viram `--color-*`, usadas ou não); escuro em `[data-theme="dark"]` e em `prefers-color-scheme` para `:root:not([data-theme="light"])` | Bate com o "Como funciona" do design; o servidor renderiza o `data-theme`, sem piscar |
| 2026-10-17 | Fase 2 | Porta preferida 47800, com fallback para porta livre. Token de 32 bytes na URL (`?t=`) → cookie `HttpOnly; SameSite=Strict` + redirect; Host precisa ser `127.0.0.1`/`localhost` na porta; Origin obrigatório e igual em métodos não-GET; CSP `default-src 'self'` | Segurança do servidor local (CSRF e DNS rebinding) |
| 2026-10-17 | Fase 2 | Instância única: `instance.json` {id, porta, token} + `GET /health` (sem token, devolve só o id). A 2ª execução reabre o navegador na instância viva e sai | Dispensa checar PID; arquivo velho ou porta reaproveitada por outro programa não enganam |
| 2026-10-17 | Fase 2 | Favicon em SVG (`logo-16.svg`); o `.ico` fica para a Fase 7 (exe e bandeja). No header, `logo-white.svg` (quadrado branco, caneta preta) com traço mais grosso, legível a 24 px | Navegadores atuais aceitam favicon SVG |
| 2026-10-17 | Fase 2 | Abas "Arquivos"/"Gravar" e "Sair" do header ainda não existem: chegam com as telas (Fases 3–5) e a bandeja (Fase 7). Dropzone e botão de gravar aparecem desabilitados | Sem funcionalidade por trás nesta fase |
| 2026-10-17 | Fase 2 | E2E no Edge instalado (`channel: "msedge"`), sem baixar navegador. Referência visual gerada no runner e versionada em `e2e/tests/tela.spec.ts-snapshots/`: o workflow manual `e2e-snapshots.yml` (ou um run que falhe) publica as imagens como artefato para commit. Localmente a comparação visual é pulada | Renderização local e do runner diferem (fontes, DPI) |
| 2026-10-17 | Fase 2 | `about.hbs` credita os assets que não são crates: Inter (OFL 1.1), htmx (0BSD), Lucide (ISC), Tailwind (MIT) | O `cargo about` só enxerga crates |
| 2026-10-17 | Fase 2 | Medição: job `checks` com cache em ~7–9 min (7 min 20 s e 8 min 41 s nas duas execuções do PR; antes ~3 min 50 s); a diferença é o `cargo build` de debug para o E2E, separado do build instrumentado da cobertura | Ainda bem abaixo da meta de 20 min |
| 2026-10-19 | Fase 3 | Versões: `axum` com a feature `multipart` (`multer` 3.1.0) e `tokio` com `fs`, `io-util` e `sync`. Só nos testes: `proptest` 1.12.0, `insta` e `windows-sys` 0.61.2 (pico de memória do processo) | Reprodutibilidade; o exe só ganha o parser de multipart |
| 2026-10-19 | Fase 3 | A API da fila responde fragmentos HTML (htmx), sem JSON. Erro de envio volta com 422 (507 se a gravação falhar), `HX-Retarget` para o alerta da dropzone e `responseHandling` no `htmx-config` liberando a troca. O card ativo faz polling de 1 s trocando o card inteiro; no fim, 286 com o card final | O único cliente é a própria UI; o status HTTP continua certo para os testes |
| 2026-10-19 | Fase 3 | `JobId` no formato ULID gerado no próprio código (`getrandom`), monotônico no mesmo milissegundo; store em `RwLock<BTreeMap>` em vez do `HashMap` previsto | Sem crate novo; "Recentes" sai na ordem de criação pelo próprio id |
| 2026-10-19 | Fase 3 | Barra de progresso no `<progress>` nativo; o comportamento fica no `app.js`, sem `hx-on` | A CSP (`default-src 'self'`) bloqueia `style` inline e `eval`; o `<progress>` já é acessível |
| 2026-10-19 | Fase 3 | O `FakePipeline` é o pipeline do app até a Fase 4, com ganchos pelo nome do arquivo: `lento` (10× mais devagar), `falha` (erro amigável) e `panico`. Na Fase 4 ele vai para a feature `test-support` | O objetivo da fase é validar a fila e a UI antes dos motores |
| 2026-10-19 | Fase 3 | Envio gravado em `uploads/<id>.<ext>` (o nome do cliente só é exibido), apagado quando o job termina ou é removido; `uploads/` é esvaziada ao iniciar. As opções vêm antes do arquivo no formulário, e o servidor recusa o envio sem opção antes de gravar; num erro, lê o corpo até o fim para o navegador receber a resposta | Nenhum caminho controlado pelo cliente, nenhuma sobra entre execuções, erro rápido em envio grande |
| 2026-10-19 | Fase 3 | Cancelamento cooperativo: na fila, termina na hora; rodando, vira "Cancelando…" e a flag para o pipeline. Depois do pedido, `Ok` vale como Concluído, erro vira Cancelado e panic vira Falhou. "Remover" só em job terminado e sem confirmação, porque só apaga a cópia temporária; "Tentar de novo" e o `confirm-dialog` ficam para a Fase 4 | Semântica testada com proptest e com o worker real |
| 2026-10-19 | Fase 3 | Ctrl+C cancela o job ativo e espera o worker; as requisições em andamento têm 5 s (`SHUTDOWN_GRACE`) | Um upload longo ou parado não trava o encerramento |
| 2026-10-19 | Fase 3 | O botão de ação do card mantém o id entre Cancelar e Remover, e o htmx devolve o foco a ele a cada polling (teste E2E). A região `#avisos` anuncia só a entrada e o fim dos jobs | O card é trocado a cada segundo; anunciar cada % seria ruído |
| 2026-10-19 | Fase 3 | Design: `status-badge-active` com `colors.link`, porque o `primary` pedido para "Processando" dava 2,97:1 no escuro; `option-toggle` mostra o check só quando selecionada | Contraste AA e a regra "texto azul usa `colors.link`" |
| 2026-10-19 | Fase 3 | E2E com um `scriba.exe` por teste (fixture `scriba`), no lugar do `global-setup` compartilhado | Mitigação "diretórios temporários por teste"; nenhum estado vaza entre testes |
| 2026-10-19 | Fase 3 | Piso de cobertura sobe de 70% para 85% | A Fase 2 terminou com 92,6% das linhas cobertas no CI |
| 2026-10-19 | Fase 3 | Medição: upload de 2 GB pelo exe de debug em 12,4 s (165 MB/s), com o pico de working set indo de 14 MB para 19 MB (+5 MB). O CI repete com 256 MB e falha acima de +100 MB | Critério "upload de 2 GB com RAM estável" |
| 2026-10-19 | Fase 3 | `.gitattributes` com `* text=auto eol=lf`: todo texto sai com LF no checkout, em qualquer máquina | No runner, o `core.autocrlf` trocava os SVGs embutidos por CRLF e quebrava o snapshot HTML do card; os assets do exe ficam idênticos entre máquinas. Custo único: o `vcpkg.json` passa a sair com LF, o hash da chave do cache muda e o FFmpeg é recompilado uma vez na branch e uma na `main` |
| 2026-10-19 | Fase 3 | Medição no CI do PR: job `checks` em 39 min, dos quais 29,5 min recompilando o FFmpeg (cache refeito pelo `.gitattributes`); sem isso, ~9,5 min, com testes e cobertura em ~3 min e os 16 testes E2E em ~1 min. Cobertura de linhas: 94,1% (piso de 85%) | Com cache, ainda bem abaixo da meta de 20 min |
| 2026-10-19 | Planejamento | Cria a Fase 3.5 (build nativo mais rápido): triplet `x64-windows-static-release`, chave do cache com o triplet e CI semanal na `main`. Ficam de fora o cache NuGet do vcpkg (a chave inclui o hash do compilador, invalidado a cada atualização do MSVC no runner) e o clone raso do vcpkg (economiza menos de 30 s e o modo manifesto precisa do histórico); o FFmpeg enxuto vai para o Backlog | Os dois builds frios de 2026-10-19 levaram 29,5 e 18,9 min no `setup-native`, dos quais 11,3 e 6,8 min compilando o FFmpeg em debug, que nunca é linkado |
| 2026-10-19 | Fase 3.5 | Triplet `x64-windows-static-release` como overlay em `triplets/`, declarado no `vcpkg.json` (`vcpkg-configuration.overlay-triplets`), e não o triplet comunitário direto | O CI clona o `master` do vcpkg; a cópia versionada não muda por baixo e entra no hash da chave do cache |
| 2026-10-19 | Fase 3.5 | CI na `main` toda segunda às 06:00 UTC (`schedule`) e sob demanda (`workflow_dispatch`) | O GitHub apaga um cache sem acesso por 7 dias; o run também pega quebras do runner e avisos novos do RustSec |
| 2026-10-19 | Fase 3.5 | Medições: localmente o build frio do FFmpeg leva 8,0 min (x264 1,1 min, FFmpeg 6,4 min) e o `vcpkg_installed` cai de 428 para 217 MB; o exe de release segue com 85 MB, o `--self-test` passa e o `dumpbin` só lista DLLs do Windows. No CI do PR, `setup-native` frio em 16,3 min (antes 29,5 min num runner de velocidade parecida, −45%), com FFmpeg em 12 min e x264 em 2,4 min; job `checks` em 25,6 min; cache do FFmpeg de 77 para 61 MB. O único build de debug que sobra é o do `pkgconf` do host (15 s) | Critério da Fase 3.5 |
| 2026-10-19 | Fase 3.5 | Medições na `main`: o push do merge refez o build frio (FFmpeg 12 min, x264 2,4 min; `checks` em 26 min) e o run por `workflow_dispatch`, o mesmo workflow do agendado, passou em 8 min com o cache | Custo único previsto: um build frio na branch e outro na `main` |
| 2026-10-19 | Planejamento | Saída em `.txt` e `.md`: os dois vão para a pasta da saída, com "Baixar .txt" e "Baixar .md" no card e no leitor | Pedido do usuário |
| 2026-10-19 | Planejamento | Compactação como passo automático do fluxo padrão: o envio deixa de ter as opções "Transcrever" e "Compactar". A compactação roda depois da transcrição e só quando vale a pena (taxa de bits e resolução do original, mais a regra dos 5%) | Compactar não é uma funcionalidade à parte. Compactar antes não acelera a transcrição: o whisper só usa o áudio, extraído direto do original sem decodificar o vídeo, e o tempo depende da duração do áudio, não do tamanho do arquivo |
| 2026-10-19 | Planejamento | Troca de ordem: a Compactação vira a Fase 5 e a Gravação, a Fase 6 | A compactação é um passo do fluxo padrão dos envios, e a gravação reaproveita a compactação do áudio |
| 2026-10-19 | Fase 4 | A fase é dividida em seis etapas, cada uma com o próprio PR: 4.1 whisper otimizado e investigação da CPU, 4.2 `media` e `transcript`, 4.3 `asr` e benchmark, 4.4 pipeline real, 4.5 UI do resultado, 4.6 release e fechamento | Fase grande; cada etapa fecha com o CI verde |
| 2026-10-19 | Fase 4 | Corrige o achado (2) do spike A: o whisper.cpp do release compilava sem otimização. O cmake-rs descarta as flags `/O*` e, no gerador do Visual Studio, sobrescreve `CMAKE_<LANG>_FLAGS_RELEASE` quando o whisper-rs-sys fixa `CMAKE_BUILD_TYPE=Release`; o MSVC caía no `/Od`. No debug a variável sobrescrita é outra e o `/O2` sobrevivia. Correção: `CMAKE_C_FLAGS_RELEASE`/`CMAKE_CXX_FLAGS_RELEASE = "/O2 /Ob2 /DNDEBUG"` no `.cargo/config.toml`; o `release.yml` limpa o `whisper-rs-sys` antes do build e o self-test falha acima de 20 s (`SCRIBA_SELF_TEST_MAX_SECS`) | `CMakeCache.txt` do release sem `/O2`; os ~87 s do spike A e os 48 s do self-test no runner eram isso |
| 2026-10-19 | Fase 4 | Medições na CPU, `fala.wav` (10,9 s), 18 threads, na tomada e em "Melhor desempenho", antes → depois da correção: base 4,27 → 0,74 s; small 15,74 → 2,03 s; large-v3-turbo 82,1 → 9,06 s (5,8× a 9,1×). Contra o whisper-cli oficial 1.8.3 (com OpenMP): small 1,95 s e large 9,35 s (diferença ≤ 4%); base 0,60 s (+0,14 s). Na bateria e em "Melhor eficiência", os mesmos testes levam ~2× mais | Critério da investigação: nosso build a ≤ 15% da referência (o base passa por fração de segundo). OpenMP descartado: no MSVC exige o `vcomp140.dll` |
| 2026-10-19 | Fase 4 | Threads = núcleos lógicos − 2 continua (18 nesta máquina): melhor ou empatado em base, small e large em 65 s de áudio; 20 threads piora todos (~15%). `flash_attn` desligado na CPU e na Intel (CPU 10–25% mais lenta; Intel até 3,7× mais lenta) e ligado na RTX (large 12,5 → 1,29 s, ~51× tempo real) | Varredura 4–20 threads × flash, `bench.rs` com `SCRIBA_BENCH_REPEAT=6` |
| 2026-10-19 | Fase 4 | RTF em 65 s de áudio: CPU base 0,041, small 0,118, large 0,435; Intel large 0,53, small 0,17; RTX large 0,020 com flash | Entrada do benchmark small × base (4.3) e do ponto de controle: o large na CPU já fica abaixo de RTF 0,5 |
| 2026-10-19 | Fase 4 | Etapa 4.2: `media` e `transcript` prontos, ainda fora do app. Os oito formatos das fixtures (MP4, MP3, M4A, OGG, MKV, MOV, M4A 48 kHz estéreo e WAV) dão 10,931 s extraídos, igual ao WAV. O resampler é esvaziado no fim (`flush` com o frame de saída já alocado; vazio, o swr recusa com "Output changed") e recriado se o formato muda no meio. Um pacote ruim é pulado; erro de leitura ou nenhuma amostra viram "Não consegui ler o áudio…". MP4 truncado sem o `moov` não abre e dá "Não reconheci…" | `tests/media.rs` |
| 2026-10-19 | Fase 4 | O `probe` dá a duração do arquivo inteiro (contêiner), não só do áudio. O `fala.mp4` atual tem 13,7 s de vídeo para 10,9 s de áudio: o `-shortest` do xtask passava do áudio. As fixtures novas (`fala.mkv`, `fala.mov`) já nascem com o vídeo do tamanho do WAV; o `fala.mp4` não foi regerado | `ffprobe` das fixtures |
| 2026-10-19 | Planejamento | A mídia é descartável: sai a compactação de vídeo (x264/NVENC, regra de "vale a pena"). No fim de cada transcrição, "Guardar o áudio?": "Descartar" apaga o temporário; "Guardar áudio" gera um `.m4a` AAC mono e abre o "Salvar como" do Windows. A Fase 5 vira "Guardar áudio"; a gravação (Fase 6) usa a mesma pergunta e apaga as trilhas | Pedido do usuário: depois do texto, a gravação é lixo; compactar o vídeo não reduz a memória da transcrição, que só usa o áudio |
| 2026-10-19 | Planejamento | Reuniões de 2–3 h: a transcrição em blocos de ~30 min, cortados nos silêncios do VAD, passa a ser obrigatória na 4.6 (meta: pico de RAM ≤ ~1,5 GB em qualquer duração, medido com 1 h e 3 h) | Hoje o áudio inteiro, a cópia do VAD e o mel ficam na RAM: ~2,5–3 GB estimados para 3 h, pesado num PC de 8 GB |
| 2026-10-19 | Fase 4 | Formato da saída: `.txt` com `Arquivo:`, `Data:` (`19/10/2026 14:30`) e `Duração:` (`hh:mm:ss`), e um parágrafo por linha com `[hh:mm:ss]`, separados por linha em branco; `.md` em LF. `job.json` v1: `version`, `id`, `name`, `media` (`audio`/`video`), `size_bytes`, `duration_ms`, `created` (RFC 3339 com fuso), `backend`, `model`, `timings` e `files`. Nome da pasta limpo até 80 caracteres; nome reservado ganha `_` na frente | Proptest: `.txt` → `parse` volta igual; o nome limpo nunca é inválido |
| 2026-10-19 | Fase 4 | Etapa 4.3: `asr` pronto, ainda fora do app. O `state.full` do whisper-rs (`whisper_full_with_state`) ignora `params.vad`: o VAD Silero roda no nosso código (`vad.rs`), carregado da memória por loader próprio, sem gravar o modelo em `<data-dir>/models/` como o plano previa. A fala é juntada e os tempos voltam ao áudio original pelo `TimeMap` | Sem isso, o silêncio virava frase-fantasma e o tempo de transcrição incluía as pausas |
| 2026-10-19 | Fase 4 | O `set_abort_callback_safe` do whisper-rs 0.16 tem confusão de tipos (UB), e os callbacks "safe" vazam a closure: progresso e cancelamento usam trampolins C próprios sobre dados na pilha da chamada. Isso dispensa os handles `'static` no `Ctx` previstos para a 4.4 | Revisão do código do whisper-rs 0.16 |
| 2026-10-19 | Fase 4 | No áudio juntado pelo VAD, uma pausa longa vira ~0,2 s e o whisper encosta o início de um segmento no fim do anterior; o `TimeMap::segment_ms` usa a regra da "ponta" de 0,25 s para devolver o início ao trecho de fala certo | Teste do `pausas.wav`: 2 parágrafos |
| 2026-10-19 | Fase 4 | O 1º uso do Vulkan em cada processo compila os shaders (~9 s na RTX); vale para a estimativa de tempo da 4.4. A memória da GPU volta depois do ocioso (5224 → 4416 → 5224 MB livres na RTX) | Suíte de hardware de `tests/asr.rs` |
| 2026-10-19 | Fase 4 | Benchmark com as configurações do app (`tests/bench_asr.rs`), FLEURS pt_br (CC BY 4.0): 3 áudios de ~15 min (45,5 min) montados de 202 falas, com o texto oficial como referência, na tomada e em "Melhor desempenho". Transcrição / RTF / WER / RAM: base CPU 126 s / 0,046 / 15,9% / 498 MB; base Intel 222 s / 0,081 / 18,4% / 550 MB; base RTX 44 s / 0,016 / 15,8% / 93 MB; small CPU 286 s / 0,105 / 8,7% / 699 MB; small Intel 401 s / 0,147 / 8,7% / 756 MB; small RTX 56 s / 0,020 / 8,7% / 76 MB; large CPU 958 s / 0,350 / 6,6% / 1200 MB; large Intel 1107 s / 0,405 / 9,0% / 1248 MB; large RTX 72 s / 0,026 / 7,2% / 548 MB. Carga ≤ 1,2 s em todos. Nas fixtures (`fala.wav` + `pausas.wav`): large 2,9% de WER, base e small 8,6% | Entrada das decisões do modelo da CPU e da GPU integrada |
| 2026-10-19 | Fase 4 | Modelo da CPU: `small` (`CPU_MODEL`). Metade do erro do `base` por ~2,3× o tempo, e 1 h de áudio em ~6,3 min, bem dentro da meta de ≤ 1 h. O large na CPU (6,6%) custaria 3,3× o tempo e 1,2 GB de RAM | Benchmark acima; mantém a política da Fase 0 (modelo menor embutido) |
| 2026-10-19 | Fase 4 | Revê a política de backend da Fase 0: a GPU integrada sai da lista, que fica "GPU dedicada (mais memória primeiro) → CPU com o `small`". A Intel foi mais lenta que a CPU com todos os modelos e, com o large, teve WER pior (9,0% contra 6,6%) | Benchmark acima |
| 2026-10-19 | Fase 4 | Versões: `whisper-rs` com a feature `raw-api` (registro de dispositivos do ggml, VAD da memória e log); feature `vulkan` no `scriba` (`whisper-rs/vulkan`); `windows-sys` com `Win32_System_Threading`. O cache de modelos do CI passa a ter o base q5_1 e o Silero v6.2.0 | Reprodutibilidade |
| 2026-10-20 | Fase 4 | Etapa 4.4: o `Engine` (`src/engine.rs`: `media` → `asr` → `transcript`) é o pipeline do app, com a detecção de GPU síncrona na criação. O trait `Pipeline` ganha `probe` e `backend`; `Outputs` vira `{ dir, txt, md }`; o `Job` perde `options` e ganha `duration_ms`, `created` (a hora da pasta) e `estimate`. Sai o `Stage::Compressing`. O tipo de mídia passa a vir do probe (um MP4 só com áudio é áudio); a extensão continua como primeiro filtro | Sem caixinhas, todo job extrai o áudio e transcreve; o `Ctx` não precisou de handles `'static` (trampolins da 4.3) |
| 2026-10-20 | Fase 4 | Estimativa no card da fila: duração do probe × RTF do benchmark da 4.3 (GPU: base 0,016, small 0,020, large 0,026; CPU: 0,046, 0,105, 0,350), + 1,2 s se o modelo não está carregado, + 9 s no 1º uso do Vulkan no processo; a extração fica de fora até a 4.6 medir. Texto: "Cerca de N min na GPU" / "Menos de 1 min no processador"; o rodapé diz "Transcrição na GPU: NVIDIA GeForce RTX 3050 6GB Laptop GPU" ou "Transcrição no processador" (o candidato que carregou, ou o 1º antes da carga). Na RTX, `fala.mp4` (13,7 s): estimativa de 11 s e o teste de ponta a ponta em 16,3 s com a compilação dos shaders | "Processador" no lugar de "CPU" para quem não é técnico |
| 2026-10-20 | Fase 4 | "Tentar de novo": `Event::Retry` (Falhou → Na fila), `POST /api/jobs/{id}/retry`. O envio fica em `uploads/` quando o job falha (inclusive por panic) e é apagado ao concluir, ao cancelar e ao remover; a nova tentativa ganha outra flag de cancelamento, e cancelar um job já terminado não apaga o envio | Proptest com o `Retry`; `tests/jobs.rs` |
| 2026-10-20 | Fase 4 | Remover um job concluído: `DELETE /api/jobs/{id}` exige `?confirm=1` (409 sem) e manda a pasta da saída para a Lixeira (`SHFileOperationW` com `FOF_ALLOWUNDO`, `src/trash.rs`); pasta já apagada fora do app não impede. Na tela, `hx-confirm` abre o `confirm-dialog` (`<dialog>` modal pelo `htmx:confirm`) com o foco em "Cancelar". O `confirm-dialog`, o "Tentar de novo" e os `btn-secondary`/`btn-danger` foram antecipados da 4.5 | Itens herdados da Fase 3 no escopo da 4.4. Custo aceito: os testes da Lixeira deixam pastas pequenas na Lixeira da máquina |
| 2026-10-20 | Fase 4 | Feature `test-support`: o `FakePipeline` (grava uma transcrição fixa com o `transcript` real; ganchos novos `sem-audio` e `falha-uma-vez`) e a flag `--fake-pipeline`. O `tests/upload.rs` tem `required-features`; o CI passa `scriba/test-support` no clippy, nos testes e no build do E2E. O `--self-test` usa `media` e `asr` (sem o whisper duplicado), e o `embedded-test-model` embute também o VAD (+0,9 MB); o `hound` sai das dependências do `scriba` | O probe real recusaria os bytes falsos do E2E e do upload |
| 2026-10-20 | Fase 4 | `cargo deny` ignora o RUSTSEC-2026-0374 (`basic-toml` sem manutenção), com o motivo no `deny.toml` | Só o proc-macro do askama 0.16.1, a versão mais nova, o usa na compilação; nada vai para o exe. Rever quando o askama trocar de parser |
| 2026-10-20 | Fase 4 | Etapa 4.5: o leitor (`GET /jobs/{id}`, `src/web/reader.rs`) lê o `.txt` do disco com `transcript::parse`; os metadados vêm do job. Sem fala, `empty-state`; `.txt` apagado fora do app, `alert-inline` com a pasta e sem as ações; id fora da lista ou job não concluído, 404 com a mesma página ("Não encontrei esta transcrição na lista de Recentes.", porque a lista começa vazia a cada execução) | O leitor mostra o que está em disco, inclusive um `.txt` editado à mão |
| 2026-10-20 | Fase 4 | Downloads em duas rotas explícitas (`/api/jobs/{id}/transcricao.txt` e `.md`), com `Content-Disposition` `filename="transcricao.<ext>"` (ASCII fixo) e `filename*=UTF-8''` com o nome original limpo por `clean_name` ("Reunião - transcrição.md"), em percent-encoding próprio (RFC 5987), sem crate novo. No `app.js`, o clique baixa por `fetch` → `blob:` com o nome do atributo `download`, para um erro (arquivo apagado) virar toast; sem JS, o link baixa sozinho | Erro de ação sempre na tela, nunca um "Falha no download" do navegador |
| 2026-10-20 | Fase 4 | "Abrir pasta": `POST /api/jobs/{id}/open` (204), pelo trait `Opener` no `AppState` (`src/opener.rs`). O app usa o `open::that_detached`; com `--fake-pipeline`, o `PrintOpener` só imprime `abrir pasta: <dir>` no stdout, que a fixture do E2E lê (`scriba.opened()`). Pasta apagada: 404 com a mensagem, sem chamar o opener | Testável sem abrir o Explorer na máquina nem no runner |
| 2026-10-20 | Fase 4 | "Copiar texto" copia o `.txt` inteiro (cabeçalho e marcações de tempo) de um `<template>`, porque a CSP proíbe script inline. O parser do HTML troca CRLF por LF; o `app.js` volta o CRLF, e o texto copiado fica idêntico ao arquivo baixado (E2E) | Decisão 5 do plano da fase |
| 2026-10-20 | Fase 4 | Toast no rodapé central (`#toasts`, `aria-live="polite"`): "Texto copiado" some em 3 s; os erros das ações (POST e DELETE do htmx, downloads) mostram o texto da resposta por 6 s, sem cor de status. Os 409 e 500 do Remover, antes silenciosos, também aparecem | O erro precisa de tempo para ser lido |
| 2026-10-20 | Fase 4 | Design: `button-utility` com 44 px de altura (o padding de 8 px dava ~36 px, abaixo do alvo mínimo); `transcript-view` com `{rounded.lg}`; tempo do leitor como `00:00:08`, sem colchetes, na mesma linha do parágrafo. Ícones Lucide novos: copy, download, folder-open e chevron-left (lucide-static 1.59.0) | Acessibilidade (alvos de 44 × 44 px) |
| 2026-10-20 | Fase 4 | `windows-sys` com `Win32_UI_Shell` (Lixeira). Referências visuais do E2E regeradas (dropzone sem caixinhas, rodapé, metadados), com a data do card mascarada | Reprodutibilidade |
| 2026-10-20 | Fase 4 | Etapa 4.6: reuniões longas em blocos. A extração grava 16 kHz mono f32 cru num temporário ao lado do envio (`uploads/<id>.f32`, ~230 MB/h, apagado em qualquer desfecho; a pasta é esvaziada ao iniciar), e o `asr` lê uma janela de 30 min por vez (trait `Samples`): o VAD roda na janela, o corte cai no meio da maior pausa cujo meio está no último 1/6 (5 min), a mais tardia no empate; sem pausa ali, a última pausa da janela; sem pausa nenhuma, corte seco. Os tempos voltam ao original (`TimeMap::offset`), e o progresso de cada bloco ocupa a fatia dele | Só cortar a transcrição não bastava: com o áudio inteiro na RAM, 3 h já seriam 690 MB e 5 h, 1,15 GB. f32 e não i16 para os blocos lerem exatamente o que a extração entregou. Nos testes com bloco de 12 s, o corte seco no meio da fala trocava "sobre o cronograma" por "sobe. O Cronograma."; com o recuo para a última pausa, o texto fica igual ao sem blocos |
| 2026-10-20 | Fase 4 | Áudio real de referência: a audiência pública da Comissão de Viação e Transportes da Câmara de 02/09/2025 (evento 78800, 3 h 13 min, MP3 de 48 kbps gerado pelo arquivo sonoro da Câmara), com as notas taquigráficas. Fora do repositório; o `ffmpeg` CLI gerou 1 h em MP3 e em MP4 (H.264 720p sintético + AAC, como uma gravação do Teams) e trechos de 10 min em MP4, MP3, M4A, WAV e OGG | Reunião de verdade, com vários falantes e texto de referência público |
| 2026-10-20 | Fase 4 | Medições de 1 h e 3 h, build de release, na tomada e em "Melhor desempenho" (`uma_hora`/`tres_horas` em `tests/engine.rs`). 1 h (MP4): RTX 2 min 9 s (extração 1,2 s, RTF 0,036, pico de RAM 680 MB); CPU com o `small` 8 min 8 s (RTF 0,135, 1.073 MB). 1 h (MP3), repetida na CPU: 7 min 22 s (RTF 0,123, 997 MB). 3 h 13 min (MP3): RTX 5 min 55 s (extração 6 s, RTF 0,031, 631 MB); CPU 24 min 15 s (RTF 0,126, 1.139 MB). Metas cumpridas: ≤ 10 min na RTX, ≤ 1 h na CPU e pico ≤ 1,5 GB em qualquer duração | Critérios do "Pronto quando" e da meta de RAM. O "três backends" do "Pronto quando" vira "os dois da lista": a integrada saiu na 4.3, e o benchmark dela segue no registro |
| 2026-10-20 | Fase 4 | Comparação com um bloco só (`SCRIBA_LONG_BLOCK_MIN=999`, 1 h de MP3, já com o temporário em disco): RTX 1 min 45 s e 862 MB; CPU 7 min e 1.182 MB. Os blocos de 30 min custam ~5% de tempo na CPU e cortam ~185 MB do pico em 1 h; antes da 4.6, o áudio inteiro em RAM somava mais ~230 MB por hora | A diferença de tempo entre as execuções (RTF 0,117 a 0,135 na CPU) é da mesma ordem da variação entre rodadas |
| 2026-10-20 | Fase 4 | WER de apoio contra as notas taquigráficas, nas 3 h 13 min: 30,4% no large (RTX) e 41,0% no `small` (CPU). As notas são editadas (sem hesitações e repetições, frases reescritas; 26 mil palavras contra 28 mil transcritas): é um teto pessimista, não a taxa de erro | O texto foi conferido por leitura; o WER só compara os backends |
| 2026-10-20 | Fase 4 | Estimativa do card: o RTF do large na GPU passa a 0,032 e o do `small` na CPU a 0,125, as médias das reuniões medidas (mais densas de fala que o FLEURS do benchmark, que dava 0,026 e 0,105); os outros modelos seguem com o benchmark. A extração entra como 0,001 s por segundo de mídia (`engine::with_extraction`): 1 h de vídeo extraída em 1,3 s e 3 h de MP3 em 6 s, com folga de ~2× | A extração ficava de fora até ser medida (4.4); é desprezível perto da transcrição |
| 2026-10-20 | Fase 4 | `scriba-model`: o `embedded-cpu-model` (small q5_1, Silero e `fala.wav`) substitui o `embedded-test-model`; o `embedded-model` = large + o pacote da CPU. O `Models::embedded()` passa a ter o modelo da CPU: até a 4.5 o exe da release só embutia o base para o self-test, e o app dele não tinha modelo para transcrever. Exe de release com o `small`: 205 MB (antes 85 MB); o self-test transcreve a amostra em 3,1 s aqui e em 6,2 s no runner, e o `SCRIBA_SELF_TEST_MAX_SECS` cai de 20 para 15 s (sem `/O2` o `small` é ~7,7× mais lento); o `dumpbin` só lista DLLs do Windows. Versão 0.2.0 | Decisão 4 do plano da fase: release com transcrição só na CPU; large e Vulkan na Fase 7 |
| 2026-10-20 | Fase 4 | `release.yml`: `models: small,silero` no `setup-native` (a entrada `model: true/false` virou a lista do `--only`, com a chave do cache pelo hash do `xtask/src/models.rs`, onde ficam os SHA-256), build com `embedded-cpu-model` e o tamanho do exe no log. Também roda sem publicar nos PRs que mexem na release (`release.yml`, `setup-native`, `scriba-model`) e por `workflow_dispatch`; a publicação e a conferência da tag só rodam na tag | O `workflow_dispatch` só vale depois de estar na `main`; o gatilho de PR mede o self-test no runner antes do merge |
| 2026-10-20 | Fase 4 | `about.hbs` credita os modelos Whisper (OpenAI, MIT; conversão ggml do whisper.cpp, MIT) e o Silero VAD (MIT) | Os modelos vão dentro do exe |
| 2026-10-20 | Fase 4 | Piso de cobertura de 85% para 90% | A `main` estava com 93,2% das linhas cobertas no CI (`lcov.info` do último run) |
| 2026-10-20 | Fase 4 | Localmente, a suíte inteira roda com `cargo nextest run` (como o CI): com `cargo test`, os testes de whisper de um mesmo binário rodam em paralelo e as threads em espera ativa do ggml chegaram a travar o `asr` por mais de 30 min | Achado (3) do spike A, agravado pelos testes de blocos |
| 2026-10-20 | Fase 4 | E2E "a alternância de tema persiste após recarregar": cada clique espera o `htmx:afterSettle`, porque o segundo clique chegava antes de o htmx processar o botão trocado | Intermitência vista na 4.5 |
| 2026-10-20 | Fase 5 | A fase é dividida em duas etapas, cada uma com o próprio PR: 5.1 `media::encode_audio` e 5.2 a pergunta no app | O encoder é revisado e medido antes da UI; a 5.2 vai inteira porque guardar o envio sem a pergunta deixaria temporários sem saída |
| 2026-10-20 | Fase 5 | `.m4a` AAC-LC mono a 64 kbps, na taxa do original quando o AAC a aceita, com teto de 32 kHz e sem reamostrar para cima (`media::audio_rate`); encoder `aac` nativo do FFmpeg e muxer `ipod` com `+faststart` (o índice no começo, para o celular). A fonte é o original enviado, e não o `.f32` da extração (16 kHz, apagado depois da transcrição). O `vcpkg.json` não mudou: o `ff_aac_encoder` e o `ff_ipod_muxer` já estavam no FFmpeg estático | Decisão do mantenedor entre 48, 64 e 96 kbps: 32 kHz dá ~15 kHz de banda, transparente para fala e aceitável para música de fundo |
| 2026-10-20 | Fase 5 | A extração e a codificação dividem o `Source` (abrir a melhor faixa e decodificar numa taxa dada); a `extract_audio_into` não mudou por fora. Nas oito fixtures, o `.m4a` sai com 10,931 s, a duração exata do áudio do original (a edit list do MP4 tira o atraso do AAC) | `tests/media.rs` |
| 2026-10-20 | Fase 5 | Medições do encoder, build de release, na tomada: 1 h de MP4 (H.264 720p + AAC) em 14,2 s (254× tempo real) → 30,1 MB; 1 h de MP3 em 15,1 s (238×) → 30,3 MB; os dois a 32 kHz | `guardar_audio_longo` em `tests/media.rs` |
| 2026-10-20 | Fase 5 | Disco: o envio fica em `uploads/` até a resposta, sem descarte automático; sai ao Descartar, ao Guardar com sucesso, ao Remover, ao fechar o app (o `Worker::shutdown` esvazia a pasta) e ao iniciar. O card já mostra o tamanho do original | Decisão do mantenedor; mantém o ROADMAP e o mixdown só sob pedido da Fase 6 |
| 2026-10-20 | Fase 5 | Estado: `State::Done { outputs, audio }`, com a máquina `Audio` pura (pendente → escolhendo → guardando → guardado, ou descartado; cancelar ou erro volta à pendente, com o erro uma vez) e proptest. Cada "Guardar áudio" roda numa thread própria (`scriba-audio-<id>`), fora do worker, para não esperar uma transcrição: abre a janela, grava em `<arquivo>.parte` pelo `Pipeline::save_audio` e renomeia no fim, substituindo o arquivo que a janela já mandou sobrescrever. O POST responde na hora com o card em "Escolhendo onde guardar…", e o card acompanha pelo polling (`?audio=1`); nenhuma requisição fica presa com a janela aberta | O encerramento continua com os 5 s de folga; o `Worker::shutdown` cancela as gravações e espera só as que estão codificando |
| 2026-10-20 | Fase 5 | Rotas `POST /api/jobs/{id}/audio/{discard,save,cancel,show}`; 409 para job não concluído, áudio já respondido ou ocupado, e Remover com o áudio ocupado. O erro chega ao fim do polling como `HX-Trigger` `scriba-erro`, com o JSON escapado em ASCII (`\uXXXX`), porque o XHR lê o cabeçalho como Latin-1; o `app.js` o mostra num toast. A lista não repete o toast | Erro de ação sempre em toast, como na 4.5 |
| 2026-10-20 | Fase 5 | "Salvar como": `IFileSaveDialog` pelo crate `windows` 0.62.2 (o mesmo que o `cpal` já trazia ao `Cargo.lock`; features `Win32_Foundation`, `Win32_Graphics_Gdi`, `Win32_System_Com`, `Win32_System_Threading`, `Win32_UI_Shell`, `Win32_UI_Shell_Common` e `Win32_UI_WindowsAndMessaging`), numa thread STA própria, dona de uma janela oculta topmost no monitor do navegador, com `AttachThreadInput` + `SetForegroundWindow`. A janela abriu em primeiro plano e topmost no teste local e no app. Nome sugerido = a pasta da transcrição (`AAAA-MM-DD HHmm - nome.m4a`); pasta inicial = a última escolhida (`audio_dir` no `config.json`), salvo se ela foi a pasta de outra transcrição ou sumiu, quando vale a desta | Decisão do mantenedor para a pasta inicial; o `SetFolder` (e não o `SetDefaultFolder`) vale mais que a pasta que o Windows lembra |
| 2026-10-20 | Fase 5 | Achado no teste com o exe: o `SHCreateItemFromParsingName` recusa caminho com `/`, e a janela caía em Documentos. O caminho passa pelo `std::path::absolute` (que troca por `\`) e a recusa vai para o stderr | `--output-dir` com barras do Git Bash |
| 2026-10-20 | Fase 5 | "Mostrar no Explorer": `Opener::reveal` com `SHOpenFolderAndSelectItems` (a pasta aberta com o `.m4a` selecionado, conferido no app); o `PrintOpener` imprime `mostrar arquivo: …`. O E2E controla o "Salvar como" pelo `ScriptedDialog` (`<data-dir>/salvar-como.txt`) | Mesmo padrão do `PrintOpener` da 4.5 |
| 2026-10-20 | Fase 5 | Design: componente `audio-prompt` (parchment dentro do card branco, `{typography.callout}`, `{rounded.md}`, 12 × 16 px), com "Guardar áudio" e "Descartar" em `button-secondary` de mesmo peso e "Mostrar no Explorer" em `button-utility` (ícone Lucide folder-search). Enquanto grava, o botão de ação do card vira "Cancelar o áudio de …"; com a janela aberta, ele fica inativo | Nenhuma resposta é a errada; a troca de superfície separa a pergunta sem borda nem cor |
| 2026-10-20 | Fase 5 | "Pronto quando" com o exe de release (Vulkan) e o MP4 de 1 h (472 MB): transcrição em 119 s na RTX (estimativa do card: "Cerca de 3 min"); "Guardar áudio" pela janela real em 25 s do clique ao fim; `.m4a` de 30,1 MB, AAC-LC mono a 32 kHz, 3.600,02 s, que o shell do Windows (o Media Foundation, o mesmo do Media Player) lê como "Arquivo M4A" de 01:00:00. A conferência no celular fica com o mantenedor | Critério da fase |
| 2026-10-20 | Fase 5 | Celular, conferido sem aparelho: o `.m4a` de 1 h é AAC-LC mono a 32 kHz em MPEG-4 com a marca `M4A ` (compatíveis `isom`/`iso2`), `moov` antes do `mdat` e edit list do atraso do AAC, o formato que Android e iOS tocam. O `<audio>` do Edge, que não usa o FFmpeg do Scriba, dá 3.600,02 s, toca o começo e toca o fim até o `ended`; `canPlayType("audio/mp4; codecs=mp4a.40.2")` = "probably". O teste num aparelho fica para a primeira versão de uso | O mantenedor só testa à mão na primeira versão de uso; até lá, os itens que dependeriam dele são conferidos com o melhor substituto possível |
| 2026-10-20 | Fase 6 | A fase é dividida em quatro etapas, cada uma com o próprio PR: 6.1 captura e `Recorder`, 6.2 job de gravação e transcrição rotulada, 6.3 UI e rotas, 6.4 "Pronto quando" e fechamento | Fase maior que a 5; o motor de captura é medido no hardware antes de entrar no app |
| 2026-10-20 | Fase 6 | Decisões do mantenedor: (1) Ctrl+C ou crash durante a gravação não a perdem: as trilhas ficam em `<data-dir>/gravacoes/<id>/` com um `gravacao.json`, e ao abrir o app cada gravação ainda não transcrita volta à fila; (2) cancelar a transcrição de uma gravação descarta as trilhas, com `confirm-dialog`; (3) WAVs a 32 kHz fixos, mono 16-bit (~230 MB/h por trilha), reamostrados na thread de escrita; (4) a voz "Você" é conferida no hardware de forma híbrida (a Maria misturada depois na trilha real do microfone), sem instalar um cabo de áudio virtual | As trilhas são a única cópia da gravação; 32 kHz é o teto do `.m4a` (Fase 5) e deixa as duas trilhas na mesma taxa com qualquer dispositivo; esta máquina tem uma saída só, e tudo que toca nela cai no loopback |
| 2026-10-21 | Fase 6 | Medições da etapa 6.1 com o microfone e o loopback reais (`tests/capture.rs`, build de release; microfone: matriz Intel Smart Sound; saída: alto-falantes Realtek a 22%): (1) 15 s de fala da Câmara, 15 s sem nada tocando e 15 s de novo: loopback sem nenhuma lacuna no silêncio; marcadores tocados a 7 s ficaram a 6,9995 s no WAV; erro máximo contra o relógio 0,9 ms (microfone, −528 ppm) e 2,5 ms (sistema). (2) 3 min gravando com o `small` transcrevendo 1 h na CPU: nenhum pacote perdido, marcadores a −2,0 ms em 177 s. (3) 60 min: nenhum pacote perdido, marcadores a −1,7 ms em 3.595 s, erro máximo 2,2 ms e 4,0 ms, deriva −15 e −2 ppm; pico de 452 MB no processo do teste, que inclui o áudio decodificado para tocar (o Recorder sozinho é medido na 6.4). A abertura dos streams leva ~0,6–0,8 s, e o microfone tem 20–50 ms de lacuna logo no começo | Critérios da etapa. O microfone desta máquina cancela no DSP o som dos alto-falantes, por isso o alinhamento é medido por marcadores na trilha do sistema e pelo erro de cada trilha contra o relógio, e não pelo eco |
| 2026-10-21 | Fase 6 | Suspensão do computador durante a gravação: enquanto grava, a thread de escrita pede ao Windows para não suspender por inatividade (`SetThreadExecutionState` com `ES_SYSTEM_REQUIRED`); se ele suspender mesmo assim (tampa fechada no intervalo, Modern Standby com a tela apagada), a gravação segue ao acordar, na mesma gravação: uma lacuna de mais de 30 s numa trilha tira o tempo suspenso do relógio das duas juntas (as marcações de tempo seguem corridas, e o status avisa a pausa), e um timestamp mais de 1 s no futuro (um dispositivo que voltou com o relógio errado) reancora só a trilha dele. Um intervalo da reunião com o computador ligado não dispara nada: os pacotes continuam chegando, silenciosos | Decisão do mantenedor (continuar em vez de encerrar, para uma reunião com intervalo). Na 1ª tentativa da gravação de 1 h, o notebook entrou em Modern Standby várias vezes durante a noite: o teste durou 10 h, o alinhamento preencheu 9,3 h de silêncio e o microfone voltou de uma suspensão com um timestamp 8,9 h à frente |
| 2026-10-20 | Fase 6 | O `src/capture/wasapi.rs` (o `CpalSource`) sai do cálculo de cobertura do CI, como os spikes | Só roda com placa de som, que o runner não tem; a suíte local de hardware (`tests/capture.rs`) o exercita. Com ele, a cobertura cairia de ~91,9% para 89,4%, abaixo do piso de 90% |
| 2026-10-20 | Fase 6 | Alinhamento pelo timestamp de captura de cada pacote (o QPC do 1º frame, o mesmo relógio nas duas trilhas): lacuna acima de 20 ms vira silêncio, deriva entre 2 e 20 ms é corrigida aos poucos pelo `swr_set_compensation`, pacote adiantado acima de 20 ms perde o começo; meta de ±10 ms entre as trilhas. Medidores por SSE (`GET /api/record/levels`); sem abas no header; eco removido por sobreposição (±1 s) e ≥ 60% das palavras em ordem (LCS); `.txt` de gravação com `Gravação:` no cabeçalho e `Você:`/`Outros:` depois do tempo, lidos só nesse caso | Plano da fase |
| 2026-10-21 | Fase 6 | Etapa 6.2: a gravação vira job. `Media::Recording`, com o `Job.input` na pasta da gravação; `Job::recording` monta "Gravação de 20/10, 14:30", o tamanho dos dois WAVs, a duração e a estimativa = 2× a de um arquivo da mesma duração (o `Pipeline` ganha `estimate`, `None` por padrão). Saída na pasta `AAAA-MM-DD HHmm - Gravação`, `job.json` v1 com `"media": "recording"`; o download do `.txt`/`.md` usa o nome da pasta ("2026-10-20 1430 - Gravação - transcrição.txt"), porque o nome da lista tem `/` e `:` | Decisões 16 e 18 do plano da fase |
| 2026-10-21 | Fase 6 | O `Engine` extrai as duas trilhas (sistema e depois microfone, cada uma com o seu `.f32` dentro da pasta da gravação) e só então as transcreve; em cada etapa, cada trilha fica com metade do progresso, e o card nunca volta de "Transcrevendo" para "Extraindo áudio". Uma trilha sem amostras (o app caiu antes do 1º `flush`) fica de fora | Progresso em duas metades sem regressão de etapa |
| 2026-10-21 | Fase 6 | Eco (`transcript::merge`): um segmento "Você" é eco quando algum "Outros" cruza a janela de ±1 s em volta dele e ≥ 60% das palavras dele (minúsculas, sem pontuação e sem acento, por uma dobra à mão das letras do pt-BR, sem crate novo) aparecem em ordem (LCS) no texto "Outros" dessa janela. "Outros" passa intacto; a saída é ordenada por tempo, com "Outros" antes no empate. Nas fixtures (`voce-com-eco.wav` = "Você" + "Outros" a −12 dB e 60 ms depois), o modelo base transcreve o eco inteiro no microfone sozinho, e a mescla tira as duas frases | Decisão 17; proptest e `tests/engine.rs` |
| 2026-10-21 | Fase 6 | Ciclo de vida das trilhas no store (decisão 5): Descartar, Guardar com sucesso, Remover e Cancelar (na fila ou rodando) apagam a pasta da gravação. No encerramento, só a gravação concluída com a pergunta sem resposta sai; a que estava na fila, rodando (o Ctrl+C a cancela, mas a pasta fica) ou que falhou volta na próxima abertura. Ao concluir, o `gravacao.json` passa a `"state": "transcrita"`, e o `capture::recover` apaga essas pastas ao iniciar (crash depois da transcrição), para não transcrever duas vezes | Decisão 1; `tests/jobs.rs` |
| 2026-10-21 | Fase 6 | `media::encode_mix`: lê os WAVs com o `hound` (mono 16-bit, mesma taxa; senão o erro amigável de áudio ilegível), soma por blocos de 1 s com limite em ±1, completa a mais curta com silêncio e entrega ao `AacWriter` da Fase 5 (32 kHz), sem WAV misturado em disco; apaga o parcial em erro ou cancelamento. As fixtures novas (`voce.wav`, `outros.wav`, `voce-com-eco.wav`, 29,5 s a 32 kHz) saem do `cargo xtask fixtures --only …`, que ganha o `hound` | Decisão 19 |
| 2026-10-21 | Fase 6 | Conferência local da 6.2 (`gravacao_real` em `tests/engine.rs`, build de release com Vulkan, RTX 3050 com o large-v3-turbo), feita assim: as trilhas reais da 6.1 em `%TEMP%\scriba-gravacao\silencio` (58,9 s; microfone: matriz Intel Smart Sound; saída: alto-falantes Realtek; 15 s do trecho da Câmara, 15 s sem nada, 15 s de novo, depois os marcadores) foram copiadas para um temporário, e o `voce.wav` (Maria) foi somado ao `mic.wav` real a partir de 16 s: a 1ª fala dela caiu no trecho sem nada tocando e a 2ª por cima da Câmara. O `Engine` transcreveu as duas trilhas em 21,1 s (com a carga do modelo). Resultado: "Outros" com 111 palavras do texto da audiência, idêntico (similaridade 1,00) à transcrição do `sistema.wav` sozinho; as duas falas da Maria como "Você" (similaridade 1,00 e 0,89; "email" saiu "e-mail"), num parágrafo só, porque o whisper as entregou num segmento; nenhum parágrafo "Você" repete "Outros" (0,20). O microfone desta máquina cancela os alto-falantes no DSP, então a trilha real não tinha eco a remover; a remoção foi provada nas fixtures (o base transcreve o eco de −12 dB inteiro no microfone sozinho). Uma pessoa falando de verdade fica para a primeira versão de uso | Verificação 2 do plano da fase |
| 2026-10-21 | Fase 6 | Etapa 6.3, rotas da gravação (`src/web/record.rs`): Gravar e Parar respondem só fragmentos fora de banda (`hx-swap="none"`): a `recording-bar` e, com `?painel=1` (a página inicial), o `record-panel` e, no Parar, o card novo no topo de `#jobs`, mais o aviso de `#avisos`. A barra é quem faz o polling (`GET /api/record/status?id=…`), em todas as páginas: gravando, 200 com a barra e só o tempo e os avisos do painel (o painel inteiro trocado a cada segundo fecharia os Ajustes abertos); parada, 286 com o painel parado e, se a gravação acompanhada acabou, o card e o motivo do fim automático no toast `scriba-erro`. Se o Parar e o polling trouxerem o mesmo card, o `app.js` deixa só um. Erro ao começar: 409 em toast; 422 (sem microfone ou saída, microfone bloqueado) e 507 (disco, arquivos) no `alert-inline` do painel, ou em toast numa página sem ele | Decisões 7, 8, 9 e 10 do plano da fase; um só polling por página |
| 2026-10-21 | Fase 6 | Medidores e tempo pelo SSE (`GET /api/record/levels`): um evento a cada 100 ms com o tempo e os dois níveis já na escala do medidor (RMS em dB, −60 dBFS vazio e 0 dBFS cheio, para a fala ocupar o meio do `<meter>`); termina com `event: fim` quando a gravação para e quando o app fecha (`AppState::close` no Ctrl+C), e o `EventSource` do `app.js` fecha nele em vez de reconectar. No Ctrl+C, depois do servidor, `recorder.finish_on_exit()` fecha os WAVs antes do `worker.shutdown` | Decisões 1 e 7 |
| 2026-10-21 | Fase 6 | Ajustes: o `<details>` carrega a lista a cada `toggle` (`hx-trigger="toggle"`: os filtros de evento do htmx usam `eval`, que a CSP bloqueia), salva na hora (`change`) o `mic` e o `output` no `config.json` (vazio = o padrão do Windows), fica inativo durante a gravação (409 no POST) e devolve um id salvo que sumiu ao padrão, também no Gravar. O aviso Bluetooth vale para o microfone escolhido ou, em "Padrão do Windows", para o padrão de agora | Decisões 11 e 12 |
| 2026-10-21 | Fase 6 | `--fake-audio` (feature `test-support`): o `ScriptedSource` grava em tempo real dois tons que sobem e descem de volume (`Signal::Pulsing`, para os medidores mexerem), com os dispositivos simulados (um Bluetooth), e relê a cada abertura o `<data-dir>/audio-falso.txt`: `perder-mic` e `perder-saida` (o dispositivo some 2 s depois), `sem-microfone`, `microfone-bloqueado` e `mic-mudo`. A fixture do E2E ganha `audioScript` e `restart()` (mata o exe e abre outro na mesma pasta de dados) | Decisão 1; padrão do `ScriptedDialog` |
| 2026-10-21 | Fase 6 | Ctrl+Shift+R: o Edge não reserva o atalho (conferido no E2E: a página não recarrega, inclusive no leitor), então ele ficou. Na página inicial clica no botão do painel; no leitor, no Parar da barra, ou pede o Gravar pelo `htmx.ajax` | Risco listado no plano da fase |
| 2026-10-21 | Fase 6 | O E2E instável "o foco no botão do card sobrevive ao polling…": o htmx só processa o elemento trocado depois do `settleDelay` (20 ms por padrão); um clique ou Enter nessa janela cai num botão ainda sem o handler e não faz nada. Com `defaultSettleDelay: 0` no `htmx-config`, o processamento é síncrono. Medido: com 20 ms, 3 falhas em 24 execuções desse teste e do Cancelar de uma gravação; com 0, nenhuma em 24. O `confirm-dialog` também refaz o pedido a partir do botão de agora (mesmo id) se o polling trocou o card enquanto ele estava aberto | Os testes novos da gravação usam o mesmo padrão |
| 2026-10-21 | Fase 6 | Achado no E2E: o "Voltar" do navegador mostrava a página inicial do cache, com a lista de antes. As páginas saem com `Cache-Control: no-store` | Lista sempre atual |
| 2026-10-21 | Fase 6 | Gravação na tela: card com o ícone `mic`; o Cancelar (na fila ou transcrevendo) e o Remover de uma gravação que falhou abrem o `confirm-dialog` "Descartar a gravação?" / "As trilhas gravadas serão apagadas." (botão "Descartar"; o título e o botão vêm dos `data-confirmar-*` do botão); o leitor mostra o `transcript-speaker` ("Você:", "Outros:"). O `AppState::new` recebe o `Desktop` (opener, "Salvar como" e `Recorder`) | Decisão 2 |
| 2026-10-21 | Fase 6 | Conferência local da 6.3, feita assim: o exe de debug (o do `cargo nextest` da etapa, com Vulkan) aberto com `--no-open --port 0` numa pasta de dados temporária; o cookie tirado do redirecionamento do `?t=`; os Ajustes (`GET /api/record/devices`) listaram "Padrão do Windows (Grupo de microfones (Tecnologia Intel® Smart Sound…))" e "Padrão do Windows (Alto-falantes (Realtek(R) Audio))"; `POST /api/record/start` com o `Origin` do app → 200; 15 s do `trecho.m4a` da Câmara tocados nos alto-falantes pelo `ffplay`, mais 2 s; o status mostrou 00:00:20; `POST /api/record/stop` → 200, com `mic.wav` e `sistema.wav` de 1,3 MB cada (20 s a 32 kHz). O job concluiu em 5 s na RTX 3050 (large-v3-turbo), com "Guardar o áudio?" no card e o texto da audiência rotulado "Outros" a partir de 00:00:03 (o atraso do `ffplay`); nenhum parágrafo "Você", como esperado nesta máquina (o DSP do microfone cancela o eco dos alto-falantes, registro da 6.1). O stderr só avisou dois buracos de 20 e 30 ms no microfone no 1º segundo, cobertos com silêncio pelo alinhamento | Verificação da etapa; o "Pronto quando" com voz de verdade fica para a 6.4 |
| 2026-10-21 | Fase 6 | Conferência da etapa 6.4 ("Pronto quando"), com o exe de release com Vulkan (`cargo build --release --features vulkan`), na tomada e no modo de melhor desempenho, feita assim: (1) o app aberto com `--no-open --port 0` numa pasta temporária, o cookie do `?t=`, `POST /api/record/start` com o `Origin` do app, o `ffplay` tocando nos alto-falantes um arquivo montado com o `ffmpeg`: marcador de 3 kHz/80 ms, 20 s do `trecho.wav` da Câmara, 15 s de silêncio, marcador, mais 20 s, marcador (57 s), e `POST /api/record/stop` 2 s depois. Card concluído na RTX 3050 (large-v3-turbo) com "Outros" a partir de 00:00:02 com o texto da audiência e nenhum parágrafo "Você". Os marcadores no `sistema.wav` ficaram a 1,8945, 37,8945 e 58,8946 s: intervalos de 36,0000 e 21,0001 s contra 36 e 21 s tocados, ou seja, a defasagem antes e depois do silêncio difere ≤ 0,1 ms. O microfone teve pico de 0,0015 (o DSP da matriz Intel cancelou os alto-falantes) e só os buracos de 20 e 10 ms do 1º segundo. (2) Híbrido: essa gravação copiada e o `voce.wav` somado ao `mic.wav` real a partir de 14 s (`SCRIBA_REC_VOCE_AT=14`; a 1ª fala da Maria no silêncio, a 2ª por cima da Câmara), `gravacao_real`: 17,2 s na GPU, "Você" com as duas falas (similaridade 1,00 e 0,89), "Outros" igual ao `sistema.wav` sozinho (1,00, 118 palavras), nenhum "Você" repete "Outros" (0,30). (5) Ctrl+C: o exe aberto no próprio console (`Start-Process`), 25 s gravando a Câmara, e um PowerShell anexado ao console dele (`AttachConsole` + `GenerateConsoleCtrlEvent`): o app fechou com as duas trilhas finalizadas (32,9 s cada, `gravacao.json` em `parada`); reaberto na mesma pasta, imprimiu "gravação retomada", transcreveu na GPU e marcou `transcrita` | Verificação 1, 2 e 4 do plano da fase |
| 2026-10-21 | Fase 6 | Custo das duas trilhas (`custo_da_gravacao` em `tests/engine.rs`, novo): 10 min do trecho da Câmara como sistema; microfone "com fone" = as duas frases da Maria a cada minuto, "sem fone" = isso mais o sistema a −12 dB e 60 ms depois; o arquivo = a soma das trilhas "sem fone"; modelo carregado antes. Última medição: GPU (RTX 3050, large-v3-turbo) arquivo 19,6 s, com fone 1,36×, sem fone 1,96×; CPU (`small`) arquivo 87,5 s, com fone 1,11×, sem fone 1,83×. Nas cinco rodadas, o "sem fone" ficou entre 1,68× e 2,07× (2,06× em 3 min na CPU). A estimativa do card de gravação passa de 2× para `RECORDING_COST` = 2,2× a de um arquivo da mesma duração (arredondada para cima), e o teste afirma que cada razão cabe nele | Decisão 16: o 2× era o teto provisório e foi passado em duas medições |
| 2026-10-21 | Fase 6 | Achado do custo: "sem fone" (eco cru, sem o cancelamento que o microfone desta máquina faz), o `small` entra em laço no eco e repete por minutos, em pedaços rotacionados, uma frase que "Outros" disse uma vez; a janela de ±1 s não o alcança, e saíram 1.172 palavras "Você" em vez de ~200. A mescla ganha duas regras só para "Você" longo (≥ 8 palavras): é eco se ≥ 60% das palavras dele estão em ordem num só "Outros" que começou até 10 min antes, ou no texto dos "Você" que começaram até 30 s antes do fim dele (o laço). A janela conta do fim porque, depois de um silêncio, o whisper às vezes põe o começo do segmento no fim do anterior (a fala parecia 45 s mais cedo, e a 1ª versão com 60 s pelo começo tirou falas da Maria). Resultado: CPU "sem fone" 1.172 → 90 palavras "Você"; "com fone" 200 de 200 nos dois backends (o teste afirma ≥ 90%). Resposta curta repetida ("Combinado.") e frase longa repetida 40 s depois ficam | Decisão 17 ampliada; teste unitário `laco_do_whisper_no_eco_sai`. Lacuna: na GPU, "sem fone", sobram ~90 palavras de um laço de frases de 7 palavras, abaixo do mínimo |
| 2026-10-21 | Fase 6 | Gravação longa e concorrente com o build de release (`tests/capture.rs`): `gravacao_longa` (60 min) sem pacotes perdidos, deriva −11 ppm (microfone) e −2 ppm (sistema), erro máximo 2,09 e 2,75 ms, marcadores a +6,7 ms em 3.595 s, 39,5 ms de silêncio inserido no microfone em 3 vezes, pico de 444 MB no processo do teste (inclui a hora de áudio decodificada para tocar, como na 6.1; aos ~29 min de gravação, o `tasklist` mostrava o processo com 67 MB, então o pico é da decodificação, e o `Recorder` em si fica nos rings de 10 s e nos buffers de escrita); `gravar_enquanto_transcreve_na_cpu` (`SCRIBA_LONG_AUDIO` = `sessao-1h.mp3`) sem pacotes perdidos, erro máximo 1,12 e 1,97 ms, marcadores a −1,5 ms em 177 s | Verificação 3 do plano da fase; confirma a decisão 15 sem reduzir as threads do whisper |
| 2026-10-21 | Fase 6 | Textos revistos no fechamento: decisão 5 (as trilhas ficam até a resposta a "Guardar o áudio?"; a linha "Captura" das decisões de arquitetura e a entrega "Job de gravação" já dizem isso desde a 6.2) e decisão 9 (sem abas no header: a tela única com o painel e a dropzone, a `recording-bar` em todas as páginas, como no `DESIGN-scriba.md`). Os limiares do alinhamento ficaram mais justos que no plano: lacuna acima de 8 ms (não 20) vira silêncio, e a deriva entre 2 e 8 ms é corrigida | A linha de 2026-10-20 sobre o alinhamento registra os números do plano |
| 2026-10-21 | Fase 7 | A fase é dividida em cinco etapas, cada uma com o próprio PR: 7.1 logs e panic hook, 7.2 app de bandeja, 7.3 histórico, config e limpeza, 7.4 release completa, 7.5 Sandbox e entrega. Decisões do mantenedor: (1) um exe só, com Vulkan, o large-v3-turbo e o `small` (~840 MB); (2) a mídia sem resposta a "Guardar o áudio?" é descartada depois de 24 h, porque o app de bandeja fica aberto por semanas; (3) o histórico mostra as 50 transcrições mais recentes, lidas dos `job.json`; (4) o Windows Sandbox é habilitado por `scripts/setup.ps1 -Sandbox`. O exe de release importa direto três funções do `vulkan-1.dll` (`dumpbin`), então o delay-load do "Plano B" dos riscos passa a ser necessário (etapa 7.4) | Os logs vêm antes da bandeja: sem console, o log é o único registro. Uma máquina sem driver de GPU (VM, Sandbox) não abriria o exe |
| 2026-10-21 | Fase 7 | Etapa 7.1, logs (`src/logs.rs`, sem crate novo): `<data-dir>\logs\scriba.log`, girando acima de 5 MB e guardando 4 antigos (até 25 MB), cada linha com data e hora em milissegundos. A captura é nos handles: o stdout e o stderr passam por dois pipes (`SetStdHandle`, e `_open_osfhandle` + `_dup2` nos fd 1 e 2 do CRT estático), e uma thread por pipe grava no arquivo e repete a linha no destino de antes (uma cópia do handle original: o console, em UTF-16, ou o pipe de quem abriu o exe). O panic hook (thread, `Location`, mensagem e backtrace) e o `SetUnhandledExceptionFilter` gravam direto no arquivo. Só a instância que serve liga os logs; ela abre com a versão, a build do Windows (pelo registro: o `ProductName` diz "Windows 10" também no 11), as pastas e o backend. Um erro fatal sem ninguém vendo o stderr vira uma `MessageBoxW` | Os ~65 `eprintln!` não mudam, e o `std::cerr` do ggml-vulkan e o `av_log` do FFmpeg também caem no log; trocar as chamadas por uma macro deixaria o código C de fora. O tee usa uma cópia do handle original porque o `_dup2` do CRT pode fechar o handle que o fd tinha |
| 2026-10-21 | Fase 7 | Etapa 7.2, app de bandeja. A release é `windows_subsystem = "windows"` (o debug continua de console); `--self-test`, `--help`, `--version` e erro de argumento se anexam ao console de quem chamou (`AttachConsole(ATTACH_PARENT_PROCESS)`, `src/console.rs`), o modo app nunca. Um `Shutdown` só (`src/shutdown.rs`) para o Ctrl+C, o "Sair" da bandeja, o `WM_ENDSESSION` e a rota nova `POST /api/app/sair` (204, atrás do `security::guard`), na ordem de antes (servidor, `finish_on_exit`, `worker.shutdown`, `instance.json`); no `WM_ENDSESSION` a gravação fecha primeiro, o servidor não espera a folga e o `ShutdownBlockReasonCreate` mostra "Finalizando a gravação…" até o app terminar (teto de 20 s). A bandeja é feita à mão com o `windows-sys` (o mesmo crate da família `windows` que já fazia a janela do erro fatal; o tipado não ajuda num `wndproc`): janela oculta top-level `ScribaBandeja`, `Shell_NotifyIconW` v4, clique abre, menu "Abrir o Scriba"/"Sair", dica "Scriba — gravando", `TaskbarCreated` (tira e põe o ícone, porque uma mensagem sem reinício real fazia o `NIM_ADD` falhar) e `WM_SETTINGCHANGE` "ImmersiveColorSet". A decisão fica em `tray::run`, puro, com o `Desk` (abrir, confirmar) e o `quit_warning`. `--no-tray` tira só o ícone: a janela fica para o fim da sessão. Instância única com o mutex `Local\Scriba-<FNV-1a da pasta de dados>` (o `DefaultHasher` muda entre versões do Rust); a 2ª execução espera até 10 s o `/health` da 1ª. `winresource` (sem o `toml`) embute os três `.ico`, a VERSIONINFO pt-BR e o manifesto; os `.ico` saem de `cargo xtask icon` (`resvg` 0.48 sem features, `ico` 0.6, só no xtask; o `cargo deny` passou sem mudar o `deny.toml`). O `win32.rs` fica na cobertura: os testes de integração exercitam a janela e o `WM_ENDSESSION`. Medido aqui no exe de release (small embutido): subsistema PE 2, nenhum console nem processo filho, 168 ms da abertura ao "Scriba em" no log, 14 MB de RAM ociosa, `--self-test \| Out-Host` com código 0 em 3 s, a 2ª execução sai com 0 em ~1 s, `POST /api/app/sair` 204 e saída 0, `TaskbarCreated` e troca de tema enviados à janela real sem erro, o "Salvar como" com o manifesto PerMonitorV2 em primeiro plano (`janela_abre_na_frente`). Não feitos aqui: reiniciar o Explorer de verdade e clicar no menu | Sem console, um Ctrl+C ou fechar o terminal não mata uma gravação; o logoff deixa os WAVs fechados para a próxima abertura; duas aberturas simultâneas não sobem dois servidores |
| 2026-10-21 | Fase 7 | Etapa 7.3, histórico (`src/jobs/history.rs`): ao abrir, `<output-dir>/*/job.json` v1 vira job concluído (`Jobs::restore`, sem fila), as 50 mais recentes pela hora do `created` (com o fuso, `LocalTime::parse_rfc3339` novo); a pasta é a do achado. Fica de fora com uma linha no log: sem `job.json`, JSON inválido, versão > 1, id ou data inválidos e id repetido (fica o mais recente). O áudio volta `Saved` se o `job.json` tem o `audio` novo (regravado pelo "Guardar áudio", `transcript::set_manifest_audio`) e o arquivo existe; senão `Discarded`. Com mais de 50, o fim da lista diz "As mais antigas continuam em Documentos\Scriba." com o "Abrir pasta" (`POST /api/app/saidas`, `older-note` no design). O `finished` fica na `Entry` do store, não no `Job`, para não mexer nos ~15 literais de `Job` | Decisões 3 e 13; `tests/historico.rs` com as fixtures de `tests/fixtures/historico/`, E2E "a transcrição continua na lista depois de reabrir o app" |
| 2026-10-21 | Fase 7 | Etapa 7.3, `config.json` v1: `version: 1`; a v0 (sem `version`) migra direto; leitura campo a campo (um `theme` inválido não apaga mais o microfone e a pasta do áudio); JSON corrompido copiado para `config.json.invalido`; chaves desconhecidas e uma versão mais nova preservadas; gravação atômica (`write_atomic`) | Decisão 14 |
| 2026-10-21 | Fase 7 | Etapa 7.3, prazo do áudio e limpeza: trait `Clock` (`SystemClock`, `FakeClock`); o store marca a hora da conclusão e `Jobs::expire_audio` passa a pergunta pendente há 24 h a `Audio::Expired` (evento `Expire`, só a partir de `Pending`: a janela aberta e a gravação do `.m4a` não expiram) e apaga a mídia (o envio ou a pasta da gravação). O zelador roda a cada 10 min no `app.rs`. Na abertura, `src/cleanup.rs` apaga os temporários do `--self-test` em `%TEMP%` e os `*.m4a.parte` nas pastas do histórico e na `audio_dir` com mais de 1 dia | Decisões 2 e 15; `tests/jobs.rs` `audio_sem_resposta_expira_em_24_h` (23 h 59 min fica, 24 h sai) |

## Referências verificadas

- [cpal: loopback WASAPI](https://github.com/RustAudio/cpal/blob/master/src/host/wasapi/device.rs): `AUDCLNT_STREAMFLAGS_LOOPBACK` em stream de entrada aberto em dispositivo de saída.
- [ffmpeg-sys-next: build.rs](https://github.com/zmwangx/rust-ffmpeg-sys/blob/master/build.rs): vcpkg ou `FFMPEG_DIR`, feature `static`, bindgen.
- [ffmpeg-next no crates.io](https://crates.io/crates/ffmpeg-next) e [README do rust-ffmpeg](https://github.com/zmwangx/rust-ffmpeg): versão 9.0.0 (ago/2026) e modo manutenção.
- [vcpkg: port do ffmpeg](https://github.com/microsoft/vcpkg/tree/master/ports/ffmpeg): FFmpeg 9.0.2; features `x264`, `gpl` e `nvcodec`; NVENC carregado em runtime, sem CUDA SDK.
- [whisper-rs 0.16](https://docs.rs/whisper-rs/latest/whisper_rs/): features, VAD, callbacks e `new_from_buffer_with_params`.
- [Guia de build estático do FFmpeg no Windows](https://docs.rs/crate/ez-ffmpeg/0.17.0/source/docs/INSTALL.md): triplet `x64-windows-static-md`.
