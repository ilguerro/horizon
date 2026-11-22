//! A cloud doses each mob at its own place in the level's join order
//! (`ServerLevel.tick`'s `entityTickList`, #12183). The expected values are
//! the official 26.3 server's, stepped one tick at a time (the issue's table):
//! a pig that joined before the cloud has counted its effects and hurt timer
//! down when the dose lands, one that joined after counts the new effect down
//! in the same tick.
use crate::game::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::chunk::Chunk;
use horizon_client::motion::PlayerMotion;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES");
    let home = std::env::var_os("HORIZON_HOME");
    if root.is_none() && home.is_none() {
        // Neither is set: the test is selected but nothing was provided for it.
        return;
    }
    // The daily run sets both; someone selecting this test by hand must too, and a
    // half-set pair fails instead of passing without a tick or an assertion.
    let root = root.expect("HORIZON_TEST_RESOURCES is not set: this test needs prepared resources");
    let home = home.expect("HORIZON_HOME is not set: this test needs a scratch game home");
    let root = std::path::PathBuf::from(root);
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.game_dir = Some(
        std::path::PathBuf::from(home).join(format!("h12183-{:?}", std::thread::current().id())),
    );
    options.player_name = Some("H12183".into());
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    // Entities tick in a chunk with the chunks two around it in too.
    for (x, z) in (-2..=2).flat_map(|x| (-2..=2).map(move |z| (x, z))) {
        game.world
            .chunks
            .insert((x, z), Arc::new(Chunk::empty(x, z)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(x, z))
            .unwrap();
    }
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
    game.creative = true;
    test(&mut game);
}

const CLOUD: &str = "{Radius:1.5f,WaitTime:0,Duration:200,potion_contents:{custom_effects:[\
    {id:\"minecraft:speed\",duration:100},{id:\"minecraft:instant_damage\"}]}}";

/// Pig `n`: its Speed ticks left, hurt timer and health.
fn pig(game: &Game<'_>, n: i32) -> Option<(i32, i32, f32)> {
    let m = game
        .mobs
        .iter()
        .find(|m| game.persist.uuid_of(m.id) == Some([0, 12183, 0, n]))?;
    let speed = m.extra.effects.iter().find(|e| e.id == "minecraft:speed");
    Some((speed.map_or(0, |e| e.duration), m.hurt_time, m.health))
}

#[test]
#[ignore = "daily"]
fn a_cloud_doses_each_mob_in_the_order_they_joined() {
    with_game(|game| {
        let summon_pig = |game: &mut Game<'_>, n: i32, (x, z): (f64, f64)| {
            let command = format!(
                "summon minecraft:pig {x} 100 {z} {{UUID:[I;0,12183,0,{n}],NoAI:1b,Silent:1b}}"
            );
            assert!(game.run_command(&command), "{command}");
        };
        let summon_cloud = |game: &mut Game<'_>, (x, z): (f64, f64)| {
            let command = format!("summon minecraft:area_effect_cloud {x} 100 {z} {CLOUD}");
            assert!(game.run_command(&command), "{command}");
        };
        // Pig 1, then its cloud.
        summon_pig(game, 1, (2.5, 8.5));
        summon_cloud(game, (2.5, 8.5));
        // A cloud, then pig 2.
        summon_cloud(game, (7.5, 8.5));
        summon_pig(game, 2, (7.5, 8.5));
        // Pig 3, a cloud, then pig 4: each pig on its own side of it.
        summon_pig(game, 3, (12.5, 8.5));
        summon_cloud(game, (12.5, 8.5));
        summon_pig(game, 4, (12.5, 8.5));
        // Pig 5, a cloud, then pig 6, with pig 6 first in the mob list, as a
        // mob put back out of join order would be: the walk is still by join.
        summon_pig(game, 5, (7.5, 3.5));
        summon_cloud(game, (7.5, 3.5));
        summon_pig(game, 6, (7.5, 3.5));
        let six = game
            .mobs
            .iter()
            .position(|m| game.persist.uuid_of(m.id) == Some([0, 12183, 0, 6]))
            .unwrap();
        let six = game.mobs.remove(six);
        game.mobs.insert(0, six);
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }
        let got: Vec<_> = (1..=6).map(|n| pig(game, n)).collect();
        // Before the cloud: Speed 100 and hurt timer 10; after it: 99 and 9.
        let want = [
            Some((100, 10, 7.0)),
            Some((99, 9, 7.0)),
            Some((100, 10, 7.0)),
            Some((99, 9, 7.0)),
            Some((100, 10, 7.0)),
            Some((99, 9, 7.0)),
        ];
        assert_eq!(got, want, "pigs 1-6 after the dose tick (the fifth)");
    });
}

/// A pass that pauses keeps the clouds' doses in join order afterwards: a fox that finishes
/// its food on the dose tick pauses the pass before the cloud's slot, and the pig after the
/// cloud still counts its dose down in its own turn. Without the rebuilt list's boundary the
/// pig ticks before its dose and reads 100/10 instead of 99/9.
#[test]
#[ignore = "daily"]
fn a_resumed_pass_keeps_the_cloud_doses_in_join_order() {
    with_game(|game| {
        let summon_pig = |game: &mut Game<'_>, n: i32| {
            let command = format!(
                "summon minecraft:pig 2.5 100 8.5 {{UUID:[I;0,12183,0,{n}],NoAI:1b,Silent:1b}}"
            );
            assert!(game.run_command(&command), "{command}");
        };
        // The fox joins first and finishes its food on the dose tick, pausing the pass.
        assert!(
            game.run_command(
                "summon minecraft:fox 12.5 100 2.5 {UUID:[I;0,12183,9,1],Silent:1b}"
            ),
            "the fox"
        );
        summon_pig(game, 1);
        let cloud = format!("summon minecraft:area_effect_cloud 2.5 100 8.5 {CLOUD}");
        assert!(game.run_command(&cloud), "{cloud}");
        summon_pig(game, 2);
        let fox = |game: &Game<'_>| {
            game.mobs
                .iter()
                .position(|m| game.persist.uuid_of(m.id) == Some([0, 12183, 9, 1]))
                .unwrap()
        };
        // The first tick settles the fox; it stands still, on the floor, and has not started
        // eating yet.
        game.tick_world(true).unwrap();
        let at = fox(game);
        game.mobs[at].velocity = Vec3::ZERO;
        game.mobs[at].on_ground = true;
        game.mobs[at].ai.goals = Default::default();
        game.mobs[at].ai.targets = Default::default();
        game.mobs[at]
            .extra
            .set_main_hand(Some("minecraft:sweet_berries".into()));
        for _ in 0..3 {
            game.tick_world(true).unwrap();
        }
        // The fifth tick: the fox eats its food and the cloud doses, in one pass.
        let at = fox(game);
        game.mobs[at].extra.ints[1] = 600;
        game.tick_world(true).unwrap();
        assert_eq!(pig(game, 1), Some((100, 10, 7.0)), "the pig before the cloud");
        assert_eq!(
            pig(game, 2),
            Some((99, 9, 7.0)),
            "the pig after the cloud, after the pause"
        );
    });
}

/// A cloud that joined after every mob doses after the loop, and a lethal dose still dies in
/// that tick, as one inside the loop does: its death is enqueued for the same creature pass.
#[test]
#[ignore = "daily"]
fn a_lethal_tail_dose_dies_in_the_same_tick() {
    with_game(|game| {
        let pig = "summon minecraft:pig 2.5 100 8.5 \
            {UUID:[I;0,12183,0,1],NoAI:1b,Silent:1b,Health:1.0f}";
        assert!(game.run_command(pig), "{pig}");
        let cloud = "summon minecraft:area_effect_cloud 2.5 100 8.5 {Radius:1.5f,WaitTime:0,\
            Duration:200,potion_contents:{custom_effects:[{id:\"minecraft:instant_damage\"}]}}";
        assert!(game.run_command(cloud), "{cloud}");
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }
        let at = game
            .mobs
            .iter()
            .position(|m| game.persist.uuid_of(m.id) == Some([0, 12183, 0, 1]))
            .expect("the pig is there");
        assert!(!game.mobs[at].alive(), "the tail dose killed the pig");
        assert!(
            game.died.contains(&game.mobs[at].id),
            "its death is enqueued in the dose's tick"
        );
    });
}
