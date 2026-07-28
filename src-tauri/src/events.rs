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

            // Troca de arma ativa.
            let weapon = active_weapon(curr);
            if weapon.is_some() && weapon != prev_own.and_then(active_weapon) {
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
    let owner = s(v, "/provider/steamid");
    let is_own = owner.is_some() && owner == s(v, "/player/steamid");
    let player = if is_own {
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
