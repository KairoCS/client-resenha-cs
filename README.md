# Resenha Client

Aplicativo desktop (Windows) que integra o Counter-Strike 2 com a plataforma
Resenha via **Game State Integration (GSI)**. Roda na bandeja do sistema, em
segundo plano, e **só processa eventos quando existe uma partida criada pelo
site** — Premier, Competitivo, Casual e Deathmatch são ignorados e nada sai da
máquina.

Feito com **Tauri 2 + Rust + TypeScript** (sem Electron): binário pequeno,
consumo mínimo de RAM/CPU.

## Arquitetura

```
Next.js (Vercel)  --HTTPS-->  Backend API (dedicado)
                                 |  REST + WebSocket
                                 |
                              Resenha Client (Tauri)
                                 |  HTTP local 127.0.0.1:3210
                                 |
                              Counter-Strike 2 (GSI)
```

## Módulos (src-tauri/src)

| Arquivo     | Responsabilidade |
|-------------|------------------|
| `main.rs`   | bootstrap, comandos da UI, janela (fechar = esconder), autostart |
| `config.rs` | Config Manager — `%APPDATA%\ResenhaClient\config.json` |
| `auth.rs`   | Authentication + Token Manager — tokens no **Windows Credential Manager** |
| `ws.rs`     | WebSocket Manager — reconexão com backoff (1s→60s), refresh automático em 401, START_MATCH/END_MATCH |
| `gsi.rs`    | GSI Manager + Local HTTP Server (127.0.0.1:3210) + Security (token secreto, comparação em tempo constante) |
| `events.rs` | Event Dispatcher — diff de payloads GSI → eventos de domínio |
| `state.rs`  | Match Controller + status pra UI |
| `tray.rs`   | Tray Manager — bandeja, menu, autostart |

## Como compilar

Pré-requisitos (uma vez):

1. **Rust**: https://rustup.rs (`winget install Rustlang.Rustup`)
2. **Visual Studio Build Tools** com "Desktop development with C++"
   (`winget install Microsoft.VisualStudio.2022.BuildTools`) — o rustup instrui na instalação
3. **Node.js 18+** (já instalado)
4. WebView2 (já vem no Windows 11)

```
npm install
npm run tauri dev      # desenvolvimento
npm run tauri build    # instalador NSIS em src-tauri/target/release/bundle/nsis/
```

## Fluxo de funcionamento

1. Primeira execução: o usuário loga no **site** com a Steam, gera um código
   de conexão no perfil e digita no client. Os tokens recebidos ficam no
   Credential Manager — nunca mais precisa logar.
2. O app instala `gamestate_integration_resenha.cfg` na pasta do CS2
   (detecta a Steam pelo registro + `libraryfolders.vdf`). O arquivo é **fixo**:
   criado uma vez, nunca removido, reescrito só se divergir.
3. Conecta ao backend via WebSocket e fica em espera na bandeja.
4. `START_MATCH` → habilita o processamento GSI com o `matchId` recebido.
5. Eventos do CS2 viram eventos de domínio e vão pro backend em tempo real.
6. `END_MATCH` → limpa o estado e volta à espera. O `.cfg` permanece.

Sem partida ativa, os POST do CS2 são respondidos com `200 OK` e descartados —
custo praticamente zero e **nenhum dado sai da máquina**.

## Segurança

- **GSI local**: o `.cfg` inclui um token secreto aleatório (gerado na
  instalação); todo POST é validado com comparação em tempo constante.
  O servidor escuta **somente** em `127.0.0.1` — nunca exposto à rede.
- **Tokens de sessão**: Windows Credential Manager (não ficam em arquivo).
- **Payload**: precisa ser JSON válido; formato inválido → `400`.

---

## Contrato do backend (a implementar no servidor dedicado)

> O site autentica via **Steam OpenID + cookie** (não existe senha), então o
> client usa **pareamento por código**: o usuário loga no site com a Steam,
> gera um código curto no perfil ("Conectar Resenha Client") e digita no
> client uma única vez. O backend troca o código pelos tokens.
>
> Fluxo a implementar no lado do site/backend:
> 1. Site (logado via cookie) → `POST /api/client/pair-code` → gera código
>    curto (ex.: 6 chars, expira em 5 min, uso único) vinculado ao steamid.
> 2. Client → `POST /api/client/auth/pair { code }` no backend dedicado →
>    valida o código no banco compartilhado → emite os tokens.

### REST (backend dedicado)

| Endpoint | Body | Resposta |
|---|---|---|
| `POST /api/client/auth/pair` | `{ "code" }` | `200 { "access_token", "refresh_token" }` · `401` código inválido/expirado |
| `POST /api/client/auth/refresh` | `{ "refresh_token" }` | `200` novo par de tokens · `401` sessão expirada (client volta pro login) |

### WebSocket — `{backend}/ws/client`

Handshake com header `Authorization: Bearer <access_token>`.
Responder `401` no upgrade quando o token estiver expirado (o client renova e reconecta).

**Backend → Client:**

```json
{ "type": "START_MATCH", "matchId": 52 }
{ "type": "END_MATCH" }
{ "type": "PING" }
```

> **Ressincronização (obrigatório):** ao receber o `HELLO` de um client, o
> backend deve reenviar o estado atual — `START_MATCH` se aquele jogador tem
> partida em andamento, `END_MATCH` caso contrário. O client não persiste o
> `matchId` entre reinícios, e pode ter perdido um `END_MATCH` enquanto estava
> offline; sem esse reenvio ele ficaria dessincronizado (parado ou coletando
> à toa). Como o client reenvia `HELLO` a cada reconexão, isso resolve os dois
> casos.

**Client → Backend:**

| Tipo | Quando |
|---|---|
| `HELLO` | ao conectar (`client`, `version`) |
| `MATCH_ACK` / `MATCH_ENDED_ACK` | confirmação de START/END_MATCH |
| `PONG` | resposta ao PING de aplicação |

**Eventos de partida** (todos com `matchId`, `timestamp` em ms e `data`):

| Tipo | data |
|---|---|
| `MAP_CHANGE` | `map`, `mode` |
| `MAP_PHASE` | `phase` (`warmup`/`live`/`intermission`/`gameover`) |
| `FREEZETIME` / `ROUND_START` | `round` (1-indexado) |
| `ROUND_END` | `round`, `winner` (`CT`/`T`), `score_ct`, `score_t` |
| `SCORE_UPDATE` | `score_ct`, `score_t` |
| `BOMB_PLANTED` / `BOMB_DEFUSED` / `BOMB_EXPLODED` | — |
| `PLAYER_KILL` / `PLAYER_ASSIST` / `PLAYER_MVP` | `total`, `delta` |
| `PLAYER_DEAD` | `deaths` (total) |
| `PLAYER_ALIVE` | — |
| `WEAPON_CHANGE` | `weapon` — **só `weapon_c4`**: é o que atribui o plant, e nenhuma outra arma é lida |
| `PLAYER_TEAM` | `team` (troca de lado) |
| `GAME_OVER` | `score_ct`, `score_t` |
| `STATE_SYNC` | estado condensado a cada 10s (mapa, placar, stats do jogador) — deixa o backend se recuperar de eventos perdidos |

Cada jogador envia apenas os **próprios** eventos (GSI de quem joga não expõe
`allplayers`); o backend agrega os 10 clients da partida. O `STATE_SYNC`
periódico garante consistência mesmo com quedas de conexão.

**Não existe evento de dano.** O CS2 não expõe dano por round: capturamos 27
payloads de uma partida competitiva ao vivo e o `player.state` traz `health`,
`armor`, `helmet`, `flashed`, `smoked`, `burning`, `money`, `round_kills`,
`round_killhs` e `equip_value` — o `round_totaldmg` é do CS:GO. Há teste
travando isso (`nenhum_evento_de_dano_e_emitido`), para ninguém reintroduzir
um ADR fantasma no cálculo do elo.

**O backend descarta parte do que chega.** `MAP_CHANGE`, `MAP_PHASE`,
`SCORE_UPDATE`, `PLAYER_ALIVE`, `PLAYER_TEAM` e `BOMB_EXPLODED` são aceitos e
não gravados — tudo que eles diriam já está no `STATE_SYNC`. Não é erro, e não
vira log. Se algum dia forem necessários, basta o backend voltar a gravá-los,
sem precisar de versão nova do client.

## Configuração local

`%APPDATA%\ResenhaClient\config.json`:

```json
{
  "gsi_port": 3210,
  "gsi_token": "<gerado automaticamente>"
}
```

O endereço do backend é **fixo no binário** (definido em `src/config.rs`): não
fica no config.json, não aparece na interface e não vai no payload de status.
Quem instala pelo site não configura nada. Pra desenvolver contra um backend
local, compile apontando pra ele:

```bash
RESENHA_BACKEND_URL=http://localhost:4000 npm run tauri build
```

Logs em `%APPDATA%\ResenhaClient\logs\` (rotação diária).

## Versão e atualização obrigatória

O backend informa a versão atual em `GET /api/client/version`. Se este binário
for mais antigo, o app **trava numa tela de atualização**: não conecta no
WebSocket e descarta todo evento do CS2 até o usuário instalar a versão nova
(o instalador é baixado do próprio site). A checagem roda no boot e a cada 2h,
e o backend reforça no HELLO — não dá pra contornar deixando o app aberto.

Pra publicar uma versão: `node release.mjs 0.3.0` (sobe a versão nos três
manifests, builda e copia o instalador pro site).
