//! `/tp`'s wake-up: `ServerPlayer.teleportTo` wakes a sleeper before the move,
//! and a relative destination then moves from where the wake left them.
use crate::game::native_entity_refs::tests::{bind_local_player, with_game};
use crate::game::Game;
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::geometry::Vec3;
use std::sync::Arc;

/// Puts the player in a bed (foot at 4,100,4 facing south, head at 4,100,5) on
/// a stone floor, asleep, and gives the command the operator rights it needs.
fn asleep(game: &mut Game<'_>) {
    game.build_registries();
    game.set_allow_commands(true);
    game.ui.registries.permission = 4;
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.player = Some(PlayerMotion::new(game.motion, BED).unwrap());
    bind_local_player(game, 1, 99);
    assert!(game.run_command("fill 0 99 0 8 99 8 minecraft:stone"));
    assert!(game.run_command("setblock 4 100 4 minecraft:red_bed[part=foot,facing=south]"));
    assert!(game.run_command("setblock 4 100 5 minecraft:red_bed[part=head,facing=south]"));
}

/// Where the sleeper's body lies before the teleport.
const BED: Vec3 = Vec3::new(4.5, 100.5625, 4.5);

/// Where the wake stands them, worked out from the decompiled 26.3 source and
/// not from a teleport: `LivingEntity.stopSleeping` asks
/// `AbstractBedBlock.findStandUpPosition` for the bed at (4,100,4) facing
/// south, with the body's yaw 0. `Direction.isFacingAngle(0)` is false for the
/// bed's clock-wise side (WEST), and its first offset, (3,100,4), is a safe
/// spot on the stone floor, so `DismountHelper.findSafeDismountLocation`
/// answers `Vec3.upFromBottomCenterOf` of that block.
const STOOD: Vec3 = Vec3::new(3.5, 100.0, 4.5);

/// Sleeps in the bed `asleep` made.
fn sleep(game: &mut Game<'_>) {
    // The wake's stand-up position is picked with the body's yaw, which the
    // wake then turns to look at the bed: both wakes start from the same one.
    game.yaw = 0.0;
    if let Some(p) = game.player.as_mut() {
        p.position = BED;
        p.velocity = Vec3::ZERO;
        p.yaw = 0.0;
    }
    game.sleeping = Some([4, 100, 4]);
    game.sleep_timer = 100;
}

#[test]
#[ignore = "daily"]
fn a_teleport_wakes_a_sleeping_player_before_it_moves_them() {
    with_game(|game| {
        asleep(game);
        // `AbstractBedBlock.findStandUpPosition` beside the bed, with no move of
        // its own: the wake's own teleport, then the command's.
        sleep(game);
        assert!(game.run_command("tp @s ~ ~ ~"));
        assert_eq!(
            game.player.as_ref().unwrap().position,
            STOOD,
            "the wake stands them beside the bed, where the command's own ~ ~ ~ leaves them"
        );
        assert!(game.sleeping.is_none(), "the sleeper is out of bed");
        // `stopSleepInBed(true, true)` is a forceful wake-up: their own sleep
        // counter stays 0 (their client fades from the animate packet), as on
        // every other forceful wake.
        assert_eq!(game.sleep_timer, 0, "a forceful wake-up leaves no fade");
        // `ServerPlayer.teleportTo` calls `stopSleepInBed(true, true)` first, and
        // `Entity.teleportSetPosition` reads `PositionMoveRotation.of(this)` after
        // it: the relative axes move from where the wake left them, not from the
        // bed. z is the bed's + 2.
        sleep(game);
        assert!(game.run_command("tp @s ~ ~ ~2"));
        let at = game.player.as_ref().unwrap().position;
        assert_eq!(at, Vec3::new(STOOD.x, STOOD.y, STOOD.z + 2.0));
        assert!(game.sleeping.is_none(), "the sleeper is out of bed");
        assert_eq!(game.sleep_timer, 0, "a forceful wake-up leaves no fade");
        // An absolute destination is the command's own, wherever the wake left
        // them.
        sleep(game);
        assert!(game.run_command("tp @s 0.5 100.0 0.5"));
        let at = game.player.as_ref().unwrap().position;
        assert_eq!(at, Vec3::new(0.5, 100.0, 0.5));
        assert!(game.sleeping.is_none(), "the sleeper is out of bed");
    });
}

/// `native_player_teleported` hears the position the player was put at, not
/// the command's own: a wake moves a relative destination off it.
#[test]
#[ignore = "daily"]
fn a_waking_teleport_tells_the_mods_where_it_put_the_player() {
    with_game(|game| {
        asleep(game);
        // A command's teleport of a server's player is told to the mods once
        // their move is in the hub: `TeleportNote` is that queue, and
        // `flush_teleport_notes` hands its `to` to `native_player_teleported`.
        game.lan_acting = Some(1);
        game.lan_acting_uuid = Some(99);
        game.lan_acting_name = Some("Alice".into());
        sleep(game);
        // What `tp @s ~ ~ ~2` hands `teleportTo`: the command resolves the
        // relative destination against the bed before the wake moves them.
        game.teleport_to_in_level(
            super::super::Target::Player,
            [BED.x, BED.y, BED.z + 2.0],
            None,
            [true, true, true],
            false,
        );
        let note = game.teleport_notes.last().expect("a note for the mods");
        assert_eq!(
            note.to,
            [STOOD.x, STOOD.y, STOOD.z + 2.0],
            "the mods hear where the teleport put them, not the command's own ~ ~ ~2"
        );
        assert_eq!(
            game.player.as_ref().unwrap().position,
            Vec3::new(note.to[0], note.to[1], note.to[2]),
            "and the note is where they were put"
        );
    });
}
