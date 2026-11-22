//! PURPOSE: the doses a potion cloud gives the mobs at its own place in the
//! level's join order (`Game::tick_potion_clouds` ticks the clouds).
//! WHY: `ServerLevel.tick` ticks every entity in the order it joined
//! (`entityTickList.forEach`), so a mob that joined before a cloud has
//! counted its effects and hurt timer down when the cloud doses it, and one
//! that joined after counts the new effect down in the same tick. The clouds
//! tick before the mobs here; a cloud that a mob joined before gives its mobs
//! their dose in the mob loop instead, just before the first mob that joined
//! after it ticks (#12183).
//! REFS: vanilla `AreaEffectCloud.serverTick`, `ServerLevel.tick`;
//! `tick_creature_entities`; `ordered_tick.rs`.
//! LOGS: none. READS-BY: `Game::tick_potion_cloud`, `tick_creature_entities`.
//! ENDS-WHEN: every kind ticks from the join-order walk.

use super::super::dragon::Breath;
use super::{Victim, mob_can_be_affected, mob_effect_id};
use crate::game::Game;
use crate::game::combat::PlayerCredit;
use horizon_client::health::DamageLevel;
use horizon_client::join_seq::{JoinList, JoinSeq, ResidentList, RestoreOrder};
use horizon_client::lan::playerdata::ints_uuid;
use horizon_client::mobs::{Damage, Mob, Seen, Who, seen_grid::SeenGrid};
use horizon_core::geometry::Aabb;
use std::collections::{HashMap, HashSet};

/// One of a cloud's effects: (id, duration, amplifier).
type Effect = (&'static str, i32, i32);
/// The entity type tags (`inverted_healing_and_harm`, the effect immunities).
pub(in crate::game) type EntityTags = HashMap<String, HashSet<String>>;

/// The mob half of this tick's doses of the clouds a mob joined before: they
/// run in the mob loop, at each cloud's place among the mobs.
#[derive(Default)]
pub(in crate::game) struct CloudDoses {
    /// Each such cloud's join number and the box it doses, in join order.
    pub(super) due: Vec<(JoinSeq, Aabb)>,
    /// The clouds, out of `Game` while the mobs tick (`held`).
    clouds: ResidentList<Breath>,
    held: bool,
    /// The effects the doses in the loop added (`onEffectAdded`), told after it:
    /// no mod can change the mob list while it ticks.
    added: Vec<(&'static str, u32, i32, bool)>,
    /// The clouds a dose used up: `finish_cloud_doses` forgets their entity
    /// state, as the tick that expires a cloud does (`forget_entity`).
    removed: Vec<u32>,
}

/// `AreaEffectCloud.serverTick` for an entity in its box: one already a
/// victim or out of the radius is passed over; any other becomes a victim.
pub(super) fn admit(c: &mut Breath, victim: Victim, x: f64, z: f64) -> bool {
    let (dx, dz) = (x - c.at[0], z - c.at[2]);
    let Some(potion) = c.potion.as_mut() else {
        return false;
    };
    if potion.victims.iter().any(|(v, _)| *v == victim)
        || dx * dx + dz * dz > f64::from(c.radius * c.radius)
    {
        return false;
    }
    let until = c.age.wrapping_add(potion.settings.reapplication_delay);
    potion.victims.push((victim, until));
    true
}

/// After a dose: `radiusOnUse` and `durationOnUse`. False once the cloud is gone.
pub(super) fn used(c: &mut Breath) -> bool {
    let Some(potion) = c.potion.as_ref() else {
        return true;
    };
    if potion.settings.radius_on_use != 0.0 {
        c.radius += potion.settings.radius_on_use;
        if c.radius < 0.5 {
            return false;
        }
        c.radius = c.radius.clamp(0.0, 32.0);
    }
    if potion.settings.duration_on_use != 0 && c.duration != -1 {
        c.duration = c.duration.wrapping_add(potion.settings.duration_on_use);
        if c.duration <= 0 {
            return false;
        }
    }
    true
}

/// Whether one of the cloud's effects can affect this mob
/// (`allEffects.stream().anyMatch(entity::canBeAffected)`).
fn takes(tags: &EntityTags, effects: &[Effect], m: &Mob) -> bool {
    effects
        .iter()
        .any(|(id, ..)| mob_can_be_affected(tags, &m.kind.id, id))
}

fn inverted(tags: &EntityTags, m: &Mob) -> bool {
    tags.get("minecraft:inverted_healing_and_harm")
        .is_some_and(|t| t.contains(&m.kind.id))
}

/// One of a cloud's effects on a mob: an instant one at half strength
/// (`HealOrHarmMobEffect.applyInstantaneousEffect`, `indirectMagic(cloud,
/// owner)`), any other at the cloud's duration scale (`LivingEntity.addEffect`).
/// Returns whether the cloud hurt it, and the effect it added (and whether it
/// was new) for the notice told after the loop.
#[allow(clippy::too_many_arguments)]
fn affect_mob(
    m: &mut Mob,
    level: &mut DamageLevel<'_>,
    context: &horizon_world::native_load_context::LoopContext,
    tags: &EntityTags,
    &(id, duration, amp): &Effect,
    (hidden, ambient, duration_scale): (&[&'static str], &[&'static str], f32),
    (at, owner, inverted): ([f64; 3], Option<Who>, bool),
) -> (bool, Option<(&'static str, bool)>) {
    match id {
        "instant_damage" | "instant_health" => {
            let amount =
                |base: i32| (0.5 * f64::from(base.wrapping_shl(amp as u32)) + 0.5) as i32 as f32;
            if (id == "instant_damage") != inverted {
                let source = Damage::indirect_magic(owner, at);
                let hurt = m.damage_by(level, amount(6), source, "minecraft:area_effect_cloud");
                (hurt, None)
            } else {
                m.heal(context, amount(4));
                (false, None)
            }
        }
        // `LivingEntity.addEffect` refuses what the mob cannot be affected by.
        _ => match mob_effect_id(id) {
            Some(name) if mob_can_be_affected(tags, &m.kind.id, id) => {
                let new = !m.has_effect(name);
                let scaled = horizon_client::effects::scaled_duration(duration, duration_scale);
                m.add_effect_with(
                    name,
                    scaled,
                    amp,
                    ambient.contains(&id),
                    !hidden.contains(&id),
                );
                (false, Some((name, new)))
            }
            _ => (false, None),
        },
    }
}

impl CloudDoses {
    /// Whether a dose is due before a mob's turn this tick, not only after
    /// them all: a cloud due joined before one of `mobs`.
    #[cfg(feature = "upstream-parallel-mobs")]
    pub(in crate::game) fn between_turns(&self, mobs: &[Mob]) -> bool {
        self.due
            .first()
            .is_some_and(|&(join, _)| mobs.iter().any(|m| join < m.join))
    }
}

impl Game<'_> {
    /// The mobs a cloud doses in `area`: the living ones at least one of its
    /// effects can affect (`allEffects.stream().anyMatch(entity::canBeAffected)`),
    /// so the others neither become victims nor use up its radius.
    pub(super) fn cloud_mob_candidates(&mut self, effects: &[Effect], area: Aabb) -> Vec<(u32, f64, f64)> {
        let effects = effects.to_vec();
        let candidates: Vec<(u32, String, f64, f64)> = self
            .mobs
            .iter()
            .filter(|m| m.alive() && m.bounds().intersects(area))
            .map(|m| (m.id, m.kind.id.clone(), m.position.x, m.position.z))
            .collect();
        candidates
            .into_iter()
            .filter(|(id, kind, ..)| {
                effects
                    .iter()
                    .any(|(effect, ..)| self.can_be_affected(Some(*id as i32), kind, effect))
            })
            .map(|(id, _, x, z)| (id, x, z))
            .collect()
    }
    /// Before the mob loop, when it doses a cloud's mobs: the mobs in join
    /// order, as `ServerLevel.tick` walks them (`entityTickList`), since a
    /// cloud's doses go just before the first mob that joined after it. The
    /// list is in that order unless something put a mob back out of place.
    pub(in crate::game) fn order_mobs_for_cloud_doses(&mut self) {
        if self.potions.cloud_doses.due.is_empty() || self.mobs.is_sorted_by_key(|m| m.join) {
            return;
        }
        // An insertion sort in place: swaps keep every mob's identity and join number.
        for i in 1..self.mobs.len() {
            let mut j = i;
            while j > 0 && self.mobs[j].join < self.mobs[j - 1].join {
                self.mobs.swap(j - 1, j);
                j -= 1;
            }
        }
    }
    /// Before the mob loop: the doses due in it, with the clouds out of the
    /// level for the loop (nothing reaches them while the mobs tick).
    pub(in crate::game) fn take_cloud_doses(&mut self) -> CloudDoses {
        let mut doses = std::mem::take(&mut self.potions.cloud_doses);
        if !doses.due.is_empty() {
            doses.due.sort_unstable_by_key(|d| d.0);
            doses.clouds = self.breath_clouds.lift();
            doses.held = true;
        }
        doses
    }
    /// Before mob `at`'s turn in the mob loop: the doses of the clouds that
    /// joined before it. The loop holds `Game`'s mobs, so the dose works on the
    /// loop's own list: it reads the mobs as they are at that point, as
    /// `getEntitiesOfClass` does, and redoes the dosed mobs' snapshot slots so
    /// the mobs after them see the new state. `at` is the current mob's place in
    /// that list; `slots` maps a mob's id to its snapshot slot, which differ
    /// once a resumed pass has rebuilt the list. What reaches outside the mobs
    /// (`onEffectAdded`) is told after the loop; a mob that died after its own
    /// turn dies now (`dead`).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn run_cloud_doses(
        &self,
        doses: &mut CloudDoses,
        level: &mut DamageLevel<'_>,
        at: usize,
        mobs: &mut JoinList<Mob>,
        slots: &horizon_client::fast_hash::FxHashMap<u32, u32>,
        keep: &[bool],
        (snapshot, grid): (&mut [Seen], &mut SeenGrid),
        credit: &mut HashMap<u32, PlayerCredit>,
        dead: &mut Vec<Mob>,
        died: &mut horizon_client::fast_hash::FxHashSet<u32>,
    ) {
        let before = mobs.get(at).map_or(JoinSeq::MAX, |m| m.join);
        let slot_of = |id: u32| slots.get(&id).map(|&slot| slot as usize);
        let kept = |slot: Option<usize>| slot.is_none_or(|s| keep.get(s).copied().unwrap_or(true));
        let tags = &world_stations!(self).enchantments.entity_tags;
        let items = world_items!(self);
        let context = &self.loop_context;
        let uuid = self.level.player_uuid;
        let CloudDoses {
            due,
            clouds,
            added,
            removed,
            ..
        } = doses;
        while let Some(&(join, area)) = due.first()
            && join < before
        {
            due.remove(0);
            let Some(k) = clouds.locate(join) else {
                continue;
            };
            let mut gone = None;
            clouds.update(k, |c| {
                let Some(effects) = c.potion.as_ref().map(|p| p.effects.clone()) else {
                    return;
                };
                let inside: Vec<usize> = (0..mobs.len())
                    .filter(|&j| {
                        let m = &mobs[j];
                        kept(slot_of(m.id))
                            && m.alive()
                            && m.bounds().intersects(area)
                            && takes(tags, &effects, m)
                    })
                    .collect();
                for j in inside {
                    let (id, x, z) = {
                        let m = &mobs[j];
                        (m.id, m.position.x, m.position.z)
                    };
                    if !admit(c, Victim::Mob(id), x, z) {
                        continue;
                    }
                    let Some(p) = c.potion.as_ref() else {
                        break;
                    };
                    let looks = (&p.hidden[..], &p.ambient[..], p.settings.duration_scale);
                    let (owner, guest, cloud_at) = (p.owner, p.guest, c.at);
                    let flip = inverted(tags, &mobs[j]);
                    for effect in &effects {
                        let m = &mut mobs[j];
                        let (hurt, told) = affect_mob(
                            m,
                            level,
                            context,
                            tags,
                            effect,
                            looks,
                            (cloud_at, owner, flip),
                        );
                        if hurt
                            && owner == Some(Who::Player)
                            && let Some(uuid) = self.player_uuid_of(guest)
                        {
                            credit.insert(id, PlayerCredit(ints_uuid(uuid)));
                        }
                        if let Some((name, new)) = told {
                            added.push((name, id, effect.2, new));
                        }
                    }
                    let m = &mut mobs[j];
                    // A mob whose turn is over has already left its death to the
                    // next tick: it dies now (`LivingEntity.die` runs at the blow).
                    if !m.alive() && !m.removed && j < at && died.insert(m.id) {
                        dead.push(m.clone());
                    }
                    let slot = slot_of(m.id);
                    let alive = kept(slot);
                    if let Some(slot) = slot
                        && let Some(s) = snapshot.get_mut(slot)
                    {
                        *s = m.seen(uuid, items);
                        s.alive &= alive;
                        grid.update(slot, s.bounds);
                    }
                    if !used(c) {
                        gone = Some(c.id);
                        return;
                    }
                }
            });
            if let Some(id) = gone {
                clouds.remove(k);
                removed.push(id);
            }
        }
    }
    /// After the mob loop, with the mobs and the deaths set back in `Game`:
    /// the doses of the clouds that joined after every mob (all have ticked;
    /// the ones gone have left the list), the clouds back in the level, then
    /// what the doses did outside the mobs. `complete` is false when the pass
    /// stopped on a panic: its tail doses are not given, but the clouds and the
    /// effects of the doses that ran are kept.
    pub(in crate::game) fn finish_cloud_doses(
        &mut self,
        mut doses: CloudDoses,
        dead: &mut Vec<Mob>,
        complete: bool,
    ) {
        // A dose's callback can unwind (`damage_by`, a mod's notice): the clouds
        // are restored below either way, before the panic goes on.
        let tail = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if !complete {
                doses.due.clear();
                return;
            }
            for (join, area) in std::mem::take(&mut doses.due) {
                let Some(k) = doses.clouds.locate(join) else {
                    continue;
                };
                let mut gone = None;
                doses.clouds.update(k, |c| {
                    let Some(effects) = c.potion.as_ref().map(|p| p.effects.clone()) else {
                        return;
                    };
                    for (id, x, z) in self.cloud_mob_candidates(&effects, area) {
                        if !admit(c, Victim::Mob(id), x, z) {
                            continue;
                        }
                        let Some(p) = c.potion.as_ref() else {
                            break;
                        };
                        let looks = (&p.hidden[..], &p.ambient[..], p.settings.duration_scale);
                        self.cloud_affect(
                            Victim::Mob(id),
                            &p.effects,
                            looks,
                            c.at,
                            &p.source,
                            (p.owner, p.guest),
                        );
                        // Every mob has ticked: a lethal dose dies now, as one in
                        // the loop does (`LivingEntity.die` runs at the blow).
                        let killed = self
                            .mobs
                            .iter()
                            .find(|m| m.id == id && !m.alive() && !m.removed)
                            .cloned();
                        if let Some(m) = killed
                            && self.died.insert(id)
                        {
                            dead.push(m);
                        }
                        if !used(c) {
                            gone = Some(c.id);
                            return;
                        }
                    }
                });
                if let Some(id) = gone {
                    doses.clouds.remove(k);
                    doses.removed.push(id);
                }
            }
        }));
        // The clouds go back into the level whether the tail finished or a
        // callback unwound: the local list would drop them.
        if std::mem::take(&mut doses.held) {
            let clouds = std::mem::take(&mut doses.clouds);
            self.breath_clouds.restore(clouds, RestoreOrder::SurvivorsFirst);
        }
        // A cloud a dose used up takes its `noPhysics` write with it.
        for id in std::mem::take(&mut doses.removed) {
            self.forget_entity(id);
        }
        for (name, mob, amp, new) in std::mem::take(&mut doses.added) {
            self.mod_effect_added(name, Some(mob as i32), amp, new);
        }
        self.potions.cloud_doses = doses;
        if let Err(payload) = tail {
            std::panic::resume_unwind(payload);
        }
    }
}
