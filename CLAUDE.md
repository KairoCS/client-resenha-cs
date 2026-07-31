# CLAUDE.md — Resenha Client

@AI_RULES.md

---

## O que é este projeto

App de Windows que roda na bandeja, lê o CS2 via **GSI** (Game State
Integration) e manda os eventos da partida pro backend por WebSocket. É o que
permite o placar ao vivo e o registro automático do resultado.

Faz parte de um sistema de três peças:

```
Navegador ──> resenha-cs-next (Vercel) ──┐
                                          ├──> Turso (banco compartilhado)
CS2 ──GSI──> Resenha Client (AQUI) ──WS──> resenha-backend (Render)
```

- **backend** (`backend-cs-resenha`): manda `START_MATCH`/`END_MATCH`, recebe
  os eventos, decide a versão mínima do client.
- **site** (`resenha-cs`): gera o código de pareamento e serve o instalador.

## A promessa de privacidade (regra inegociável)

O app fica ligado o tempo todo, mas **só processa eventos de partidas criadas
pela plataforma**. Premier, Competitivo, Casual e Deathmatch são ignorados —
nada desses modos sai da máquina do usuário.

Sustentado por quatro filtros em ordem, em `gsi/servidor.rs`:

1. **Token secreto** — só o CS2 configurado por nós é aceito;
2. **Versão do app** — client desatualizado não coleta nada;
3. **Partida ativa** — sem `matchId` da plataforma, o evento é descartado;
4. **Escopo do jogador** — cada client manda só os próprios dados
   (`events.rs` compara `player.steamid` com `provider.steamid`).

**Nunca amplie o que é coletado por conveniência técnica.** Só com pedido
explícito do usuário.

## Stack

Tauri 2 + Rust (edition 2021). UI mínima em TypeScript + Vite — toda a lógica
fica no Rust. Instalador NSIS, distribuído pelo site.

## Estrutura

```
src-tauri/src/
  main.rs        só monta o app; nada de lógica
  commands.rs    superfície que a UI invoca
  gsi/           instalacao.rs (o .cfg) + servidor.rs (recebe do CS2)
  ws.rs          conexão com o backend (reconecta com backoff)
  auth.rs        pareamento e tokens (Windows Credential Manager)
  config.rs      config.json + endereço do backend (fixo no binário)
  state.rs       estado compartilhado + StatusPayload pra UI
  update.rs      versão obrigatória
  events.rs      diff do GSI → eventos de domínio
  tray.rs        ícone na bandeja
src/             UI (main.ts, styles.css) + index.html
```

## Princípios e padrões

Clean Code, SOLID, DRY, KISS e YAGNI — **até onde pagam**.

- **S**: `main.rs` só monta o app; `commands.rs` é a superfície da UI; `gsi/`
  separa instalar o `.cfg` de receber do CS2. Nada de lógica no `main`.
- **I**: `StatusPayload` expõe pra UI só o que ela desenha — não vaze estado
  interno (o endereço do backend saiu daqui de propósito).
- **DRY** vale para duplicação **real**. Atenção: a lógica de comparar versão
  existe em Rust (`update.rs`) e em JS (`version.js` do backend) — é
  duplicação **inevitável** entre linguagens; se mudar uma, mude a outra.
- **YAGNI**: a escala real é uma sala de 10 jogadores. Não construa para 10 mil.

**Adotados:** state compartilhado via `AppState` + `Manager` (idioma do Tauri),
canais `watch` para acordar tarefas (sessão, atualização), backoff com teto na
reconexão.

**Rejeitado de propósito** — não reintroduza sem argumentar o ganho concreto:
**container de injeção de dependência**. `AppState` via `Manager` é o padrão do
Tauri; trocar por traits deixaria o código estranho para quem conhece o
framework.

## Regras deste projeto

- **O `.cfg` do GSI é fixo.** Depois de instalado fica na pasta do CS2 para
  sempre (o jogo só lê `gamestate_integration_*.cfg` ao iniciar). Só reescreva
  se o conteúdo divergir — trocar o token à toa obriga a reiniciar o jogo.
- **O `gsi_token` do `config.json` não pode ser regerado à toa.** Se o parse do
  config falhar, o `load()` cai no default, gera token novo e reescreve o
  `.cfg` — que só passa a valer depois de reiniciar o CS2. Há teste para isso
  (`config_antigo_com_backend_url_continua_carregando`).
- **O endereço do backend é fixo no binário** (`config::backend_url()`), fora
  do `config.json`, da UI e do payload de status. Em dev, compile com
  `RESENHA_BACKEND_URL=http://localhost:4000`.
- **A interface `Status` (`src/main.ts`) espelha o `StatusPayload`
  (`state.rs`).** Mudou um, mude o outro — não há checagem automática, e a
  divergência só aparece em runtime.
- Nada de `any` no TypeScript.
- O servidor GSI escuta **somente** em `127.0.0.1`. Nunca exponha na rede.
- Comparação de segredo em tempo constante (`constant_time_eq`).
- Nunca passe string externa pro shell sem validar (`baixar_atualizacao` exige
  `http(s)://`).

## Versão obrigatória

O backend informa a versão atual em `GET /api/client/version`. Client mais
antigo é **bloqueado**: não conecta no WebSocket, descarta todo evento do CS2 e
a UI trava numa tela de atualização. A checagem roda no boot e a cada 2h, e o
backend reforça no `HELLO`.

Instalar por cima preserva o pareamento (os tokens ficam no Credential
Manager, não no diretório do app).

## Comandos

```bash
npm run tauri dev

cd src-tauri
cargo check                       # tem que passar SEM warnings
cargo test --bin resenha-client
cd .. && npx tsc --noEmit

node release.mjs 0.3.0            # bump nos 3 manifests + build + copia pro site
```

## Testes

- `cargo check` passar **não prova que o app roda**. Caminho tocado precisa ser
  exercitado de verdade: suba o app e confira que o servidor GSI responde na
  3210, que o `gsi_token` foi preservado e que os filtros rejeitam token
  errado.
- Correção de bug entra com teste que **falha antes** da correção.

## Publicar uma versão

1. `node release.mjs 0.3.0` — sobe a versão em `Cargo.toml`,
   `tauri.conf.json` e `package.json`, builda o NSIS e copia pro site.
2. Deploy do **site** (leva o instalador).
3. Só então suba `CLIENT_LATEST_VERSION` no Render.

**Nunca inverta a ordem** — bloquearia todo mundo sem ter para onde ir.

**Refatoração sem mudança de comportamento não justifica bump de versão:**
forçaria reinstalação inútil.

O instalador não é assinado; o SmartScreen alerta na primeira execução.

## Checklist antes de entregar

- [ ] `cargo check` sem warnings.
- [ ] `cargo test --bin resenha-client` passando.
- [ ] `npx tsc --noEmit` limpo.
- [ ] **App executado de verdade** (GSI na 3210, token preservado, filtros ok).
- [ ] Nada órfão: import, CSS, arquivo temporário.
- [ ] Processo do client encerrado.
- [ ] Nenhum segredo em código, log ou saída.
