//! `/clear`: `Inventory.clearOrCountMatchingItems` over a player's slots
//! (`ClearInventoryCommands.clearInventory`), and the menu broadcasts the
//! command ends with.
use super::{Game, Target};
use horizon_client::{
    command::args::ItemInput,
    items::ItemStack,
    menu::{Kind, Menu},
};
use horizon_protocol::packets::play::{self, Clientbound as C};

impl Game<'_> {
    /// `ClearInventoryCommands.clearInventory` for one player: the matching
    /// items go (or are only counted), then the player's menu hears what
    /// changed (`containerMenu.broadcastChanges`, `inventoryMenu.slotsChanged`).
    pub(in crate::game) fn clear_player(
        &mut self,
        t: Target,
        predicate: &ItemInput,
        max: i32,
        counting: bool,
    ) -> i32 {
        // The inventory menu's 2x2 grid is a guest's own (`Guest::craft_grid`):
        // the body's grid is empty while a guest's turn is swapped in.
        let conn = match t {
            Target::Player => self.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => None,
        };
        let gi = conn.and_then(|c| {
            let host = self.lan.as_ref()?;
            host.hub.guests.iter().position(|g| g.conn == c)
        });
        let mut grid = gi.map(|i| self.lan.as_ref().unwrap().hub.guests[i].craft_grid.clone());
        let cleared = self.for_player(t, |body| {
            // The menu's grid, in the body's while it clears.
            if let Some(grid) = grid.as_mut() {
                for k in 0..4 {
                    std::mem::swap(&mut body.craft_grid[k], &mut grid[k]);
                }
            }
            // `inventoryMenu.getCraftSlots()` is the 2x2 inventory grid; an
            // open crafting table's 3x3 is a different menu. The body's grid
            // is the guest's 2x2 here, else only when the inventory menu is
            // the open one.
            let table_grid = (grid.is_none() && body.craft_size != 2).then(|| {
                let mut saved: [Option<ItemStack>; 4] = Default::default();
                for k in 0..4 {
                    saved[k] = body.craft_grid[k].take();
                }
                saved
            });
            let was = body.carried.clone();
            let n = body.clear_matching(predicate, max);
            let now = body.carried.clone();
            if let Some(saved) = table_grid {
                body.craft_grid[..4].clone_from_slice(&saved);
            }
            if let Some(grid) = grid.as_mut() {
                for k in 0..4 {
                    std::mem::swap(&mut body.craft_grid[k], &mut grid[k]);
                }
            }
            (n, was, now)
        });
        let Some((n, was, now)) = cleared else {
            return 0;
        };
        let Some(gi) = gi else {
            return n;
        };
        if let Some(grid) = grid
            && let Some(host) = self.lan.as_mut()
        {
            host.hub.guests.update(gi, |g| g.craft_grid = grid);
        }
        if !counting {
            self.clear_broadcast(gi, was, now);
        }
        n
    }
    /// `AbstractContainerMenu.broadcastChanges` for a guest's inventory menu:
    /// the cursor it no longer holds, then the result slot
    /// (`CraftingMenu.slotChangedCraftingGrid`, which the next broadcast sends
    /// as slot 0) of the grid the clear left.
    fn clear_broadcast(&mut self, gi: usize, was: Option<ItemStack>, now: Option<ItemStack>) {
        let Some(grid) = self
            .lan
            .as_ref()
            .map(|host| host.hub.guests[gi].craft_grid.clone())
        else {
            return;
        };
        let mut menu = Menu::new(Kind::Inventory);
        let Some(result) = self.guest_menu_op(gi, &mut menu, |menu, env| {
            for (k, s) in grid.into_iter().enumerate() {
                menu.set_slot(env, 1 + k, s);
            }
            menu.slot(0).cloned()
        }) else {
            return;
        };
        let (uuid, name) = {
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            let g = &host.hub.guests[gi];
            (g.uuid, g.name.clone())
        };
        // The menu engine's result goes through `PrepareItemCraft` before the
        // guest's result slot keeps it, as a click's does.
        let mut result = result;
        self.native_prepare_item_craft(uuid, &name, None, &mut result);
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        host.hub.guests.update(gi, |g| g.craft_result = result);
        // `synchronizeCarriedToRemote`: only what the player's copy differs on.
        if now != was {
            let net = host.hub.stack(&now);
            host.hub.guests[gi].send(&C::SetCursorItem(play::SetCursorItem { item: net }));
        }
    }
}
