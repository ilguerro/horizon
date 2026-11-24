//! `ThrownEnderpearl.onHit` for a pearl whose owner is in another level: the
//! owner goes to the pearl's level (`ServerPlayer.teleport` with the pearl's
//! `TeleportTransition(level, teleportPos, ..)`), not to the pearl's
//! coordinates in their own level. A stasis chamber's pearl, left in the
//! Overworld, takes its owner back from the Nether.

use std::cell::Cell;

use super::{Dim, Game, LevelTeleport};
use horizon_core::geometry::Vec3;

thread_local! {
    /// The level of the guest whose pearl is landing now, when known.
    static OWNER_LEVEL: Cell<Option<Dim>> = const { Cell::new(None) };
}

/// Runs `f`, a guest's pearl landing, with the guest's own level `level` known
/// to `pearl_takes_owner` (the hub, which holds it, is out of reach during the
/// guest's turn).
pub(in super::super) fn with_owner_level<R>(level: Option<Dim>, f: impl FnOnce() -> R) -> R {
    let outer = OWNER_LEVEL.with(|c| c.replace(level));
    let result = f();
    OWNER_LEVEL.with(|c| c.set(outer));
    result
}

impl Game<'_> {
    /// `ThrownEnderpearl.onHit` once the pearl may take its owner (the player
    /// in place): `player.teleport` to `to`, then `hurtServer(enderPearl, 5)`
    /// (`damage`, as the mods left it) and the teleport sound at `to`.
    pub(in super::super) fn pearl_takes_owner(&mut self, to: Vec3, damage: f32) {
        let owner = OWNER_LEVEL.with(Cell::get);
        let away = owner.filter(|level| *level != self.dim);
        if let (Some(conn), Some(owner)) = (self.lan_acting, away) {
            // Into the pearl's level: a new body there, out of its vehicle and
            // with no fall distance or motion (`run_level_teleports`); the
            // rotation is the player's own (`Relative.ROTATION`). The owner
            // leaves their own level, and the 5 damage lands on the player the
            // teleport returns, once they are in the pearl's level.
            self.level_teleports.push(LevelTeleport {
                conn,
                dim: self.dim,
                from: Some(owner),
                at: [to.x, to.y, to.z],
                rotation: None,
                vehicle: None,
                hurt: (damage > 0.0).then_some(("minecraft:ender_pearl", damage)),
            });
        } else {
            // ServerPlayer.teleport: removeVehicle before the placement.
            let origin = self
                .player
                .as_ref()
                .map(|pl| [pl.position.x, pl.position.y, pl.position.z]);
            crate::game::creatures::section_steps::teleport_player(self, to);
            if let Some(origin) = origin {
                self.acting_teleport_cause(origin, "ender_pearl");
            }
            if damage > 0.0 {
                self.hurt_player_by("minecraft:ender_pearl", damage);
            }
        }
        self.audio.play(
            "minecraft:entity.player.teleport",
            Some([to.x, to.y, to.z]),
            1.0,
            1.0,
        );
    }
}
