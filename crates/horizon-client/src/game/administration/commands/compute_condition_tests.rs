//! `/compute`'s number providers read a `random_chance` chance through
//! `ContextFloatProvider.getFloat` and `binomial`'s probability through
//! `getFloatOrThrow` (`LootItemRandomChanceCondition.test`,
//! `BinomialDistributionGenerator.getIntUnsafe`): an inline provider chance is
//! evaluated (1.0 always holds, 0.0 never, a non-finite value reads as 0), and
//! a non-finite probability fails the command instead of counting every draw.
use crate::game::*;
use horizon_client::motion::PlayerMotion;
use std::sync::Arc;

fn game() -> Game<'static> {
    game_with(None)
}

/// A game over the prepared resources, with `providers` (a data-pack folder)
/// as its context providers when one is given.
fn game_with(providers: Option<&std::path::Path>) -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let mut loaded = load_resources(
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
    if let Some(dir) = providers {
        Arc::get_mut(&mut loaded.resources)
            .unwrap()
            .rest
            .get_mut()
            .unwrap()
            .providers = horizon_client::providers::Providers::load(dir);
    }
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
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
    game.world
        .chunks
        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap());
    game
}

/// Runs `command`: whether it succeeded and what the issuer was told.
fn run(game: &mut Game<'_>, command: &str) -> (bool, Vec<String>) {
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.chat_capture.take().unwrap_or_default();
    (
        ok,
        said.iter().map(horizon_client::text::Text::plain).collect(),
    )
}

/// The one line a command told its issuer.
fn only(said: &[String]) -> &str {
    assert_eq!(said.len(), 1, "{said:?}");
    &said[0]
}

/// `ConditionalValue` picks `on_true` or `on_false` from its condition, whose
/// `random_chance` reads the provider. A non-finite chance is 0, so it never
/// holds (`ContextFloatProvider.getFloat`).
#[test]
#[ignore = "daily"]
fn a_conditional_reads_its_random_chance_provider() {
    let mut game = game();
    let conditional = |chance: &str| {
        format!(
            r#"compute default integer {{type:"minecraft:conditional",condition:{{type:"minecraft:random_chance",chance:{chance}}},on_true:5,on_false:7}}"#
        )
    };
    for _ in 0..3 {
        let (ok, said) = run(
            &mut game,
            &conditional(r#"{type:"minecraft:constant",value:1.0}"#),
        );
        assert!(ok, "{said:?}");
        assert_eq!(only(&said), "Number provider returned value 5");
        let (ok, said) = run(
            &mut game,
            &conditional(r#"{type:"minecraft:constant",value:0.0}"#),
        );
        assert!(ok, "{said:?}");
        assert_eq!(only(&said), "Number provider returned value 7");
        let (ok, said) = run(
            &mut game,
            &conditional(r#"{type:"minecraft:div",left:1,right:0}"#),
        );
        assert!(ok, "{said:?}");
        assert_eq!(only(&said), "Number provider returned value 7");
    }
}

/// `BinomialDistributionGenerator.getIntUnsafe` asks for the probability with
/// `getFloatOrThrow`: a non-finite one is an `ArithmeticException`, which
/// `ComputeCommand.computeAsInt` reports, not a full count.
#[test]
#[ignore = "daily"]
fn a_non_finite_binomial_probability_fails_the_command() {
    let mut game = game();
    let binomial =
        |p: &str| format!(r#"compute default integer {{type:"minecraft:binomial",n:3,p:{p}}}"#);
    let (ok, said) = run(
        &mut game,
        &binomial(r#"{type:"minecraft:constant",value:1.0}"#),
    );
    assert!(ok, "{said:?}");
    assert_eq!(only(&said), "Number provider returned value 3");
    let (ok, said) = run(
        &mut game,
        &binomial(r#"{type:"minecraft:constant",value:0.0}"#),
    );
    assert!(ok, "{said:?}");
    assert_eq!(only(&said), "Number provider returned value 0");
    let (ok, said) = run(
        &mut game,
        &binomial(r#"{type:"minecraft:div",left:1,right:0}"#),
    );
    assert!(!ok, "vanilla throws ArithmeticException: {said:?}");
    assert_eq!(
        only(&said),
        "Number provider returned invalid value (Invalid value: Infinity)"
    );
}

/// A data-pack folder holding one self-referencing float provider: `loop` is a
/// conditional whose `random_chance` chance refers back to `loop`, which the
/// registry loader accepts (a `chance` reference is bound with `follow: false`).
fn self_referencing_providers() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-compute-self-reference-{}",
        std::process::id()
    ));
    let providers = dir.join("data/minecraft/context_float_provider");
    std::fs::create_dir_all(&providers).unwrap();
    std::fs::write(
        providers.join("loop.json"),
        r#"{"type":"conditional","condition":{"type":"minecraft:random_chance","chance":"minecraft:loop"},"on_true":1.0,"on_false":0.0}"#,
    )
    .unwrap();
    dir
}

/// A provider that reads its own chance must hit `provider_float`'s `depth >
/// 64` guard instead of recursing without bound: the guarded read is 0, so the
/// chance never holds and the conditional picks `on_false`.
#[test]
#[ignore = "daily"]
fn a_self_referencing_random_chance_provider_stops_at_the_depth_guard() {
    let providers = self_referencing_providers();
    let mut game = game_with(Some(&providers));
    std::fs::remove_dir_all(&providers).unwrap();
    let (ok, said) = run(
        &mut game,
        r#"compute default integer {type:"minecraft:conditional",condition:{type:"minecraft:random_chance",chance:"minecraft:loop"},on_true:5,on_false:7}"#,
    );
    assert!(ok, "{said:?}");
    assert_eq!(only(&said), "Number provider returned value 7");
}
