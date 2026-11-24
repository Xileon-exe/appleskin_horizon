//! AppleSkin for Horizon: a port of AppleSkin (Fabric, Unlicense), following its code.
//!
//! - the saturation, as AppleSkin's golden outlines over the food bar;
//! - the exhaustion, as its dithered strip under the food bar;
//! - what the food in the hand (the off hand when the main one holds nothing edible) would give:
//!   hunger, saturation, and the estimated health, pulsing like AppleSkin's flash;
//! - the same values in a food's tooltip, always or with SHIFT held;
//! - the three values on the F3 screen;
//! - the server's exhaustion, saturation and rules, sent to each client (vanilla keeps them to
//!   itself), as AppleSkin does.
mod icons_data;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use mod_api::client::hud_layout::HeartKind;
use mod_api::text::{TextContent, TextObject, TextStyle};
use mod_api::{
    ClientHost, ClientTickPostEvent, DebugEntry, DebugEntryLinesEvent, DebugGroup, DrawContext,
    ComponentId, Flow, GameRuleValue, GuiDraw, GuiLayerPostEvent, GuiLayerPreEvent, Hand,
    HealthHudEvent, Hooks, Identifier, ItemResource, ItemStack, ItemTooltipEvent, PayloadCodec, PayloadId, PlayerLoggedInEvent,
    RegisterDebugEntriesEvent, RegisterRuntimeSpritesEvent, RegistrationContext, Registrar,
    RuntimeSprite, ServerHost, ServerTickEvent, SettingSpec, Tag, Text, TextureAtlas, ToClient,
    VarInt,
};

// ---------------------------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------------------------

/// The HUD layer that holds the health, armor, food and air rows.
const PLAYER_HEALTH_LAYER: &str = "minecraft:player_health";

/// Vanilla's rows: icons 9 px wide, the food bar's right edge 91 px right of the screen's middle
/// and the health bar's left edge 91 px left of it, both rows' top 39 px above the bottom.
const ICON: f32 = 9.0;
const HALF_WIDTH: f32 = 91.0;
const ABOVE_BOTTOM: f32 = 39.0;

const MAX_FOOD: i32 = 20;
/// `FoodHelper.MAX_EXHAUSTION` and `REGEN_EXHAUSTION_INCREMENT`.
const MAX_EXHAUSTION: f32 = 4.0;
const REGEN_EXHAUSTION_INCREMENT: f32 = 6.0;

/// AppleSkin's sheet as a GUI sprite of the mod (`textures/icons.png`, 256x256).
const SHEET: &str = "appleskin:gui/icons";

/// Vanilla's HUD sprites, named by path alone.
const FOOD_FULL: &str = "gui/sprites/hud/food_full";
const FOOD_HALF: &str = "gui/sprites/hud/food_half";
const FOOD_EMPTY: &str = "gui/sprites/hud/food_empty";
const FOOD_FULL_HUNGER: &str = "gui/sprites/hud/food_full_hunger";
const FOOD_HALF_HUNGER: &str = "gui/sprites/hud/food_half_hunger";
const FOOD_EMPTY_HUNGER: &str = "gui/sprites/hud/food_empty_hunger";
const HEART_CONTAINER: &str = "gui/sprites/hud/heart/container";
const HEART_FULL: &str = "gui/sprites/hud/heart/full";
const HEART_HALF: &str = "gui/sprites/hud/heart/half";
const HEART_HARDCORE_CONTAINER: &str = "gui/sprites/hud/heart/container_hardcore";
const HEART_HARDCORE_FULL: &str = "gui/sprites/hud/heart/hardcore_full";
const HEART_HARDCORE_HALF: &str = "gui/sprites/hud/heart/hardcore_half";

/// The same food icons as a tooltip's inline sprites name them (the GUI atlas, vanilla's id).
const TIP_FOOD_FULL: &str = "hud/food_full";
const TIP_FOOD_HALF: &str = "hud/food_half";
const TIP_FOOD_FULL_HUNGER: &str = "hud/food_full_hunger";
const TIP_FOOD_HALF_HUNGER: &str = "hud/food_half_hunger";

/// The settings' keys: AppleSkin's own options (`appleskin.json5`), in snake case.
const SHOW_FOOD_VALUES_IN_TOOLTIP: &str = "show_food_values_in_tooltip";
const SHOW_FOOD_VALUES_IN_TOOLTIP_ALWAYS: &str = "show_food_values_in_tooltip_always";
const SHOW_FOOD_VALUES_HUD_OVERLAY: &str = "show_food_values_hud_overlay";
const SHOW_FOOD_VALUES_HUD_OVERLAY_WHEN_OFFHAND: &str = "show_food_values_hud_overlay_when_offhand";
const SHOW_FOOD_HEALTH_HUD_OVERLAY: &str = "show_food_health_hud_overlay";
const SHOW_FOOD_EXHAUSTION_HUD_UNDERLAY: &str = "show_food_exhaustion_hud_underlay";
const SHOW_SATURATION_HUD_OVERLAY: &str = "show_saturation_hud_overlay";
const SHOW_FOOD_DEBUG_INFO: &str = "show_food_debug_info";
const SHOW_VANILLA_ANIMATIONS_OVERLAY: &str = "show_vanilla_animations_overlay";
const MAX_HUD_OVERLAY_FLASH_ALPHA: &str = "max_hud_overlay_flash_alpha";

/// The F3 entry's id: `<mod id>:<path>`.
const DEBUG_ID: &str = "appleskin:food_stats";

/// The channel that carries the server's food state to a client.
const SYNC_ID: &str = "appleskin:food_sync";

/// The settings the player picks (AppleSkin's defaults).
fn settings() -> Vec<SettingSpec> {
    let toggle = |key: &str, default: bool| {
        SettingSpec::toggle(key, format!("appleskin.setting.{key}"), default)
            .tooltip(format!("appleskin.setting.{key}.tooltip"))
    };
    vec![
        toggle(SHOW_FOOD_VALUES_IN_TOOLTIP, true),
        toggle(SHOW_FOOD_VALUES_IN_TOOLTIP_ALWAYS, true),
        toggle(SHOW_SATURATION_HUD_OVERLAY, true),
        toggle(SHOW_FOOD_VALUES_HUD_OVERLAY, true),
        toggle(SHOW_FOOD_VALUES_HUD_OVERLAY_WHEN_OFFHAND, true),
        toggle(SHOW_FOOD_EXHAUSTION_HUD_UNDERLAY, true),
        toggle(SHOW_FOOD_HEALTH_HUD_OVERLAY, true),
        toggle(SHOW_VANILLA_ANIMATIONS_OVERLAY, true),
        toggle(SHOW_FOOD_DEBUG_INFO, true),
        SettingSpec::range(
            MAX_HUD_OVERLAY_FLASH_ALPHA,
            format!("appleskin.setting.{MAX_HUD_OVERLAY_FLASH_ALPHA}"),
            0.0,
            1.0,
            0.05,
            0.65,
        )
        .tooltip(format!("appleskin.setting.{MAX_HUD_OVERLAY_FLASH_ALPHA}.tooltip")),
    ]
}

/// A survival player: the food bar is drawn for no other (`player.is_creative`, `is_spectator`).
fn is_survival(host: &dyn ClientHost) -> bool {
    host.local_player().is_some_and(|player| {
        !player.is_creative(host).unwrap_or(false) && !player.is_spectator(host).unwrap_or(false)
    })
}

/// A toggle's value, on when the game cannot say.
fn on(host: &dyn ClientHost, key: &str) -> bool {
    host.setting_bool(key).unwrap_or(true)
}

// ---------------------------------------------------------------------------------------------
// The server's food state, sent to the client
// ---------------------------------------------------------------------------------------------

/// What the server tells a client about its player: the exhaustion and the saturation (vanilla
/// sends neither as they move), and two rules the health estimate needs. `flags`: 1 = natural
/// health regeneration is on, 2 = the difficulty is peaceful.
#[derive(PayloadCodec)]
struct FoodSync {
    exhaustion: f32,
    saturation: f32,
    flags: VarInt,
}

static SYNC_TOKEN: OnceLock<ToClient<FoodSync>> = OnceLock::new();

/// Client: what the server last said, None until it does in this world.
#[derive(Clone, Copy)]
struct SyncState {
    exhaustion: f32,
    saturation: f32,
    natural_regeneration: bool,
    peaceful: bool,
}
static SYNC: Mutex<Option<SyncState>> = Mutex::new(None);

/// Server: what each player's client was last told, by uuid.
static LAST_SENT: Mutex<Option<HashMap<u128, (f32, f32, i32)>>> = Mutex::new(None);
/// AppleSkin sends the exhaustion again once it moves by this much; the saturation on any change.
const EXHAUSTION_STEP: f32 = 0.01;

fn sync_state() -> Option<SyncState> {
    SYNC.lock().ok().and_then(|s| *s)
}

/// On a server without the mod, what the client works out by itself: its count of the exhaustion
/// (the game's own, which only grows here) kept in a cycle and set back to 0 whenever the game
/// shows the server just passed a point, and the saturation moved down as the cycle wraps.
#[derive(Clone, Copy, Default)]
struct Track {
    valid: bool,
    last_count: f32,
    last_food: i32,
    last_saturation: f32,
    exhaustion: f32,
    saturation: f32,
}
static TRACK: Mutex<Track> = Mutex::new(Track {
    valid: false,
    last_count: 0.0,
    last_food: 0,
    last_saturation: 0.0,
    exhaustion: 0.0,
    saturation: 0.0,
});

fn track_state() -> Option<Track> {
    TRACK.lock().ok().map(|t| *t).filter(|t| t.valid)
}

fn forget_track() {
    if let Ok(mut t) = TRACK.lock() {
        t.valid = false;
    }
}

/// One client tick of the estimate, with the game's own food values of the local player.
fn track_unsynced(level: i32, saturation: f32, count: f32) {
    let Ok(mut t) = TRACK.lock() else {
        return;
    };
    if !t.valid {
        *t = Track {
            valid: true,
            last_count: count,
            last_food: level,
            last_saturation: saturation,
            exhaustion: 0.0,
            saturation,
        };
        return;
    }

    // What the client counted since the last tick; a smaller count is a fresh one (a respawn).
    let delta = count - t.last_count;
    t.last_count = count;
    if delta > 0.0 {
        t.exhaustion += delta;
    }

    // The game sends the food level and the saturation when the level moves or the saturation
    // reaches 0, and each time the server passed a point with nothing to take from the saturation
    // that is what moves them: so at that tick the exhaustion is just past 0.
    let food_dropped = level < t.last_food;
    let saturation_ran_out = saturation <= 0.0 && t.last_saturation > 0.0 && level == t.last_food;
    if food_dropped || saturation_ran_out {
        t.exhaustion = 0.0;
    } else {
        while t.exhaustion >= MAX_EXHAUSTION {
            t.exhaustion -= MAX_EXHAUSTION;
            t.saturation = (t.saturation - 1.0).max(0.0);
        }
    }

    // A saturation the game sent is exact.
    if saturation != t.last_saturation || level != t.last_food {
        t.saturation = saturation;
    }
    t.last_food = level;
    t.last_saturation = saturation;
}

// ---------------------------------------------------------------------------------------------
// The HUD's state, read from the game each frame
// ---------------------------------------------------------------------------------------------

/// What `health_hud` says about the health rows: AppleSkin hooks the HUD's own draw calls and
/// reads the same values.
#[derive(Clone, Copy, Default)]
struct HudState {
    tick: i32,
    health: f32,
    max_health: f32,
    absorption: f32,
    hardcore: bool,
    regeneration: bool,
    /// The health bar is poisoned or withered.
    harmed: bool,
    vehicle_hearts: i32,
}
static HUD: Mutex<Option<HudState>> = Mutex::new(None);

fn hud_state() -> HudState {
    HUD.lock().ok().and_then(|s| *s).unwrap_or(HudState {
        health: MAX_FOOD as f32,
        max_health: MAX_FOOD as f32,
        ..HudState::default()
    })
}

/// AppleSkin's flash: a wave stepped each client tick, 0 to 1, scaled by the setting.
struct Flash {
    unclamped: f32,
    direction: f32,
    alpha: f32,
}
static FLASH: Mutex<Flash> = Mutex::new(Flash {
    unclamped: 0.0,
    direction: 1.0,
    alpha: 0.0,
});

fn tick_flash(max_alpha: f32) {
    if let Ok(mut f) = FLASH.lock() {
        f.unclamped += f.direction * 0.125;
        if f.unclamped >= 1.5 {
            f.direction = -1.0;
        } else if f.unclamped <= -0.5 {
            f.direction = 1.0;
        }
        f.alpha = f.unclamped.clamp(0.0, 1.0) * max_alpha.clamp(0.0, 1.0);
    }
}

fn reset_flash() {
    if let Ok(mut f) = FLASH.lock() {
        f.unclamped = 0.0;
        f.alpha = 0.0;
        f.direction = 1.0;
    }
}

fn flash_alpha() -> f32 {
    FLASH.lock().map(|f| f.alpha).unwrap_or(0.0)
}

/// The food the player could eat now, read once a tick.
static HELD: Mutex<Option<Food>> = Mutex::new(None);

fn held_food() -> Option<Food> {
    HELD.lock().ok().and_then(|f| *f)
}

/// Whether the client has a world: cleared on leaving it.
static IN_WORLD: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------------------------
// Food values
// ---------------------------------------------------------------------------------------------

/// What a food item gives when eaten (`FoodProperties` and the effects of its `Consumable`).
#[derive(Clone, Copy)]
struct Food {
    nutrition: i32,
    saturation: f32,
    /// Can be eaten with a full bar (a golden apple).
    always: bool,
    /// Has a harmful effect: AppleSkin draws it with the hunger icons (`FoodHelper.isRotten`).
    rotten: bool,
    /// Health its Regeneration effect gives (`getEstimatedHealthIncrement`'s second part).
    regeneration: f32,
}

/// A number tag as an `f64`, whatever width the item report gave it.
fn number(tag: &Tag) -> Option<f64> {
    Some(match tag {
        Tag::Byte(v) => f64::from(*v),
        Tag::Short(v) => f64::from(*v),
        Tag::Int(v) => f64::from(*v),
        Tag::Long(v) => *v as f64,
        Tag::Float(bits) => f64::from(f32::from_bits(*bits)),
        Tag::Double(bits) => f64::from_bits(*bits),
        _ => return None,
    })
}

fn string(tag: &Tag) -> Option<String> {
    match tag {
        Tag::String(s) => s.to_string().ok(),
        _ => None,
    }
}

fn elements(tag: &Tag) -> &[Tag] {
    match tag {
        Tag::List(items) => items,
        _ => &[],
    }
}

/// The effects `MobEffectCategory.HARMFUL` holds.
fn is_harmful(effect: &str) -> bool {
    matches!(
        effect.strip_prefix("minecraft:").unwrap_or(effect),
        "slowness"
            | "mining_fatigue"
            | "instant_damage"
            | "nausea"
            | "blindness"
            | "hunger"
            | "weakness"
            | "poison"
            | "wither"
            | "levitation"
            | "unluck"
            | "bad_luck"
            | "darkness"
            | "wind_charged"
            | "weaving"
            | "oozing"
            | "infested"
            | "raid_omen"
            | "trial_omen"
    )
}

/// The food values of `stack`, None when it is not food. The components are the stack's own
/// (`ItemResource::value`: its override, else the item's default, none where it was removed), so
/// food a data pack or another mod changed reads as the player sees it.
fn food_of(host: &dyn ClientHost, stack: &ItemStack) -> Option<Food> {
    let resource = ItemResource::of(stack);
    let component = |id: &str| -> Option<Tag> {
        ComponentId::new(id).and_then(|id| resource.value(host, &id))
    };
    let food = component("minecraft:food")?;
    let nutrition = number(food.get("nutrition")?)? as i32;
    let saturation = food.get("saturation").and_then(number).unwrap_or(0.0) as f32;
    let always = food
        .get("can_always_eat")
        .and_then(number)
        .is_some_and(|v| v != 0.0);

    // What eating it applies: harmful effects (rotten), and Regeneration's health.
    let mut rotten = false;
    let mut regeneration = 0.0_f32;
    if let Some(consumable) = component("minecraft:consumable") {
        if let Some(on_consume) = consumable.get("on_consume_effects") {
            for consume_effect in elements(on_consume) {
                let Some(effects) = consume_effect.get("effects") else {
                    continue;
                };
                for effect in elements(effects) {
                    let Some(id) = effect.get("id").and_then(string) else {
                        continue;
                    };
                    rotten |= is_harmful(&id);
                    if id.strip_prefix("minecraft:").unwrap_or(&id) == "regeneration"
                        && regeneration == 0.0
                    {
                        let amplifier = effect.get("amplifier").and_then(number).unwrap_or(0.0) as u32;
                        let duration = effect.get("duration").and_then(number).unwrap_or(0.0);
                        let interval = (50_u32 >> amplifier.min(31)).max(1) as f64;
                        regeneration = (duration / interval).floor() as f32;
                    }
                }
            }
        }
    }
    Some(Food {
        nutrition,
        saturation,
        always,
        rotten,
        regeneration,
    })
}

/// The food in `hand`, None for an empty hand or one that holds no food.
fn hand_food(host: &dyn ClientHost, hand: Hand) -> Option<Food> {
    let player = host.local_player()?;
    let living = host.living(player.entity())?;
    let stack = living.hand_item(host, hand)?;
    food_of(host, &stack)
}

/// `FoodHelper.canConsume`: `Player.canEat`.
fn can_consume(level: i32, food: &Food) -> bool {
    food.always || level < MAX_FOOD
}

/// `HeldFoodCache.query`: the main hand's food if it can be eaten, else the off hand's when
/// that is allowed.
fn query_held(host: &dyn ClientHost, level: i32) -> Option<Food> {
    let main = hand_food(host, Hand::Main).filter(|f| can_consume(level, f));
    if main.is_some() || !on(host, SHOW_FOOD_VALUES_HUD_OVERLAY_WHEN_OFFHAND) {
        return main;
    }
    hand_food(host, Hand::Off).filter(|f| can_consume(level, f))
}

// ---------------------------------------------------------------------------------------------
// Health estimate (`FoodHelper.getEstimatedHealthIncrement`)
// ---------------------------------------------------------------------------------------------

/// The health the player would regain from a bar of `food`, `saturation` and `exhaustion`.
fn estimated_health(mut food: i32, mut saturation: f32, mut exhaustion: f32) -> f32 {
    if !exhaustion.is_finite() || !saturation.is_finite() {
        return 0.0;
    }
    let mut health = 0.0_f32;
    // The game's loop ends on its own; a bound keeps odd values from spinning.
    for _ in 0..10_000 {
        if food < 18 {
            break;
        }
        while exhaustion > MAX_EXHAUSTION {
            exhaustion -= MAX_EXHAUSTION;
            if saturation > 0.0 {
                saturation = (saturation - 1.0).max(0.0);
            } else {
                food -= 1;
            }
        }
        if food >= 20 && saturation > f32::MIN_POSITIVE {
            let limited = saturation.min(REGEN_EXHAUSTION_INCREMENT);
            let next_up = f32::from_bits(MAX_EXHAUSTION.to_bits() + 1);
            let until_above_max = next_up - exhaustion;
            let iterations = ((until_above_max / limited).ceil() as i32).max(1);
            health += limited / REGEN_EXHAUSTION_INCREMENT * iterations as f32;
            exhaustion += limited * iterations as f32;
        } else if food >= 18 {
            health += 1.0;
            exhaustion += REGEN_EXHAUSTION_INCREMENT;
        }
    }
    health
}

// ---------------------------------------------------------------------------------------------
// Icon positions (`OffsetsCache`): vanilla's shake, replayed
// ---------------------------------------------------------------------------------------------

/// Java's `java.util.Random`, which AppleSkin seeds with the HUD's tick as vanilla's HUD does.
struct JavaRandom(u64);

impl JavaRandom {
    const MASK: u64 = (1 << 48) - 1;

    fn new(seed: i64) -> Self {
        Self((seed as u64 ^ 0x5DEE_CE66D) & Self::MASK)
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.0 = self.0.wrapping_mul(0x5DEE_CE66D).wrapping_add(0xB) & Self::MASK;
        (self.0 >> (48 - bits)) as i32
    }

    fn next_int(&mut self, bound: i32) -> i32 {
        let m = bound - 1;
        let mut u = self.next(31);
        if bound & m == 0 {
            return ((bound as i64 * u as i64) >> 31) as i32;
        }
        loop {
            let r = u % bound;
            if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                return r;
            }
            u = self.next(31);
        }
    }
}

/// Where each heart and each food icon sits against its bar's origin, with the shake.
struct Offsets {
    health: Vec<(i32, i32)>,
    food: Vec<(i32, i32)>,
}

fn offsets(hud: &HudState, level: i32, saturation: f32, animate: bool) -> Offsets {
    let absorption = hud.absorption.ceil();
    let bars = ((hud.max_health + absorption) / 2.0).ceil() as i32;
    let bars = if (0..=1000).contains(&bars) { bars } else { 0 };
    let rows = (bars as f32 / 10.0).ceil() as i32;
    let row_height = (10 - (rows - 2)).max(3);

    let (mut shake_health, mut shake_food) = (false, false);
    if animate {
        shake_food = saturation <= 0.0 && hud.tick.rem_euclid(level * 3 + 1) == 0;
        shake_health = hud.health.ceil() <= 4.0;
    }
    let mut random = JavaRandom::new(i64::from(hud.tick.wrapping_mul(312_871)));

    let mut health = vec![(0, 0); bars as usize];
    for i in (0..bars).rev() {
        let row = ((i + 1) as f32 / 10.0).ceil() as i32 - 1;
        let mut y = -(row * row_height);
        if shake_health {
            y += random.next_int(2);
        }
        health[i as usize] = (i % 10 * 8, y);
    }
    let mut food = Vec::with_capacity(10);
    for i in 0..10 {
        let mut y = 0;
        if shake_food {
            y += random.next_int(3) - 1;
        }
        food.push((-(i * 8) - 9, y));
    }
    Offsets { health, food }
}

// ---------------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------------

/// White with the alpha `alpha` (0 to 1), as a tint.
fn argb(alpha: f32) -> u32 {
    (((alpha.clamp(0.0, 1.0) * 255.0).round() as u32) << 24) | 0x00FF_FFFF
}

/// A 9x9 vanilla sprite at `(x, y)`.
fn vanilla_icon(draw: &mut dyn DrawContext, sprite: &str, x: f32, y: f32, alpha: f32) {
    let _ = draw.sprite(sprite, [0.0, 0.0, ICON, ICON], [x, y, ICON, ICON], Some(argb(alpha)));
}

/// The food bar's right edge and top, on a screen `draw` of the GUI's size.
fn food_bar(draw: &mut dyn DrawContext) -> (f32, f32) {
    let [width, height] = draw.size();
    ((width / 2.0).floor() + HALF_WIDTH, height - ABOVE_BOTTOM)
}

/// The health bar's left edge and top.
fn health_bar(draw: &mut dyn DrawContext) -> (f32, f32) {
    let [width, height] = draw.size();
    ((width / 2.0).floor() - HALF_WIDTH, height - ABOVE_BOTTOM)
}

/// The exhaustion as the bar shows it: the server's when the mod told us, else the client's own
/// estimate (see `Track`), else the game's count folded into a cycle.
fn exhaustion_shown(client_value: f32) -> f32 {
    if let Some(sync) = sync_state() {
        return sync.exhaustion;
    }
    match track_state() {
        Some(track) => track.exhaustion,
        None => client_value.rem_euclid(MAX_EXHAUSTION),
    }
}

/// The saturation as the bar shows it: the server's, else the estimate, which follows it down as
/// the exhaustion passes 4 between the packets the game sends.
fn saturation_shown(client_value: f32) -> f32 {
    if let Some(sync) = sync_state() {
        return sync.saturation;
    }
    track_state().map_or(client_value, |track| track.saturation)
}

/// Whether the values shown are estimates: no mod on the server.
fn estimated() -> bool {
    sync_state().is_none()
}

/// `drawExhaustionOverlay`: the strip behind the food bar, from the right.
fn draw_exhaustion(draw: &mut dyn DrawContext, exhaustion: f32, right: f32, top: f32) {
    let ratio = (exhaustion / MAX_EXHAUSTION).clamp(0.0, 1.0);
    let width = (ratio * 81.0) as i32 as f32;
    if width <= 0.0 {
        return;
    }
    let _ = draw.sprite(
        SHEET,
        [81.0 - width, 18.0, width, ICON],
        [right - width, top, width, ICON],
        Some(argb(0.75)),
    );
}

/// `drawSaturationOverlay`: AppleSkin's golden outlines, over the icons the saturation covers.
fn draw_saturation(
    draw: &mut dyn DrawContext,
    offsets: &Offsets,
    level: f32,
    gained: f32,
    alpha: f32,
    right: f32,
    top: f32,
) {
    if level + gained < 0.0 {
        return;
    }
    let modified = (level + gained).clamp(0.0, 20.0);
    let end = (modified / 2.0).ceil() as i32;
    let start = if gained != 0.0 {
        (level / 2.0).max(0.0) as i32
    } else {
        0
    };
    for i in start..end {
        let (ox, oy) = offsets.food.get(i as usize).copied().unwrap_or((0, 0));
        let effective = modified / 2.0 - i as f32;
        let u = if effective >= 1.0 {
            27.0
        } else if effective > 0.5 {
            18.0
        } else if effective > 0.25 {
            9.0
        } else {
            0.0
        };
        let _ = draw.sprite(
            SHEET,
            [u, 0.0, ICON, ICON],
            [right + ox as f32, top + oy as f32, ICON, ICON],
            Some(argb(alpha)),
        );
    }
}

/// `drawHungerOverlay`: the icons the food would fill, their empty slot faint behind.
fn draw_hunger(
    draw: &mut dyn DrawContext,
    offsets: &Offsets,
    restored: i32,
    level: i32,
    alpha: f32,
    rotten: bool,
    right: f32,
    top: f32,
) {
    if restored <= 0 {
        return;
    }
    let modified = (level + restored).clamp(0, MAX_FOOD);
    let start = (level / 2).max(0);
    let end = (modified as f32 / 2.0).ceil() as i32;
    let (empty, half, full) = if rotten {
        (FOOD_EMPTY_HUNGER, FOOD_HALF_HUNGER, FOOD_FULL_HUNGER)
    } else {
        (FOOD_EMPTY, FOOD_HALF, FOOD_FULL)
    };
    for i in start..end {
        let (ox, oy) = offsets.food.get(i as usize).copied().unwrap_or((0, 0));
        let (x, y) = (right + ox as f32, top + oy as f32);
        vanilla_icon(draw, empty, x, y, alpha * 0.25);
        let is_half = i * 2 + 1 == modified;
        vanilla_icon(draw, if is_half { half } else { full }, x, y, alpha);
    }
}

/// `drawHealthOverlay`: the hearts the food would heal, their container faint behind.
fn draw_health(
    draw: &mut dyn DrawContext,
    offsets: &Offsets,
    health: f32,
    modified: f32,
    alpha: f32,
    hardcore: bool,
    left: f32,
    top: f32,
) {
    if modified <= health {
        return;
    }
    let fixed = modified.ceil() as i32;
    let start = (health.ceil() as f64 / 2.0).max(0.0) as i32;
    let end = (modified / 2.0).ceil().max(0.0) as i32;
    let (container, half, full) = if hardcore {
        (HEART_HARDCORE_CONTAINER, HEART_HARDCORE_HALF, HEART_HARDCORE_FULL)
    } else {
        (HEART_CONTAINER, HEART_HALF, HEART_FULL)
    };
    for i in start..end {
        let (ox, oy) = offsets.health.get(i as usize).copied().unwrap_or((0, 0));
        let (x, y) = (left + ox as f32, top + oy as f32);
        vanilla_icon(draw, container, x, y, alpha * 0.25);
        let is_half = i * 2 + 1 == fixed;
        vanilla_icon(draw, if is_half { half } else { full }, x, y, alpha);
    }
}

// ---------------------------------------------------------------------------------------------
// Tooltip
// ---------------------------------------------------------------------------------------------

/// An inline sprite of the GUI atlas, as one glyph of a tooltip line.
fn inline_sprite(sprite: &str) -> Text {
    Text::new(TextContent::Object(TextObject::Atlas {
        atlas: "minecraft:gui".to_string(),
        sprite: sprite.to_string(),
        fallback: None,
    }))
}

/// A gray note after a row's icons, as AppleSkin writes "x12".
fn note(text: &str) -> Text {
    let mut style = TextStyle::default();
    style.color = Some("#AAAAAA".to_string());
    let mut note = Text::literal(text);
    note.style = style;
    note
}

fn row(icons: Vec<Text>, note_text: Option<String>) -> Text {
    let mut line = icons
        .into_iter()
        .fold(Text::literal(""), |line, icon| line.append(icon));
    if let Some(text) = note_text {
        line = line.append(note(&format!(" {text}")));
    }
    line
}

/// The hunger row: a drumstick per 2 points, the half one first (AppleSkin draws from the right),
/// or one icon and "x12" past ten.
fn hunger_row(food: &Food) -> Option<Text> {
    let mut bars = (food.nutrition.abs() as f32 / 2.0).ceil() as i32;
    let mut text = None;
    if bars > 10 {
        text = Some(format!("x{}", if food.nutrition < 0 { -bars } else { bars }));
        bars = 1;
    }
    if bars <= 0 {
        return None;
    }
    let (full, half) = if food.rotten {
        (TIP_FOOD_FULL_HUNGER, TIP_FOOD_HALF_HUNGER)
    } else {
        (TIP_FOOD_FULL, TIP_FOOD_HALF)
    };
    // Icon `i` (counted from the right) holds points 2i and 2i+1.
    let icons: Vec<Text> = (0..bars)
        .rev()
        .map(|i| {
            let is_half = text.is_none() && food.nutrition.abs() - 1 == i * 2;
            inline_sprite(if is_half { half } else { full })
        })
        .collect();
    Some(row(icons, text))
}

/// The saturation row: AppleSkin's 7 px icons (a gold row for a gain, a red one for a loss), the
/// smaller ones for the last, or one icon and "x12" past ten.
fn saturation_row(food: &Food) -> Text {
    let amount = food.saturation.abs();
    let mut bars = (amount / 2.0).ceil() as i32;
    let mut text = None;
    if bars > 10 || bars == 0 {
        text = Some(format!("x{}", if food.saturation < 0.0 { -bars } else { bars }));
        bars = 1;
    }
    let tag = if food.saturation >= 0.0 { "gain" } else { "loss" };
    let icons: Vec<Text> = (0..bars)
        .rev()
        .map(|i| {
            let effective = (amount - (i * 2) as f32) / 2.0;
            let cell = if effective >= 1.0 {
                3
            } else if effective > 0.5 {
                2
            } else if effective > 0.25 {
                1
            } else if effective > 0.0 {
                0
            } else {
                4
            };
            inline_sprite(&format!("appleskin:gui/saturation_{tag}_{cell}"))
        })
        .collect();
    row(icons, text)
}

// ---------------------------------------------------------------------------------------------
// Debug screen
// ---------------------------------------------------------------------------------------------

/// `DecimalFormat("#.##")`: up to two decimals, none when whole.
fn up_to_two_decimals(value: f32) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ---------------------------------------------------------------------------------------------
// The mod
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    /// At start: the settings, and the channel the server's food state travels on.
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for setting in settings() {
            // Only a malformed spec is refused.
            let _ = r.register_setting(setting);
        }

        // Optional, so a peer without the mod still joins.
        let mut payloads = r.payload_handlers();
        let mut view = payloads.registrar("1").optional();
        let token = view.play_to_client_typed::<FoodSync>(PayloadId(SYNC_ID.into()), |value, _| {
            if let Ok(mut state) = SYNC.lock() {
                *state = Some(SyncState {
                    exhaustion: value.exhaustion,
                    saturation: value.saturation,
                    natural_regeneration: value.flags.0 & 1 != 0,
                    peaceful: value.flags.0 & 2 != 0,
                });
            }
            Ok(())
        });
        let _ = SYNC_TOKEN.set(token);
    }

    /// Before each atlas build: AppleSkin's icons, as GUI sprites of the mod.
    fn register_runtime_sprites(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterRuntimeSpritesEvent<'_>,
    ) {
        let mut add = |path: &str, png: &[u8]| {
            if let Ok(id) = Identifier::parse(&format!("appleskin:gui/{path}")) {
                event.sprites.push(RuntimeSprite {
                    atlas: TextureAtlas::Gui,
                    id,
                    png: png.to_vec(),
                });
            }
        };
        add("icons", icons_data::ICONS_PNG);
        for (path, png) in icons_data::TOOLTIP_SATURATION {
            add(path, png);
        }
    }

    /// At client start: the F3 entry.
    fn register_debug_entries(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterDebugEntriesEvent<'_>,
    ) {
        event.entries.push(DebugEntry {
            id: DEBUG_ID.to_string(),
            group: DebugGroup::Position,
        });
    }

    /// The F3 line: `hunger: 20, sat: 5, exh: 2.29/4`.
    fn debug_entry_lines(&self, host: &dyn ClientHost, event: &mut DebugEntryLinesEvent<'_>) {
        if event.id != DEBUG_ID || !on(host, SHOW_FOOD_DEBUG_INFO) {
            return;
        }
        let Some(player) = host.local_player() else {
            return;
        };
        let Some(food) = player.food(host) else {
            return;
        };
        event.lines.push(format!(
            "hunger: {}, sat: {}{}, exh: {}{:.2}/{}",
            food.level,
            if estimated() { "~" } else { "" },
            up_to_two_decimals(saturation_shown(food.saturation)),
            if estimated() { "~" } else { "" },
            exhaustion_shown(food.exhaustion),
            up_to_two_decimals(MAX_EXHAUSTION),
        ));
    }

    /// The health rows' state, once a frame (`HealthHudEvent` carries what vanilla's HUD reads).
    /// Nothing is drawn here: answering None leaves the rows to the game.
    fn health_hud(
        &self,
        _: &mut dyn ClientHost,
        event: &mut HealthHudEvent<'_>,
    ) -> Option<Vec<GuiDraw>> {
        let inputs = event.inputs;
        if let Ok(mut hud) = HUD.lock() {
            *hud = Some(HudState {
                tick: inputs.tick,
                health: inputs.health,
                max_health: inputs.max_health,
                absorption: inputs.absorption,
                hardcore: inputs.hardcore,
                regeneration: inputs.regeneration,
                harmed: matches!(inputs.kind, HeartKind::Poisoned | HeartKind::Withered),
                vehicle_hearts: inputs.vehicle_hearts,
            });
        }
        None
    }

    /// Once a client tick (`onClientTick`): the held food, and the flash's wave.
    fn client_tick_post(&self, host: &mut dyn ClientHost, _: &mut ClientTickPostEvent) {
        let host: &dyn ClientHost = &*host;
        let food = host.local_player().and_then(|player| player.food(host));
        let Some(food) = food else {
            // Out of a world: the next one starts with nothing from the last.
            if IN_WORLD.swap(false, Ordering::Relaxed) {
                forget_track();
                if let Ok(mut s) = SYNC.lock() {
                    *s = None;
                }
                if let Ok(mut h) = HUD.lock() {
                    *h = None;
                }
                if let Ok(mut h) = HELD.lock() {
                    *h = None;
                }
                reset_flash();
            }
            return;
        };
        IN_WORLD.store(true, Ordering::Relaxed);
        let level = food.level;

        // Without the mod on the server, work the values out from what the game shows.
        if estimated() {
            track_unsynced(food.level, food.saturation, food.exhaustion);
        } else {
            forget_track();
        }

        let held = if on(host, SHOW_FOOD_VALUES_HUD_OVERLAY)
            || on(host, SHOW_SATURATION_HUD_OVERLAY)
            || on(host, SHOW_FOOD_HEALTH_HUD_OVERLAY)
        {
            query_held(host, level)
        } else {
            None
        };
        if let Ok(mut slot) = HELD.lock() {
            *slot = held;
        }
        match held {
            Some(_) => {
                let max = host.setting_f64(MAX_HUD_OVERLAY_FLASH_ALPHA).unwrap_or(0.65) as f32;
                tick_flash(max);
            }
            None => reset_flash(),
        }
    }

    /// Before the health rows are drawn (`onPreRenderFood`): the exhaustion, under the food icons.
    fn gui_layer_pre(&self, host: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        if event.layer != PLAYER_HEALTH_LAYER
            || !on(host, SHOW_FOOD_EXHAUSTION_HUD_UNDERLAY)
            || hud_state().vehicle_hearts > 0
            || !is_survival(host)
        {
            return Flow::Continue;
        }
        let Some(food) = host.local_player().and_then(|player| player.food(host)) else {
            return Flow::Continue;
        };
        let (right, top) = food_bar(&mut *event.draw);
        draw_exhaustion(&mut *event.draw, exhaustion_shown(food.exhaustion), right, top);
        Flow::Continue
    }

    /// After the health rows are drawn (`onRenderFood` and `onRenderHealth`): the saturation, and
    /// what the held food would give, as hunger, saturation and health.
    fn gui_layer_post(&self, host: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
        if event.layer != PLAYER_HEALTH_LAYER || !is_survival(host) {
            return;
        }
        let show_saturation = on(host, SHOW_SATURATION_HUD_OVERLAY);
        let show_values = on(host, SHOW_FOOD_VALUES_HUD_OVERLAY);
        let show_health = on(host, SHOW_FOOD_HEALTH_HUD_OVERLAY);
        if !(show_saturation || show_values || show_health) {
            return;
        }
        let Some(food) = host.local_player().and_then(|player| player.food(host)) else {
            return;
        };
        let hud = hud_state();
        let saturation = saturation_shown(food.saturation);
        let exhaustion = exhaustion_shown(food.exhaustion);
        let offsets = offsets(
            &hud,
            food.level,
            saturation,
            on(host, SHOW_VANILLA_ANIMATIONS_OVERLAY),
        );
        let held = held_food();
        let flash = flash_alpha();

        // The food bar is not drawn while riding, so neither are its overlays.
        if hud.vehicle_hearts == 0 {
            let (right, top) = food_bar(&mut *event.draw);
            if show_saturation {
                draw_saturation(&mut *event.draw, &offsets, saturation, 0.0, 1.0, right, top);
            }
            if let (true, Some(held)) = (show_values, held) {
                draw_hunger(
                    &mut *event.draw,
                    &offsets,
                    held.nutrition,
                    food.level,
                    flash,
                    held.rotten,
                    right,
                    top,
                );
                if show_saturation {
                    let new_food = food.level + held.nutrition;
                    let new_saturation = saturation + held.saturation;
                    let gained = if new_saturation > new_food as f32 {
                        new_food as f32 - saturation
                    } else {
                        held.saturation
                    };
                    draw_saturation(&mut *event.draw, &offsets, saturation, gained, flash, right, top);
                }
            }
        }

        // The health the food would heal (`shouldShowEstimatedHealth`).
        let sync = sync_state();
        let peaceful = sync.is_some_and(|s| s.peaceful);
        let natural_regeneration = sync.map_or(true, |s| s.natural_regeneration);
        let Some(held) = held else {
            return;
        };
        if show_health
            && !offsets.health.is_empty()
            && !peaceful
            && food.level < 18
            && !hud.harmed
            && !hud.regeneration
        {
            let mut increment = 0.0;
            if hud.health < hud.max_health {
                let level = (food.level + held.nutrition).min(MAX_FOOD);
                if level >= 18 && natural_regeneration {
                    let new_saturation = (saturation + held.saturation).min(level as f32);
                    increment = estimated_health(level, new_saturation, exhaustion);
                }
                increment += held.regeneration;
            }
            let modified = (hud.health + increment).min(hud.max_health);
            if hud.health < modified {
                let (left, top) = health_bar(&mut *event.draw);
                draw_health(
                    &mut *event.draw,
                    &offsets,
                    hud.health,
                    modified,
                    flash,
                    hud.hardcore,
                    left,
                    top,
                );
            }
        }
    }

    /// A food's tooltip: what it gives, as icons, always or with SHIFT held (`onItemTooltip`).
    fn item_tooltip(&self, host: &dyn ClientHost, event: &mut ItemTooltipEvent<'_>) {
        let always = on(host, SHOW_FOOD_VALUES_IN_TOOLTIP_ALWAYS);
        let with_shift = on(host, SHOW_FOOD_VALUES_IN_TOOLTIP) && host.modifiers().shift();
        if !(always || with_shift) {
            return;
        }
        let Some(food) = food_of(host, event.stack) else {
            return;
        };
        let Some(hunger) = hunger_row(&food) else {
            return;
        };
        event.lines.push((hunger, u32::MAX));
        event.lines.push((saturation_row(&food), u32::MAX));
    }

    /// A player joined: tell their client the food state again on the next tick.
    fn player_logged_in(&self, _: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>) {
        if let Ok(mut last) = LAST_SENT.lock() {
            if let Some(map) = last.as_mut() {
                map.remove(&event.player);
            }
        }
    }

    /// Each server tick (`SyncHandler.onPlayerUpdate`): send each player's client their exhaustion
    /// and saturation when they moved, and the rules the estimate needs when they changed.
    fn server_tick(&self, host: &mut dyn ServerHost, _: &mut ServerTickEvent<'_>) {
        let Some(token) = SYNC_TOKEN.get() else {
            return;
        };
        let natural_regeneration = match host.game_rule("minecraft:natural_health_regeneration") {
            Ok(GameRuleValue::Bool(on)) => on,
            _ => true,
        };
        for player in host.players().server_players() {
            let Some(food) = player.food(&*host) else {
                continue;
            };
            let peaceful = player
                .level(&*host)
                .and_then(|level| level.difficulty(&*host))
                == Some("peaceful");
            let flags = i32::from(natural_regeneration) | (i32::from(peaceful) << 1);

            let uuid = player.uuid();
            let changed = match LAST_SENT.lock() {
                Ok(mut last) => {
                    let map = last.get_or_insert_with(HashMap::new);
                    let moved = match map.get(&uuid) {
                        Some(&(exhaustion, saturation, sent_flags)) => {
                            (exhaustion - food.exhaustion).abs() >= EXHAUSTION_STEP
                                || saturation != food.saturation
                                || sent_flags != flags
                        }
                        None => true,
                    };
                    if moved {
                        map.insert(uuid, (food.exhaustion, food.saturation, flags));
                    }
                    moved
                }
                Err(_) => false,
            };
            if changed {
                // A peer without the mod refuses it; nothing to do then.
                let _ = token.send(
                    host,
                    player,
                    &FoodSync {
                        exhaustion: food.exhaustion,
                        saturation: food.saturation,
                        flags: VarInt(flags),
                    },
                );
            }
        }
    }
}
