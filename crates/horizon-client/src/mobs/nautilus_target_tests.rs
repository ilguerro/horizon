//! Whom a nautilus angers at and whom its charge hits:
//! `NautilusAi.setAngerTarget` and `findNearestValidAttackTarget` with
//! `Sensor.isEntityAttackableIgnoringLineOfSight`, and `ChargeAttack.tick`'s
//! scan of the body's own box with `NautilusAi.ATTACK_TARGET_CONDITIONS`.
use super::hostile_target_tests::pool;
use super::*;
use crate::mobs::world::with_test_world;
use std::sync::Arc;

/// `ChargeAttack.tick` scans the body's own box with `chargeTargeting`,
/// not the remembered `ATTACK_TARGET`: a cow sharing the box takes the
/// hit while the remembered one stands outside it.
#[test]
fn a_charge_hits_the_entity_in_its_box_not_the_remembered_target() {
    let mut seen = None;
    let ran = with_test_world(&[], |_, types| {
        let kind = Arc::new(types.types["minecraft:cow"].clone());
        seen = Some((
            Mob::new(2, kind.clone(), Vec3::new(5.5, 64.0, 0.5), 0.0)
                .seen(None, &crate::items::Items::empty()),
            Mob::new(3, kind, Vec3::new(0.5, 64.0, 0.2), 0.0)
                .seen(None, &crate::items::Items::empty()),
        ));
    });
    let Some((target, bystander)) = seen.filter(|_| ran) else {
        return;
    };
    let mut events = Vec::new();
    let ran = with_test_world(&[target, bystander], |w, types| {
        let mut m = Mob::new(
            1,
            Arc::new(types.types["minecraft:nautilus"].clone()),
            Vec3::new(0.5, 64.0, 0.2),
            0.0,
        );
        m.ai.target = Some(Who::Mob(2));
        let mut goal = Charge::new(0.5);
        let mut test_level = crate::health::TestLevel::new(1);
        let level = &mut test_level.level();
        let mut cx = Cx {
            w,
            level,
            events: &mut events,
        };
        goal.tick(&mut m, &mut cx);
    });
    if !ran {
        return;
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, MobEvent::AttackMob { target: 3, .. })),
        "the cow in the box takes the hit: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, MobEvent::AttackMob { target: 2, .. })),
        "the remembered cow outside the box is untouched"
    );
}

/// Cows at `at` (ids 2, 3, ...) beside a nautilus in the pool; `hits`
/// are the cows that hurt it, in order, each followed by its brain's
/// `setAngerTarget`. Answers `ANGRY_AT` and what `find_target` picks
/// with the follow range `follow`, then with `follow_after`.
fn anger(
    at: &[Vec3],
    hits: &[u32],
    follow: f64,
    follow_after: f64,
    wall: bool,
) -> Option<(Option<Who>, Option<Who>, Option<Who>)> {
    let mut cows = None;
    crate::mobs::world::with_built_test_world(
        &[],
        |_, _| {},
        |_, types| {
            let kind = Arc::new(types.types["minecraft:cow"].clone());
            cows = Some(
                at.iter()
                    .zip(2..)
                    .map(|(&p, id)| {
                        Mob::new(id, kind.clone(), p, 0.0).seen(None, &crate::items::Items::empty())
                    })
                    .collect::<Vec<_>>(),
            );
        },
    );
    let cows = cows?;
    let mut out = None;
    crate::mobs::world::with_built_test_world(&cows, pool(wall), |w, types| {
        let kind = Arc::new(types.types["minecraft:nautilus"].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 62.0, 0.5), 270.0);
        m.fluids.in_water = true;
        m.attrs.follow_range = follow;
        // Within the target cooldown, so only `ANGRY_AT` can answer.
        m.extra.counter = 100;
        let mut test_level = crate::health::TestLevel::new(1);
        let level = &mut test_level.level();
        for &id in hits {
            m.invulnerable = 0;
            assert!(m.damage(level, 1.0, crate::mobs::Damage::mob(id, [0.0; 3])));
            set_anger_target(&mut m, w);
        }
        let mut random = crate::mobs::world::level_random(1);
        let first = find_target(&mut m, w, &mut random);
        m.attrs.follow_range = follow_after;
        let then = find_target(&mut m, w, &mut random);
        out = Some((m.extra.angry_at, first, then));
    });
    out
}

/// `NautilusAi.setAngerTarget` and `findNearestValidAttackTarget`: a hit
/// stores `ANGRY_AT` only for an attacker within the follow range, a wall
/// between them notwithstanding (`isEntityAttackableIgnoringLineOfSight`);
/// a later hit from out of range leaves the stored anger; and an angry
/// target out of range is filtered, not forgotten.
#[test]
fn a_nautilus_angers_only_at_an_attacker_in_its_follow_range() {
    let near = Vec3::new(2.5, 62.0, 0.5);
    let far = Vec3::new(12.5, 62.0, 0.5);
    let behind = Vec3::new(6.5, 62.0, 0.5);
    let (Some(beyond), Some(walled), Some(second), Some(left)) = (
        anger(&[far], &[2], 10.0, 10.0, false),
        anger(&[behind], &[2], 10.0, 10.0, true),
        anger(&[near, far], &[2, 3], 10.0, 10.0, false),
        anger(&[behind], &[2], 10.0, 4.0, false),
    ) else {
        return;
    };
    assert_eq!(beyond, (None, None, None), "past the follow range");
    let two = Some(Who::Mob(2));
    assert_eq!(walled, (two, two, two), "line of sight is ignored");
    assert_eq!(second, (two, two, two), "the far hit stores nothing");
    assert_eq!(
        left,
        (two, two, None),
        "out of range: filtered, still angry"
    );
}

/// The events `ChargeAttack.tick` pushes for a nautilus (id 1) at `at` whose
/// `ATTACK_TARGET` is mob 2, with `others` as its tick's snapshot and
/// `teams` its scoreboard teams.
fn charge(
    others: &[crate::mobs::Seen],
    at: Vec3,
    teams: Option<crate::mobs::Teams<'_>>,
) -> Option<Vec<MobEvent>> {
    let mut events = Vec::new();
    let ran = with_test_world(others, |w, types| {
        let mut m = Mob::new(
            1,
            Arc::new(types.types["minecraft:nautilus"].clone()),
            at,
            0.0,
        );
        m.ai.target = Some(Who::Mob(2));
        let mut world = w.clone();
        if let Some(teams) = teams {
            world.teams = std::sync::Arc::new(teams);
        }
        let mut goal = Charge::new(0.5);
        let mut test_level = crate::health::TestLevel::new(1);
        let level = &mut test_level.level();
        let mut cx = Cx {
            w: &world,
            level,
            events: &mut events,
        };
        goal.tick(&mut m, &mut cx);
    });
    ran.then_some(events)
}

/// `TargetingConditions.test`'s `targeter.isAlliedTo(target)`: a mob on the
/// nautilus' team is no combat target, so the charge passes it over for the
/// stranger beside it.
#[test]
fn a_charge_passes_over_a_mob_on_its_own_team() {
    let mut seen = None;
    let ran = with_test_world(&[], |_, types| {
        let kind = Arc::new(types.types["minecraft:cow"].clone());
        seen = Some([
            Mob::new(2, kind.clone(), Vec3::new(0.5, 64.0, 0.2), 0.0)
                .seen(None, &crate::items::Items::empty()),
            Mob::new(3, kind, Vec3::new(0.5, 64.0, 0.2), 0.0)
                .seen(None, &crate::items::Items::empty()),
        ]);
    });
    let Some(others) = seen.filter(|_| ran) else {
        return;
    };
    let mut board = crate::scoreboard::Scoreboard::default();
    board.add_team("t");
    let team = board.team("t").expect("the team it just made").clone();
    let teams = crate::mobs::Teams {
        player: None,
        mobs: [(1, &team), (2, &team)].into_iter().collect(),
    };
    let Some(events) = charge(&others, Vec3::new(0.5, 64.0, 0.2), Some(teams)) else {
        return;
    };
    assert!(
        events
            .iter()
            .any(|e| matches!(e, MobEvent::AttackMob { target: 3, .. })),
        "the stranger in the box takes the hit: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, MobEvent::AttackMob { target: 2, .. })),
        "its team mate is untouched"
    );
}

/// `TargetingConditions.test` refuses the targeter itself, and the tick's
/// snapshot holds the nautilus being ticked: its own row in the box is
/// passed over for the cow there.
#[test]
fn a_charge_never_targets_the_nautilus_itself() {
    let mut seen = None;
    let ran = with_test_world(&[], |_, types| {
        let nautilus = Arc::new(types.types["minecraft:nautilus"].clone());
        let cow = Arc::new(types.types["minecraft:cow"].clone());
        seen = Some([
            Mob::new(1, nautilus, Vec3::new(0.5, 64.0, 0.2), 0.0)
                .seen(None, &crate::items::Items::empty()),
            Mob::new(2, cow, Vec3::new(0.5, 64.0, 0.2), 0.0)
                .seen(None, &crate::items::Items::empty()),
        ]);
    });
    let Some(others) = seen.filter(|_| ran) else {
        return;
    };
    let Some(events) = charge(&others, Vec3::new(0.5, 64.0, 0.2), None) else {
        return;
    };
    assert!(
        events
            .iter()
            .any(|e| matches!(e, MobEvent::AttackMob { target: 2, .. })),
        "the cow in the box takes the hit: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, MobEvent::AttackMob { target: 1, .. })),
        "it does not ram itself"
    );
}

