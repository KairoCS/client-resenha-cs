// events.rs — Event Dispatcher: transforma o diff entre dois payloads GSI em
// eventos de domínio (ROUND_START, PLAYER_KILL, BOMB_PLANTED…).
//
// Detalhe importante do GSI: quando o jogador morre e passa a observar um
// colega, o bloco "player" vira o jogador OBSERVADO. Por isso todo evento de
// jogador só é emitido quando player.steamid == provider.steamid.
use serde_json::{json, Value};

pub struct GameEvent {
    pub kind: &'static str,
    pub data: Value,
}

fn ev(kind: &'static str, data: Value) -> GameEvent {
    GameEvent { kind, data }
}

fn s<'a>(v: &'a Value, ptr: &str) -> Option<&'a str> {
    v.pointer(ptr).and_then(Value::as_str)
}

fn i(v: &Value, ptr: &str) -> Option<i64> {
    v.pointer(ptr).and_then(Value::as_i64)
}

fn scores(v: &Value) -> (Option<i64>, Option<i64>) {
    (i(v, "/map/team_ct/score"), i(v, "/map/team_t/score"))
}

/// O bloco "player" é do dono desta máquina? Quando a pessoa morre e passa a
/// observar um colega, o GSI troca o bloco pelo jogador OBSERVADO — e aquelas
/// stats não são dela.
fn is_own(v: &Value) -> bool {
    match (s(v, "/provider/steamid"), s(v, "/player/steamid")) {
        (Some(owner), Some(pid)) => owner == pid,
        _ => false,
    }
}

/// Arma com state == "active" no bloco player.weapons.
fn active_weapon(v: &Value) -> Option<String> {
    let weapons = v.pointer("/player/weapons")?.as_object()?;
    weapons
        .values()
        .find(|w| w.get("state").and_then(Value::as_str) == Some("active"))
        .and_then(|w| w.get("name").and_then(Value::as_str))
        .map(str::to_owned)
}

pub fn extract(prev: Option<&Value>, curr: &Value) -> Vec<GameEvent> {
    let mut out = Vec::new();

    // ----- mapa / fase da partida -----
    let map_name = s(curr, "/map/name");
    if map_name.is_some() && map_name != prev.and_then(|p| s(p, "/map/name")) {
        out.push(ev(
            "MAP_CHANGE",
            json!({ "map": map_name, "mode": s(curr, "/map/mode") }),
        ));
    }

    let phase = s(curr, "/map/phase");
    if phase != prev.and_then(|p| s(p, "/map/phase")) {
        if let Some(ph) = phase {
            out.push(ev("MAP_PHASE", json!({ "phase": ph })));
            if ph == "gameover" {
                let (ct, t) = scores(curr);
                out.push(ev("GAME_OVER", json!({ "score_ct": ct, "score_t": t })));
            }
        }
    }

    // Aquecimento não conta: kills/rounds/placar do warmup poluiriam as stats
    // da partida. Só MAP_CHANGE/MAP_PHASE/GAME_OVER passam nessa fase.
    if phase == Some("warmup") {
        return out;
    }

    // ----- rounds -----
    // map.round é 0-indexado durante o round; ROUND_START soma 1 pra virar
    // o número "humano" do round. No ROUND_END mandamos o placar junto — o
    // placar é a fonte de verdade, o número do round é informativo.
    let round_phase = s(curr, "/round/phase");
    if round_phase != prev.and_then(|p| s(p, "/round/phase")) {
        let round = i(curr, "/map/round");
        match round_phase {
            Some("freezetime") => {
                out.push(ev("FREEZETIME", json!({ "round": round.map(|r| r + 1) })))
            }
            Some("live") => out.push(ev("ROUND_START", json!({ "round": round.map(|r| r + 1) }))),
            Some("over") => {
                let (ct, t) = scores(curr);
                out.push(ev(
                    "ROUND_END",
                    json!({
                        "round": round,
                        "winner": s(curr, "/round/win_team"),
                        "score_ct": ct,
                        "score_t": t,
                    }),
                ));
            }
            _ => {}
        }
    }

    // ----- bomba -----
    let bomb = s(curr, "/round/bomb");
    if bomb != prev.and_then(|p| s(p, "/round/bomb")) {
        match bomb {
            Some("planted") => out.push(ev("BOMB_PLANTED", json!({}))),
            Some("defused") => out.push(ev("BOMB_DEFUSED", json!({}))),
            Some("exploded") => out.push(ev("BOMB_EXPLODED", json!({}))),
            _ => {}
        }
    }

    // ----- placar -----
    if let Some(p) = prev {
        let cur = scores(curr);
        if cur.0.is_some() && cur != scores(p) {
            out.push(ev("SCORE_UPDATE", json!({ "score_ct": cur.0, "score_t": cur.1 })));
        }
    }

    // ----- jogador (somente o dono do client) -----
    let owner = s(curr, "/provider/steamid");
    let player_id = s(curr, "/player/steamid");
    if let (Some(owner), Some(pid)) = (owner, player_id) {
        if owner == pid {
            // O snapshot anterior só vale pra diff se também era o próprio jogador.
            let prev_own = prev.filter(|p| s(p, "/player/steamid") == Some(owner));

            // Stats acumulados da partida: kills/assists/mvps viram eventos de delta.
            // (deaths é coberto por PLAYER_DEAD via vida → 0, no mesmo payload.)
            for (field, kind) in [
                ("kills", "PLAYER_KILL"),
                ("assists", "PLAYER_ASSIST"),
                ("mvps", "PLAYER_MVP"),
            ] {
                let ptr = format!("/player/match_stats/{field}");
                let cur_v = i(curr, &ptr);
                let prev_v = prev_own.and_then(|p| i(p, &ptr));
                if let (Some(cv), Some(pv)) = (cur_v, prev_v) {
                    if cv > pv {
                        out.push(ev(kind, json!({ "total": cv, "delta": cv - pv })));
                    }
                }
            }

            // Vida: >0 → 0 = morreu; 0 → >0 = nasceu de novo.
            let hp = i(curr, "/player/state/health");
            let prev_hp = prev_own.and_then(|p| i(p, "/player/state/health"));
            match (prev_hp, hp) {
                (Some(ph), Some(0)) if ph > 0 => out.push(ev(
                    "PLAYER_DEAD",
                    json!({ "deaths": i(curr, "/player/match_stats/deaths") }),
                )),
                (Some(0), Some(h)) if h > 0 => out.push(ev("PLAYER_ALIVE", json!({}))),
                _ => {}
            }

            // Troca de arma ativa: só a C4 sai daqui. É ela que atribui o
            // plant (o GSI não diz quem plantou), e o backend não usa nenhuma
            // outra. Mandar toda troca fazia disso 62% da tabela de eventos,
            // com 96% de faca, rifle e pistola que ninguém lê.
            let weapon = active_weapon(curr);
            if weapon.as_deref() == Some("weapon_c4")
                && weapon != prev_own.and_then(active_weapon)
            {
                out.push(ev("WEAPON_CHANGE", json!({ "weapon": weapon })));
            }

            // Troca de lado (CT/TR) — acontece no halftime.
            let team = s(curr, "/player/team");
            if team.is_some() && prev_own.is_some() && team != prev_own.and_then(|p| s(p, "/player/team")) {
                out.push(ev("PLAYER_TEAM", json!({ "team": team })));
            }
        }
    }

    out
}

/// Estado condensado enviado periodicamente (STATE_SYNC) — deixa o backend
/// reconstruir o estado mesmo se perder eventos individuais.
pub fn condensed(v: &Value) -> Value {
    // Mesma regra dos eventos: se o bloco "player" é um colega observado
    // (jogador morto espectando), os stats dele NÃO são deste client.
    let player = if is_own(v) {
        json!({
            "steamid": s(v, "/player/steamid"),
            "name": s(v, "/player/name"),
            "team": s(v, "/player/team"),
            "health": i(v, "/player/state/health"),
            "armor": i(v, "/player/state/armor"),
            "money": i(v, "/player/state/money"),
            "kills": i(v, "/player/match_stats/kills"),
            "assists": i(v, "/player/match_stats/assists"),
            "deaths": i(v, "/player/match_stats/deaths"),
            "mvps": i(v, "/player/match_stats/mvps"),
            "score": i(v, "/player/match_stats/score"),
        })
    } else {
        Value::Null
    };
    json!({
        "map": s(v, "/map/name"),
        "mode": s(v, "/map/mode"),
        "map_phase": s(v, "/map/phase"),
        "round_phase": s(v, "/round/phase"),
        "round": i(v, "/map/round"),
        "score_ct": i(v, "/map/team_ct/score"),
        "score_t": i(v, "/map/team_t/score"),
        "player": player,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Payload do GSI com o round terminando. `observado` simula a pessoa
    /// morta assistindo um colega: o bloco player vira o do OUTRO jogador.
    fn payload_fim_de_round(observado: bool) -> Value {
        json!({
            "provider": { "steamid": "76561198000000001" },
            "map": { "name": "de_mirage", "phase": "live", "round": 7,
                     "team_ct": { "score": 5 }, "team_t": { "score": 3 } },
            "round": { "phase": "over", "win_team": "CT" },
            "player": {
                "steamid": if observado { "76561198000000002" } else { "76561198000000001" },
                "state": { "health": 0 },
                "match_stats": { "kills": 9, "assists": 2, "deaths": 6, "mvps": 1 }
            }
        })
    }

    fn achar<'a>(eventos: &'a [GameEvent], kind: &str) -> Option<&'a GameEvent> {
        eventos.iter().find(|e| e.kind == kind)
    }

    #[test]
    fn fim_de_round_gera_round_end_com_vencedor() {
        let anterior = json!({ "round": { "phase": "live" } });
        let eventos = extract(Some(&anterior), &payload_fim_de_round(false));

        let fim = achar(&eventos, "ROUND_END").expect("ROUND_END não foi emitido");
        assert_eq!(fim.data["winner"], "CT");
        assert_eq!(fim.data["round"], 7);
    }

    /// O CS2 não expõe dano por round. Capturamos 27 payloads de uma partida
    /// competitiva ao vivo (provider 14173) e o `player.state` traz apenas
    /// health, armor, helmet, flashed, smoked, burning, money, round_kills,
    /// round_killhs e equip_value. O `round_totaldmg` é do CS:GO.
    ///
    /// Este teste existe para travar a regressão: se alguém reintroduzir um
    /// evento de dano, ele quebra aqui em vez de virar ADR fantasma no elo.
    #[test]
    fn nenhum_evento_de_dano_e_emitido() {
        let anterior = json!({
            "provider": { "steamid": "76561198000000001" },
            "map": { "phase": "live", "round": 3 },
            "round": { "phase": "live" },
            "player": { "steamid": "76561198000000001", "state": { "health": 100 },
                        "match_stats": { "kills": 1, "assists": 0, "deaths": 0, "mvps": 0 } }
        });
        let morreu = json!({
            "provider": { "steamid": "76561198000000001" },
            "map": { "phase": "live", "round": 3 },
            "round": { "phase": "live" },
            "player": { "steamid": "76561198000000001", "state": { "health": 0 },
                        "match_stats": { "kills": 1, "assists": 0, "deaths": 1, "mvps": 0 } }
        });

        let eventos = extract(Some(&anterior), &morreu);
        assert!(achar(&eventos, "PLAYER_DEAD").is_some(), "a morte tem que ser detectada");
        assert!(
            achar(&eventos, "ROUND_DAMAGE").is_none(),
            "o CS2 não fornece dano — nenhum evento de dano pode ser inventado"
        );
        let sync = condensed(&morreu);
        assert!(
            sync["player"]["round_totaldmg"].is_null(),
            "campo de dano não existe no GSI do CS2 e não pode entrar no STATE_SYNC"
        );
    }

    /// Trocar de arma acontece o tempo todo e ninguém lê isso — só a C4
    /// importa, porque é o que atribui o plant.
    #[test]
    fn so_a_c4_gera_weapon_change() {
        let com_arma = |nome: &str| {
            json!({
                "provider": { "steamid": "76561198000000001" },
                "map": { "phase": "live", "round": 3 },
                "round": { "phase": "live" },
                "player": {
                    "steamid": "76561198000000001",
                    "state": { "health": 100 },
                    "match_stats": { "kills": 0, "assists": 0, "deaths": 0, "mvps": 0 },
                    "weapons": { "weapon_0": { "name": nome, "state": "active" } }
                }
            })
        };

        let rifle = extract(Some(&com_arma("weapon_knife")), &com_arma("weapon_ak47"));
        assert!(
            achar(&rifle, "WEAPON_CHANGE").is_none(),
            "troca para rifle não deve sair da máquina do jogador"
        );

        let c4 = extract(Some(&com_arma("weapon_ak47")), &com_arma("weapon_c4"));
        let ev = achar(&c4, "WEAPON_CHANGE").expect("a C4 precisa sair: é ela que atribui o plant");
        assert_eq!(ev.data["weapon"], "weapon_c4");
    }

    #[test]
    fn condensed_so_leva_stats_do_proprio_jogador() {
        let proprio = condensed(&payload_fim_de_round(false));
        assert_eq!(proprio["player"]["kills"], 9);

        let observando = condensed(&payload_fim_de_round(true));
        assert!(observando["player"].is_null());
    }
}
