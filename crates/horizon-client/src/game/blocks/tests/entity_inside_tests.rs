//! `ServerLevel.tick` ticks only the entities in entity-ticking chunks, and
//! `Entity.applyEffectsFromBlocks` runs in the entity's own tick: a boat past
//! the simulation distance breaks no lily pad, however long it stands on it.
//! (Main's `game::entity_inside_tests` has the mob and the plate.)

use crate::game::boats::Boat;
use crate::game::native_machines::tests::server_game;
use horizon_client::chunk::Chunk;
use horizon_core::geometry::Vec3;
use std::sync::Arc;

#[test]
#[ignore = "daily"]
fn a_boat_breaks_a_lily_pad_only_inside_the_simulation_distance() {
    let mut game = server_game();
    game.settings.simulation_distance = 4;
    game.settings.render_distance = 12;
    let registry = game.pack.assets.registry;
    let water = registry.default_state("minecraft:water").unwrap();
    let pad = registry.default_state("minecraft:lily_pad").unwrap();
    let air = registry.default_state("minecraft:air").unwrap();
    // The player in chunk (0, 0); a boat floating on a pad in chunk (2, 0),
    // inside the simulation distance, and another in chunk (8, 0): loaded and
    // in view, but past it.
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5)).unwrap(),
    );
    for (cx, x, id) in [(2, 40, 1), (8, 136, 2)] {
        game.world
            .chunks
            .insert((cx, 0), Arc::new(Chunk::empty(cx, 0)));
        game.load_ticking_neighbourhood(cx, 0);
        game.world.set(x, 63, 8, water).unwrap();
        game.world.set(x, 64, 8, pad).unwrap();
        // The hull's top 0.5125 of its 0.5625 is inside the pad's cell, as a
        // floating boat's is.
        game.boats.push(Boat::new(
            id,
            "oak".into(),
            Vec3::new(f64::from(x) + 0.5, 63.95, 8.5),
            0.0,
        ));
    }
    game.update_chunk_tickets();
    let ticking = game.entity_ticking_area();
    assert!(ticking(40, 8), "chunk (2, 0) ticks entities");
    assert!(
        !ticking(136, 8),
        "chunk (8, 0) is past the simulation distance"
    );
    for _ in 0..3 {
        game.tick_blocks().unwrap();
    }
    assert_eq!(
        game.world.block(40, 64, 8),
        air,
        "the ticking boat did not break the pad"
    );
    assert_eq!(
        game.world.block(136, 64, 8),
        pad,
        "the boat past the simulation distance broke the pad"
    );
}
