//! Offline `.tscn` → [`Level`] importer (CAMPAIGN_PLAN.md §4, §4a).
//!
//! **Dev-only tool.** Reads a Godot 4 text scene, decodes its `TileMapLayer`
//! tile data and instanced child nodes, and produces our engine-native [`Level`].
//! The runtime never touches `.tscn`; this just bootstraps geometry so we can
//! eyeball imported levels locally. Per the IP decision (CAMPAIGN_PLAN.md), the
//! imported Nintendo-derived layouts are kept out of the repo — we ship our own.
//!
//! Verified format (CAMPAIGN_PLAN.md "Decisions"):
//!   - `.tscn` is INI-like: `[section attrs...]` headers + `key = value` props.
//!   - `tile_map_data = PackedByteArray("base64")`; decoded = `u16` version then
//!     12-byte cells `x:i16 y:i16 source_id:u16 atlas_x:u16 atlas_y:u16 alt:u16`
//!     (little-endian).
//!
//! Translation tables (atlas→kind, scene→entity) are ours and intentionally
//! conservative: unmapped tiles fall back to solid `ground`, unmapped scenes are
//! kept as `unknown:<Name>` and reported — nothing is silently dropped.

use super::ir::{Entity, Goal, Level, TileKind, TileSpan};
use std::collections::HashMap;
use std::io;

const TILE_PX: f64 = 16.0;

/// Result of an import: the level plus any warnings worth surfacing (unmapped
/// scenes, missing spawn, etc.).
pub struct Imported {
    pub level: Level,
    pub warnings: Vec<String>,
}

/// Parse a `.tscn` text scene into a [`Level`]. `id`/`theme` are ours to supply
/// (the scene doesn't carry our vocabulary).
pub fn import_tscn(text: &str, id: &str, theme: &str) -> io::Result<Imported> {
    let mut ext: HashMap<String, String> = HashMap::new(); // ext_resource id -> scene basename
    let mut nodes: Vec<NodeAcc> = Vec::new();
    let mut cur: Option<NodeAcc> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            if let Some(n) = cur.take() {
                nodes.push(n);
            }
            let inner = &line[1..line.rfind(']').unwrap_or(line.len() - 1)];
            let toks = split_ws_quoted(inner);
            let kind = toks.first().map(|s| s.as_str()).unwrap_or("");
            let attrs = parse_attrs(&toks[toks.len().min(1)..]);
            match kind {
                "ext_resource" => {
                    if let (Some(id), Some(path)) = (attrs.get("id"), attrs.get("path")) {
                        ext.insert(strip_quotes(id), scene_basename(&strip_quotes(path)));
                    }
                }
                "node" => {
                    let mut n = NodeAcc::default();
                    n.name = attrs.get("name").map(|v| strip_quotes(v)).unwrap_or_default();
                    if let Some(inst) = attrs.get("instance") {
                        if let Some(rid) = extresource_id(inst) {
                            n.instance_scene = ext.get(&rid).cloned();
                        }
                    }
                    cur = Some(n);
                }
                _ => {} // gd_scene / sub_resource / etc. — ignored (a node was just finalized)
            }
        } else if let Some(n) = cur.as_mut() {
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "position" => n.position = parse_vector2(v.trim()),
                    "tile_map_data" => n.tile_map_data = parse_packed_byte_array(v.trim()),
                    _ => {}
                }
            }
        }
    }
    if let Some(n) = cur.take() {
        nodes.push(n);
    }

    // ---- classify nodes into raw (pixel-space) placements ----
    let mut cells: Vec<(i32, i32, TileKind)> = Vec::new(); // tile-space already
    let mut entities: Vec<Entity> = Vec::new();
    let mut spawn: Option<(i32, i32)> = None;
    let mut goal: Option<Goal> = None;
    let mut checkpoints: Vec<(i32, i32)> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let px_to_tile = |p: (f64, f64)| -> (i32, i32) { ((p.0 / TILE_PX).round() as i32, (p.1 / TILE_PX).round() as i32) };

    for n in nodes {
        if let Some(data) = &n.tile_map_data {
            let is_deco = n.name.to_ascii_lowercase().contains("deco");
            for c in decode_cells(data) {
                let kind = if is_deco { TileKind::Deco } else { atlas_kind(c.source_id, c.atlas_x, c.atlas_y) };
                cells.push((c.x, c.y, kind));
            }
            continue;
        }
        let pos = n.position;
        let name = n.name.as_str();
        if name == "Player" {
            if let Some(p) = pos {
                spawn = Some(px_to_tile(p));
            }
            continue;
        }
        if name.starts_with("End") {
            // EndFlagpole / EndSmallCastle / End... — the level exit.
            if let Some(p) = pos {
                let (tx, ty) = px_to_tile(p);
                goal = Some(Goal { kind: "flag".into(), x: tx, y: ty });
            }
            continue;
        }
        if name.starts_with("Checkpoint") {
            if let Some(p) = pos {
                checkpoints.push(px_to_tile(p));
            }
            continue;
        }
        if let Some(scene) = n.instance_scene {
            let (tx, ty) = px_to_tile(pos.unwrap_or((0.0, 0.0)));
            match classify_scene(&scene) {
                SceneClass::Drop => {}
                SceneClass::Warp => entities.push(Entity { kind: "warp".into(), x: tx, y: ty, props: vec![] }),
                SceneClass::Exit => {
                    // First exit becomes the goal; any extra is kept as an entity.
                    if goal.is_none() {
                        goal = Some(Goal { kind: "exit".into(), x: tx, y: ty });
                    } else {
                        entities.push(Entity { kind: "exit".into(), x: tx, y: ty, props: vec![] });
                    }
                }
                SceneClass::Entity(t, props) => {
                    let props = props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
                    entities.push(Entity { kind: t.into(), x: tx, y: ty, props });
                }
                SceneClass::Unknown => {
                    warnings.push(format!("unmapped scene {scene:?}"));
                    entities.push(Entity { kind: format!("unknown:{scene}"), x: tx, y: ty, props: vec![] });
                }
            }
        }
    }

    // ---- normalize: shift so the top-left of everything is (0,0) ----
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    let mut acc = |x: i32, y: i32| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    };
    for &(x, y, _) in &cells {
        acc(x, y);
    }
    for e in &entities {
        acc(e.x, e.y);
    }
    if let Some(s) = spawn {
        acc(s.0, s.1);
    }
    if let Some(g) = &goal {
        acc(g.x, g.y);
    }
    for c in &checkpoints {
        acc(c.0, c.1);
    }
    if min_x == i32::MAX {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no tiles or entities found in scene"));
    }
    let (ox, oy) = (min_x, min_y);
    let w = max_x - min_x + 1;
    let h = max_y - min_y + 1;

    // ---- build the Level (offset everything, run-length the tiles) ----
    let mut lvl = Level::new(id, theme, w, h);
    // Terrain wins over deco when both occupy a cell.
    let mut grid: HashMap<(i32, i32), TileKind> = HashMap::new();
    for &(x, y, k) in &cells {
        let key = (x - ox, y - oy);
        match grid.get(&key) {
            Some(existing) if *existing != TileKind::Deco => {} // keep terrain
            _ => {
                grid.insert(key, k);
            }
        }
    }
    let mut flat: Vec<(i32, i32, TileKind)> = grid.into_iter().map(|((x, y), k)| (x, y, k)).collect();
    flat.sort_by_key(|&(x, y, _)| (y, x));
    lvl.tiles = runs_from_sorted(&flat);

    for e in &entities {
        lvl.entities.push(Entity { kind: e.kind.clone(), x: e.x - ox, y: e.y - oy, props: e.props.clone() });
    }
    lvl.spawn = spawn.map(|(x, y)| (x - ox, y - oy)).unwrap_or_else(|| {
        warnings.push("no Player node found; spawn defaulted to (2, top)".into());
        (2, 2)
    });
    lvl.goal = goal.map(|g| Goal { kind: g.kind, x: g.x - ox, y: g.y - oy });
    lvl.checkpoints = checkpoints.iter().map(|(x, y)| (x - ox, y - oy)).collect();

    Ok(Imported { level: lvl, warnings })
}

// ---- translation tables (ours) ----------------------------------------------

/// Godot TileSet atlas cell → our [`TileKind`]. Stub: every solid tile is
/// `ground` for now so collision geometry is correct; refine per-TileSet later
/// (CAMPAIGN_PLAN.md §4a). Kept as a fn so it's the obvious extension point.
fn atlas_kind(_source_id: u16, _atlas_x: u16, _atlas_y: u16) -> TileKind {
    TileKind::Ground
}

/// How an instanced scene maps into our IR.
enum SceneClass {
    /// Emit as an entity of this type with these default props.
    Entity(&'static str, &'static [(&'static str, &'static str)]),
    /// A pipe/warp trigger → entity `warp`.
    Warp,
    /// A level-exit → the goal (or a spare `exit` entity).
    Exit,
    /// Engine plumbing / pure visual / sub-area link → skip entirely.
    Drop,
    /// Unrecognized → kept as `unknown:<Name>` and reported.
    Unknown,
}

/// Instanced scene basename → its role in our IR. Creature/item/block names are
/// the original Munchii bestiary (CAMPAIGN_PLAN.md §7–§8) — the Mario names here
/// are only input identifiers we translate away from. Behaviors/art come later.
fn classify_scene(scene: &str) -> SceneClass {
    use SceneClass::*;
    // Block props: `breakable` (Munchii can smash it from below/while big) and a
    // default `contains` so the runtime has something to release.
    const C_KIBBLE: &[(&str, &str)] = &[("contains", "kibble")];
    const C_POWER: &[(&str, &str)] = &[("contains", "big_kibble")];
    const C_POWER_HID: &[(&str, &str)] = &[("contains", "big_kibble"), ("hidden", "1")];
    const C_1UP_HID: &[(&str, &str)] = &[("contains", "lucky_squeaky"), ("hidden", "1")];
    const C_POISON: &[(&str, &str)] = &[("contains", "poison")];
    const BRICK: &[(&str, &str)] = &[("breakable", "1")];
    const BRICK_POW: &[(&str, &str)] = &[("breakable", "1"), ("contains", "big_kibble")];
    const BRICK_COIN: &[(&str, &str)] = &[("breakable", "1"), ("contains", "kibble")];
    const NONE: &[(&str, &str)] = &[];

    match scene {
        // ---- creatures ----
        "Goomba" => Entity("boneling", NONE),
        "GreenKoopaTroopa" => Entity("rollo", NONE),
        "RedKoopaTroopa" => Entity("rollo_sun", NONE),
        "GreenParatroopa" | "RedParatroopa" | "GreenKoopaParaTroopa" | "RedKoopaParaTroopa"
        | "GreenParaKoopaHori" => Entity("flutterbug", NONE),
        "FighterFly" => Entity("hoppa", NONE),
        "SideStepper" => Entity("pincher", NONE),
        "PiranhaPlant" => Entity("dandi", NONE),
        "RedPiranhaPlant" => Entity("dandi_sun", NONE),
        "BuzzyBeetle" => Entity("hardhat", NONE),
        "HammerBro" | "BowsersBro" => Entity("stick_squirrel", NONE),
        "Sigebou" => Entity("sticker", NONE),
        "BulletBill" => Entity("zoomdisc", NONE),
        "BulletBillCannon" | "BulletBillLauncher" => Entity("zoomdisc_launcher", NONE),
        "CheepCheep" | "GreenCheepCheep" => Entity("sudsfish", NONE),
        "RedCheepCheep" => Entity("sudsfish_sun", NONE),
        "Blooper" | "Bloober" => Entity("moppet", NONE),
        "Lakitu" => Entity("puffer", NONE),
        "DryBones" => Entity("rattle", NONE),
        "Podoboo" => Entity("pop", NONE),
        "Firebar" => Entity("sprinkler_bar", NONE),
        "Icicle" => Entity("drip", NONE),
        "Barrel" => Entity("log", NONE),
        "Burner" => Entity("blowdryer", NONE),
        "OnOffFanRed" => Entity("fan", &[("color", "red")]),
        "OnOffFanBlue" => Entity("fan", &[("color", "blue")]),
        "Bowser" | "TrueBowser" => Entity("baron_whiskers", NONE),
        // generators (spawn streams of a critter)
        "CheepCheepGenerator" | "CheepCheepSideGenerator" | "LeapingCheepCheepArea" => {
            Entity("spawner", &[("of", "sudsfish")])
        }
        "BulletBillGenerator" => Entity("spawner", &[("of", "zoomdisc")]),
        "BowserFlameGenerator" => Entity("spawner", &[("of", "ember")]),

        // ---- items / power-ups ----
        "Mushroom" | "SuperMushroom" => Entity("big_kibble", NONE),
        "FireFlower" => Entity("bubble_bone", NONE),
        "Starman" | "Star" => Entity("zoomies_treat", NONE),
        "OneUpMushroom" | "1UpMushroom" => Entity("lucky_squeaky", NONE),
        "WingItem" => Entity("flutter_collar", NONE),
        "Coin" | "RedCoin" | "SpinningRedCoin" => Entity("kibble", NONE),

        // ---- blocks (breakable bricks + treat-blocks) ----
        "QuestionBlock" => Entity("question", C_KIBBLE),
        "PowerUpQuestionBlock" => Entity("question", C_POWER),
        "InvisibleQuestionBlock" | "InvisiblePowerUpQuestionBlock" => Entity("question", C_POWER_HID),
        "InvisibleOneUpQuestionBlock" => Entity("question", C_1UP_HID),
        "PoisonQuestionBlock" => Entity("question", C_POISON),
        "BrickBlock" => Entity("brick", BRICK),
        "CoinBrickBlock" => Entity("brick", BRICK_COIN),
        "PowerUpBrickBlock" => Entity("brick", BRICK_POW),
        "PSwitch" | "PSwitchBlock" => Entity("pswitch", NONE),
        "Vine" => Entity("ivy", NONE),

        // ---- moving platforms ----
        "FallingPlatform" | "LargeFallingPlatform" => Entity("platform", &[("move", "falling")]),
        "Trampoline" => Entity("trampoline", NONE),
        "SuperTrampoline" => Entity("trampoline", &[("power", "super")]),
        "ElevatorPlatform" | "SmallElevatorPlatform" | "MediumElevatorPlatform"
        | "RopeElevatorPlatform" | "SmallRopeElevatorPlatform" => Entity("platform", &[("move", "lift")]),
        "SidewaysPlatform" | "SmallSidewaysPlatform" | "OnOffSidewaysPlatform" | "OnOffSidewaysPlatformBlue" => {
            Entity("platform", &[("move", "sideways")])
        }
        "VerticalPlatform" | "SmallVerticalPlatform" | "OnOffVerticalPlatform" | "OnOffVerticalPlatformBlue" => {
            Entity("platform", &[("move", "vertical")])
        }
        "TravellingPlatform" => Entity("platform", &[("move", "travel")]),
        "CloudPlatform" => Entity("platform", &[("move", "cloud")]),

        // ---- boss-room / castle features ----
        "CastleBridge" => Entity("bath_plug", NONE), // the "axe": pull it to win
        "CastleToad" | "CastlePeach" | "CastlePeachSP" => Entity("rescued_pup", NONE),

        // ---- warps & exits ----
        "PipeArea" | "TeleportPipeArea" | "AutoExitPipeArea" | "WarpZone" => Warp,
        "UndergroundExit" | "UnderwaterExit" | "LostLevelsEndingDoor" => Exit,

        // ---- engine plumbing / pure visual / logic → drop ----
        "LevelBG" | "DropShadowRenderer" | "EntityGeneratorStopper" | "BooRaceHandler" | "RaceBoo"
        | "ChallengeModeNodes" | "CastleChallengeEnd" | "PickAPathTeleport" | "PickAPathPoint"
        | "LargeSPCastleDeco" | "Deco1" | "Deco2" | "SmallCastleVisual" | "CoinHeavenAllCoinsBonus"
        | "PipeCutscene" | "LostLevelsEnding" | "StartCastle" | "LargeStartCastle"
        | "BowserFlame" | "BulletBillGeneratorStopper" | "WindGenerator" => Drop,

        // camera/zone markers (we clamp to level bounds; zones handled later)
        s if s.ends_with("Limit") => Drop, // HardCameraRightLimit / WarpZoneCameraLimit
        s if s.ends_with("Area") => Drop, // WaterArea/WindArea/FireWindArea/UpsideDownGravityArea/…

        // sub-area level links are instanced scenes named like "1-1a" / "8-4" / "2".
        s if s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) => Drop,

        _ => Unknown,
    }
}

// ---- node accumulation ------------------------------------------------------

#[derive(Default)]
struct NodeAcc {
    name: String,
    instance_scene: Option<String>,
    position: Option<(f64, f64)>,
    tile_map_data: Option<Vec<u8>>,
}

struct Cell {
    x: i32,
    y: i32,
    source_id: u16,
    atlas_x: u16,
    atlas_y: u16,
}

/// Decode a `tile_map_data` byte blob into cells. Tolerates the leading `u16`
/// version header (the common case) or its absence, by aligning to a 12-byte grid.
fn decode_cells(bytes: &[u8]) -> Vec<Cell> {
    let mut start = 0usize;
    if bytes.len() >= 2 && (bytes.len() - 2) % 12 == 0 {
        start = 2; // skip version header
    }
    let mut cells = Vec::new();
    let mut i = start;
    while i + 12 <= bytes.len() {
        let rd16 = |o: usize| -> [u8; 2] { [bytes[i + o], bytes[i + o + 1]] };
        cells.push(Cell {
            x: i16::from_le_bytes(rd16(0)) as i32,
            y: i16::from_le_bytes(rd16(2)) as i32,
            source_id: u16::from_le_bytes(rd16(4)),
            atlas_x: u16::from_le_bytes(rd16(6)),
            atlas_y: u16::from_le_bytes(rd16(8)),
            // bytes 10..12 = alternative/transform flags — unused for now
        });
        i += 12;
    }
    cells
}

/// Merge a (y,x)-sorted cell list into horizontal same-kind runs.
fn runs_from_sorted(sorted: &[(i32, i32, TileKind)]) -> Vec<TileSpan> {
    let mut out = Vec::new();
    let mut iter = sorted.iter().copied();
    if let Some((mut sx, mut sy, mut sk)) = iter.next() {
        let mut len = 1;
        let mut px = sx;
        for (x, y, k) in iter {
            if y == sy && k == sk && x == px + 1 {
                len += 1;
                px = x;
            } else {
                out.push(TileSpan { x: sx, y: sy, len, kind: sk });
                sx = x;
                sy = y;
                sk = k;
                len = 1;
                px = x;
            }
        }
        out.push(TileSpan { x: sx, y: sy, len, kind: sk });
    }
    out
}

// ---- tiny .tscn text helpers ------------------------------------------------

/// Split on whitespace, keeping double-quoted spans intact (quotes retained).
fn split_ws_quoted(s: &str) -> Vec<String> {
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut inq = false;
    for c in s.chars() {
        match c {
            '"' => {
                inq = !inq;
                cur.push(c);
            }
            c if c.is_whitespace() && !inq => {
                if !cur.is_empty() {
                    toks.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    toks
}

/// `key=value` tokens → map (values kept raw, quotes intact).
fn parse_attrs(toks: &[String]) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for t in toks {
        if let Some((k, v)) = t.split_once('=') {
            m.insert(k.to_string(), v.to_string());
        }
    }
    m
}

fn strip_quotes(s: &str) -> String {
    s.trim().trim_matches('"').to_string()
}

/// `ExtResource("id")` → `id`.
fn extresource_id(v: &str) -> Option<String> {
    let inner = v.trim().strip_prefix("ExtResource(")?.strip_suffix(')')?;
    Some(strip_quotes(inner))
}

/// `res://a/b/Goomba.tscn` → `Goomba`.
fn scene_basename(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.strip_suffix(".tscn").unwrap_or(file).to_string()
}

/// `Vector2(32, 176)` / `Vector2i(32, 176)` → (32.0, 176.0).
fn parse_vector2(v: &str) -> Option<(f64, f64)> {
    let inner = v.split_once('(')?.1.split_once(')')?.0;
    let mut parts = inner.split(',');
    let x: f64 = parts.next()?.trim().parse().ok()?;
    let y: f64 = parts.next()?.trim().parse().ok()?;
    Some((x, y))
}

/// `PackedByteArray("base64")` → decoded bytes.
fn parse_packed_byte_array(v: &str) -> Option<Vec<u8>> {
    let inner = v.trim().strip_prefix("PackedByteArray(")?.strip_suffix(')')?;
    b64_decode(&strip_quotes(inner))
}

/// Standard base64 decode (RFC 4648), tolerant of embedded whitespace.
fn b64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    }
    let clean: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.chunks(4) {
        let syms: Vec<u32> = chunk.iter().take_while(|&&c| c != b'=').map(|&c| val(c)).collect::<Option<_>>()?;
        let n = syms.len();
        if n == 0 {
            break;
        }
        if n == 1 {
            return None; // a lone symbol can't encode a byte
        }
        let mut accv = 0u32;
        for (j, &s) in syms.iter().enumerate() {
            accv |= s << (18 - 6 * j);
        }
        out.push((accv >> 16) as u8);
        if n >= 3 {
            out.push((accv >> 8) as u8);
        }
        if n >= 4 {
            out.push(accv as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test-only base64 encoder, so we can synthesize tile_map_data.
    fn b64_encode(data: &[u8]) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
            out.push(A[(n >> 18 & 63) as usize] as char);
            out.push(A[(n >> 12 & 63) as usize] as char);
            out.push(if chunk.len() > 1 { A[(n >> 6 & 63) as usize] as char } else { '=' });
            out.push(if chunk.len() > 2 { A[(n & 63) as usize] as char } else { '=' });
        }
        out
    }

    fn cell_bytes(cells: &[(i16, i16, u16, u16, u16, u16)]) -> Vec<u8> {
        let mut v = vec![0u8, 0u8]; // version header
        for &(x, y, s, ax, ay, alt) in cells {
            v.extend_from_slice(&x.to_le_bytes());
            v.extend_from_slice(&y.to_le_bytes());
            v.extend_from_slice(&s.to_le_bytes());
            v.extend_from_slice(&ax.to_le_bytes());
            v.extend_from_slice(&ay.to_le_bytes());
            v.extend_from_slice(&alt.to_le_bytes());
        }
        v
    }

    #[test]
    fn base64_roundtrip() {
        for v in [vec![], vec![0u8], vec![1, 2], vec![1, 2, 3], vec![255, 0, 128, 64, 32]] {
            assert_eq!(b64_decode(&b64_encode(&v)), Some(v));
        }
    }

    #[test]
    fn imports_a_synthetic_scene() {
        // ground row at cell y=12, x=0..3; a Goomba, a Player, and a flagpole.
        let data = cell_bytes(&[
            (0, 12, 0, 0, 0, 0),
            (1, 12, 0, 0, 0, 0),
            (2, 12, 0, 0, 0, 0),
            (3, 12, 0, 0, 0, 0),
        ]);
        let b64 = b64_encode(&data);
        let tscn = format!(
            "[gd_scene load_steps=2 format=4]\n\
             [ext_resource type=\"PackedScene\" path=\"res://Actors/Goomba.tscn\" id=\"1_g\"]\n\
             [node name=\"Level\" type=\"Node2D\"]\n\
             [node name=\"Tiles\" type=\"TileMapLayer\" parent=\".\"]\n\
             tile_map_data = PackedByteArray(\"{b64}\")\n\
             [node name=\"Player\" type=\"Node2D\" parent=\".\"]\n\
             position = Vector2(32, 176)\n\
             [node name=\"Goomba\" parent=\".\" instance=ExtResource(\"1_g\")]\n\
             position = Vector2(80, 176)\n\
             [node name=\"EndFlagpole\" type=\"Node2D\" parent=\".\"]\n\
             position = Vector2(240, 48)\n"
        );

        let imp = import_tscn(&tscn, "test-1", "overworld").unwrap();
        let l = &imp.level;

        // Geometry: 4 ground cells became one run.
        assert_eq!(l.tiles.len(), 1);
        assert_eq!(l.tiles[0].len, 4);
        assert_eq!(l.tiles[0].kind, TileKind::Ground);

        // Flag is the topmost thing (cell y=3), so it normalizes to row 0 and the
        // ground (cell y=12) to row 9.
        let g = l.goal.as_ref().unwrap();
        assert_eq!(g.y, 0);
        assert_eq!(l.tiles[0].y, 9);

        // Entity mapped Goomba -> boneling at tile (5, 11) - offset(0,3) = (5, 8).
        assert_eq!(l.entities.len(), 1);
        assert_eq!(l.entities[0].kind, "boneling");
        assert_eq!((l.entities[0].x, l.entities[0].y), (5, 8));

        // Player -> spawn at (2,11)-offset = (2,8).
        assert_eq!(l.spawn, (2, 8));
        assert!(imp.warnings.is_empty(), "no unmapped scenes expected: {:?}", imp.warnings);

        // And it round-trips through the IR text form.
        assert_eq!(Level::from_text(&l.to_text()).unwrap(), *l);
    }

    #[test]
    fn unmapped_scene_is_kept_and_warned() {
        let tscn = "[gd_scene format=4]\n\
             [ext_resource type=\"PackedScene\" path=\"res://Actors/Mystery.tscn\" id=\"1_m\"]\n\
             [node name=\"Tiles\" type=\"TileMapLayer\" parent=\".\"]\n\
             tile_map_data = PackedByteArray(\"AAA=\")\n\
             [node name=\"Mystery\" parent=\".\" instance=ExtResource(\"1_m\")]\n\
             position = Vector2(16, 16)\n";
        let imp = import_tscn(tscn, "x", "overworld").unwrap();
        assert!(imp.level.entities.iter().any(|e| e.kind == "unknown:Mystery"));
        assert!(!imp.warnings.is_empty());
    }
}
