# Scriba — Roadmap

App local para Windows, distribuído como um único `.exe`, que **grava** (microfone + som do PC), **compacta** (vídeos e áudios) e **transcreve tudo o que foi falado**, entregando o **texto**. Este documento reúne as decisões de produto e de arquitetura, as fases de desenvolvimento com checklists, a política de testes, os riscos e o registro de decisões.

Última atualização: 2026-10-17.

## Status das fases

| Fase | Tema | Status |
| --- | --- | --- |
| 0 | Fundação e spikes de risco | Concluída |
| 1 | CI/CD no GitHub | Concluída |
| 2 | Esqueleto do app e design system | Não iniciada |
| 3 | Fila de jobs e upload | Não iniciada |
| 4 | Transcrição de arquivos (MVP) | Não iniciada |
| 5 | Gravação ao vivo | Não iniciada |
| 6 | Compactação de vídeo e áudio | Não iniciada |
| 7 | Robustez, empacotamento e entrega final | Não iniciada |

Valores possíveis: Não iniciada, Em andamento, Concluída.

## Como vamos trabalhar

- **Uma fase por vez, separadamente.** Cada fase começa com um plano detalhado próprio, aprovado antes da execução, e só termina quando os critérios "Pronto quando" e os testes passam. Só então a próxima começa.
- **Ao abrir uma fase:** status "Em andamento" na tabela acima.
- **Ao fechar uma fase:**
  - Marcar os checklists.
  - Mudar o status para "Concluída".
  - Registrar as decisões e os números medidos no [Registro de decisões](#registro-de-decisões).
- **A partir da Fase 1, uma branch e um PR por fase**, com o CI verde antes do merge.
- **Definition of Done:** veja [Qualidade e testes](#qualidade-e-testes).
- Se um spike ou um teste derrubar uma decisão (por exemplo, se o exe único se mostrar inviável), este roadmap é revisado antes de seguir.

## Visão e escopo

Decisões de produto:

- **Objetivo:** gravar, compactar e transcrever. Os três são núcleo.
- **Saída:** a transcrição em texto de tudo que foi falado, venha de áudio, de vídeo ou da gravação ao vivo. O vídeo **não** recebe legendas.
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
- A gravação vem antes da compactação de vídeo, porque já entrega a compactação do áudio gravado.

## Decisões de arquitetura

| Tema | Decisão | Consequência |
| --- | --- | --- |
| Plataforma | Windows 10/11 x64 na v1 | Loopback WASAPI suportado pelo cpal. macOS/Linux ficam no backlog |
| Transcrição | `whisper-rs` 0.16 com o modelo **large-v3-turbo q5_0 (~0,6 GB) embutido no .exe**, carregado da memória (`WhisperContext::new_from_buffer_with_params`) | Invisível e offline. O .exe fica com ~0,65 GB |
| Aceleração | Backend **Vulkan** (o `vulkan-1.dll` vem com o driver da GPU), com fallback para CPU | CUDA descartado: exigiria DLLs de centenas de MB ao lado do exe |
| Mídia | `ffmpeg-next` 9 + FFmpeg 9 **estático** via vcpkg com baseline fixado no repo: `ffmpeg[core,avcodec,avformat,swresample,swscale,x264,gpl,nvcodec]` | Sem DLLs do FFmpeg. O binário distribuído é GPL (veja a Fase 1) |
| Captura | `cpal` (WASAPI): mic e loopback em trilhas separadas, WAV mono 16-bit gravado no disco | À prova de crash. Ocupa ~345 MB/h por trilha até a compactação |
| Web | `axum` 0.8 + `askama` + htmx 2 + Tailwind v4 compilado + fonte Inter (substituta do SF Pro indicada pelo design doc), tudo embutido | Nenhum arquivo solto. Funciona offline |
| Estado | Fila em memória (`Arc<RwLock<HashMap>>` + `mpsc`), mais um manifesto `job.json` por saída | O histórico sobrevive a reinícios sem precisar de banco |
| Saídas | `Documentos\Scriba\AAAA-MM-DD HHmm - nome\` (`transcricao.txt`, `audio.m4a`/`video.mp4`). Temporários em `%LOCALAPPDATA%\Scriba` | Todos os caminhos são configuráveis, o que deixa os testes isolados |
| Compressão | Vídeo: x264 CRF ~26 (padrão) ou NVENC ("rápido"), até 1080p, áudio AAC. Áudio e gravações: AAC `.m4a` 96 kbps mono | Mantém o original se o ganho for menor que 5% |
| Testabilidade | Traits `Transcriber`, `MediaEngine`, `AudioSource` e `Clock`. Feature `test-support` com fakes e a flag `--fake-audio`, fora do release | Toda a lógica roda no CI, sem placa de som nem GPU |

## Arquitetura

```text
Navegador (HTMX)  ⇄  Axum @127.0.0.1 [token + Host/Origin]
                        │ upload em streaming → %LOCALAPPDATA%\Scriba\uploads
                        ▼
                 JobStore (HashMap em memória) ──mpsc──▶ Worker (thread dedicada, 1 job por vez)
                        ▲                                   ├─ media::extract_audio  (FFmpeg → 16 kHz mono f32)
                        │                                   ├─ asr::transcribe       (whisper + VAD, progresso/cancelar)
                        │                                   ├─ transcript::write     (.txt com [hh:mm:ss] e Você/Outros)
                        │                                   └─ media::compress       (H.264/AAC)
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

- [ ] Axum em 127.0.0.1 (porta preferida e fallback), com encerramento limpo.
- [ ] Middleware de segurança: token na URL → cookie SameSite=Strict; validação de Host e Origin.
- [ ] Assets embutidos (CSS, htmx, Inter, ícones) com cache.
- [ ] Layout askama (global-nav, tiles, footer) seguindo o `DESIGN-scriba.md`.
- [ ] Logo vetorizado a partir de `docs/brand/logo-original.jpg`: SVG do ícone, versão branca e versão simplificada de 16 px (favicon e bandeja).
- [ ] `cargo xtask css`: gera os tokens a partir do YAML (tema claro em `:root`, escuro em `[data-theme="dark"]` e em `prefers-color-scheme`) e compila o Tailwind.
- [ ] Alternância de tema no header (Sistema → Claro → Escuro), salva na configuração e renderizada pelo servidor no `<html>`, sem piscar.
- [ ] Instância única via `instance.json`.
- [ ] Flags `--no-open`, `--port`, `--data-dir` e `--output-dir`.

**Testes:**

- [ ] Unitários: token, Host e Origin; fallback de porta; lockfile; gerador de tokens CSS (snapshot).
- [ ] Integração: rotas com 200/401/403; content-type e cache dos assets; segunda instância.
- [ ] E2E: a página carrega e bate com a referência visual nos dois temas; a alternância de tema persiste após recarregar.
- [ ] Unitário: contraste AA de todo par texto/fundo dos componentes, nos dois temas, calculado a partir do YAML.

**Pronto quando:**

- [ ] `cargo run` abre a tela fiel ao design.
- [ ] Requisição sem token retorna 401, e Host estranho retorna 403.
- [ ] A segunda execução reaproveita a instância aberta.

### Fase 3 — Fila de jobs e upload

Base no PRD: Sprint 2 e o polling da Sprint 4. Objetivo: arquivos entram por upload, viram jobs em uma fila em memória e a UI acompanha o andamento, ainda com um pipeline falso.

**Entregas:**

- [ ] Modelo de job: id ULID, tipo, opções (transcrever/compactar), etapa, %, erro e saídas.
- [ ] Worker em `std::thread` com `blocking_recv`, `catch_unwind` por job e cancelamento por `AtomicBool`.
- [ ] Upload multipart em streaming para o disco (limite de corpo desativado só nessa rota).
- [ ] Rotas `POST /api/jobs`, `GET /api/jobs`, `GET /api/jobs/{id}`, `POST /api/jobs/{id}/cancel` e `DELETE /api/jobs/{id}`.
- [ ] Polling de 1 s só para jobs ativos; a resposta 286 encerra o polling.
- [ ] Pipeline falso atrás do trait, para validar a UI antes dos motores.

**Testes:**

- [ ] Unitários: máquina de estados (proptest) e semântica de cancelamento.
- [ ] Integração: upload grande gravado no disco em streaming, sem acumular na memória; panic no pipeline vira FALHOU.
- [ ] E2E: upload → progresso → concluído; cancelar; falha exibida.

**Pronto quando:**

- [ ] Um upload de 2 GB mantém a RAM estável.
- [ ] Progresso, cancelamento e falha aparecem corretos na UI.

### Fase 4 — Transcrição de arquivos (MVP)

Base no PRD: Sprint 3.1 e Sprint 4. Objetivo: qualquer áudio ou vídeo vira texto em pt-BR. É a primeira entrega de valor.

**Entregas:**

- [ ] `media::extract_audio`: melhor faixa de áudio → 16 kHz mono f32, com progresso por pts e erros amigáveis (ex.: "arquivo sem áudio").
- [ ] `asr`:
  - Contexto carregado sob demanda e liberado após 10 min ocioso.
  - Idioma `pt`, VAD Silero e `no_context`.
  - Threads = núcleos − 2.
  - Progresso, cancelamento e filtro de frases-fantasma.
- [ ] Escolha do backend (achado do spike A: só CPU com o modelo grande leva ~8 h para 1 h de áudio):
  - Usar a GPU **dedicada** quando houver, e não o dispositivo Vulkan 0, que costuma ser a integrada. Se não houver dedicada, usar a integrada.
  - Sem GPU Vulkan, transcrever na CPU com um **modelo menor embutido**. A escolha entre `small` e `base` sai de um benchmark de precisão × tempo nas fixtures. O custo é o exe crescer ~60–190 MB.
  - Antes de fechar o fallback, investigar a lentidão da CPU (~87 s para 10,9 s), comparando com o whisper.cpp oficial e testando threads e OpenMP.
  - Mostrar na UI qual backend está em uso e uma estimativa de tempo antes de começar.
- [ ] `transcript`: parágrafos (pausa > 1,5 s quebra), `[hh:mm:ss]` em cada parágrafo e cabeçalho (origem, data, duração). Gera `transcricao.txt` e `job.json`.
- [ ] UI: leitor da transcrição, Copiar texto, Baixar .txt e Abrir pasta.

**Testes:**

- [ ] Unitários: parágrafos, formato de tempo, filtro de frases-fantasma e cabeçalho (snapshots).
- [ ] Integração com o modelo pequeno:
  - Fixtures MP4, MP3, M4A, OGG e WAV → palavras-chave.
  - Silêncio puro → texto vazio, sem frases-fantasma.
  - Cancelar no meio interrompe a transcrição.
  - Arquivo sem áudio → erro amigável.
- [ ] E2E: upload → texto visível → copiar → .txt baixado com o conteúdo certo.
- [ ] Unitário: a escolha do backend prefere GPU dedicada, depois integrada, depois CPU com o modelo menor (com uma lista de dispositivos falsa).
- [ ] Local: benchmark com 1 h de áudio real em cada backend (RTX, Intel integrada e só CPU).

**Pronto quando:**

- [ ] Arquivos reais (MP4 do Teams, MP3, M4A, WAV, OGG) viram texto pt-BR correto.
- [ ] O tempo para transcrever 1 h de áudio na RTX 3050 está registrado.
- [ ] Metas para 1 h de áudio: ≤ 10 min na RTX 3050 e ≤ 1 h só na CPU (com o modelo menor). Os tempos medidos nos três backends ficam no registro de decisões.

### Fase 5 — Gravação ao vivo

Base no PRD: Sprint 5. Objetivo: um clique grava o microfone e o som do PC; ao parar, saem a transcrição rotulada "Você" / "Outros" e o áudio compactado.

**Entregas:**

- [ ] Lista de dispositivos (padrões marcados), com escolha nos ajustes.
- [ ] Recorder com três streams:
  - Mic.
  - Loopback: stream de entrada aberto no dispositivo de saída com `default_output_config`.
  - Silêncio tocando na mesma saída.

  Os callbacks só copiam para ring buffers SPSC (`rtrb`).
- [ ] Thread de escrita: downmix para mono, conversão para i16, WAV via `hound` com `flush` a cada 5 s. Alinhamento por timestamp. Medidores RMS.
- [ ] Rotas `POST /api/record/start`, `POST /api/record/stop` e `GET /api/record/status`.
- [ ] UI: tile escuro com botão grande e barra fixa "● Gravando 00:12:34 · Parar".
- [ ] Job de gravação:
  - Mixdown para `audio.m4a`.
  - Transcrição de cada trilha, mesclada por tempo com os rótulos Você/Outros.
  - Remoção do eco e geração do `transcricao.txt`.
  - WAVs apagados após o sucesso.
- [ ] Robustez:
  - Dispositivo removido: salva o que foi gravado.
  - Troca da saída padrão: reabre o loopback.
  - Checagem de espaço em disco e teto de ~12 h (limite de 4 GB do WAV).

**Testes:**

- [ ] Unitários: downmix e i16; alinhamento e preenchimento; mescla e dedupe de eco (proptest); teto de duração.
- [ ] Integração com `AudioSource` falso:
  - Lacunas e jitter → trilhas alinhadas (±10 ms).
  - Erro de dispositivo → arquivo finalizado.
  - Fixtures "Você"/"Outros" + eco → texto rotulado e sem duplicatas.
- [ ] E2E com `--fake-audio`: Gravar → timer → Parar → transcrição rotulada.
- [ ] Local: mic e loopback reais com 15 s de silêncio no meio.

**Pronto quando:**

- [ ] Falar com música ou reunião tocando, clicar em Parar e receber o texto com as duas vozes rotuladas.
- [ ] 15 s de silêncio no meio da gravação não desalinham as trilhas.

### Fase 6 — Compactação de vídeo e áudio

Base no PRD: Sprint 3.2 e Sprint 4.2. Objetivo: vídeos e áudios enviados ficam menores.

**Entregas:**

- [ ] `media::compress_video`:
  - Escala até 1080p, mantendo a proporção, com dimensões pares.
  - Codifica com x264 CRF 26 (padrão) ou NVENC p5 CQ 28.
  - Áudio AAC, MP4 `faststart`, progresso e cancelamento.
- [ ] `media::compress_audio` (reaproveitado da Fase 5) para áudios enviados.
- [ ] O original é mantido quando o ganho for menor que 5%.
- [ ] UI: tamanho antes → depois (−xx%).

**Testes:**

- [ ] Unitários: regra dos 5%, cálculo de dimensões e preset → parâmetros.
- [ ] Integração: vídeo sai H.264 com as dimensões e a duração esperadas (±0,1 s) e menor que o original; áudio sai AAC; cancelar no meio interrompe.
- [ ] E2E: upload com "Compactar" → tamanho antes/depois na tela.
- [ ] Local: NVENC.

**Pronto quando:**

- [ ] Uma gravação de 1 h do Teams fica menor e abre no Media Player e no VLC.

### Fase 7 — Robustez, empacotamento e entrega final

Objetivo: o `scriba.exe` final roda em qualquer Windows sem instalar nada e se comporta como um app de bandeja.

**Entregas:**

- [ ] `windows_subsystem = "windows"`, ícone na bandeja (Abrir/Sair), ícone e metadados do exe (`winresource`).
- [ ] Logs com rotação em `%LOCALAPPDATA%\Scriba\logs` e panic hook.
- [ ] Histórico reconstruído a partir dos `job.json`, `config.json` e limpeza de temporários antigos.
- [ ] `release.yml` completo: Vulkan SDK no runner, modelo embutido e `--self-test` exercitando também o fallback para CPU (o runner não tem GPU).
- [ ] `cargo xtask dist` faz o mesmo build localmente.
- [ ] README pt-BR e `CLAUDE.md` com os comandos reais.

**Testes:**

- [ ] Unitários: config (carga, defaults e migração); limpeza por idade com `Clock` falso; ações do menu da bandeja via canal.
- [ ] Integração: histórico a partir de `job.json` de fixture; `--self-test`.
- [ ] Local: `cargo xtask sandbox-test` roda o exe no Windows Sandbox (máquina limpa) e lê o resultado.

**Pronto quando:**

- [ ] A tag gera uma release que passa no smoke test do CI e no Sandbox, com o fluxo completo de gravar, compactar e transcrever.

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

## Referências verificadas

- [cpal: loopback WASAPI](https://github.com/RustAudio/cpal/blob/master/src/host/wasapi/device.rs): `AUDCLNT_STREAMFLAGS_LOOPBACK` em stream de entrada aberto em dispositivo de saída.
- [ffmpeg-sys-next: build.rs](https://github.com/zmwangx/rust-ffmpeg-sys/blob/master/build.rs): vcpkg ou `FFMPEG_DIR`, feature `static`, bindgen.
- [ffmpeg-next no crates.io](https://crates.io/crates/ffmpeg-next) e [README do rust-ffmpeg](https://github.com/zmwangx/rust-ffmpeg): versão 9.0.0 (ago/2026) e modo manutenção.
- [vcpkg: port do ffmpeg](https://github.com/microsoft/vcpkg/tree/master/ports/ffmpeg): FFmpeg 9.0.2; features `x264`, `gpl` e `nvcodec`; NVENC carregado em runtime, sem CUDA SDK.
- [whisper-rs 0.16](https://docs.rs/whisper-rs/latest/whisper_rs/): features, VAD, callbacks e `new_from_buffer_with_params`.
- [Guia de build estático do FFmpeg no Windows](https://docs.rs/crate/ez-ffmpeg/0.17.0/source/docs/INSTALL.md): triplet `x64-windows-static-md`.
